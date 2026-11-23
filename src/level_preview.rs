//! 2.2: the Level Up page's points before you confirm them.
//!
//! The game simulates a level-up on its own copy of your character data
//! (RTTI: CS::LevelPlayerStatusSimulator, used by CS::PlayerLevelUpDialog)
//! and only writes your real attributes when you confirm. Where that copy
//! lives is found at run time instead of hard-coding an offset: starting at
//! CSMenuMan, the objects it points to are walked breadth first (a few
//! thousand objects at most, spread over several ticks, every read checked
//! with VirtualQuery). Each object is searched for eight 32-bit values equal
//! to your confirmed Vigor..Arcane (the order PlayerGameData keeps them),
//! anywhere except your real character data. Inside the level-up classes a
//! copy that already went up is accepted too (you pressed before the search
//! finished). Every copy found is watched each tick, and the one that rises
//! above your confirmed attributes is what the page shows.
//!
//! Read only: nothing here writes game memory. Source-derived; not verified
//! in game (AGENTS.md: no testing by the agent). The log says what was found.
use eldenring::cs::{CSMenuManImp, GameDataMan};
use fromsoftware_shared::{FromStatic, Program};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Objects visited per search and per tick (the rest waits for the next tick).
const MAX_NODES: usize = 30_000;
const NODES_PER_TICK: usize = 3000;
const MAX_DEPTH: u8 = 6;
/// Bytes read from an ordinary object, and from the level-up classes.
const NODE_BYTES: usize = 0x400;
const SPECIAL_BYTES: usize = 0x2400;
const ROOT_BYTES: usize = 0x900;
const MAX_CANDIDATES: usize = 24;
/// A new search this long after the last one while nothing rose yet.
/// 3.2: keep looking every 2 s while the page is open and nothing rose
/// (the 3.1 log: the page's live copy was never among the first finds, and
/// the old limit of 8 walks ended the search after ~12 s).
const SEARCH_EVERY: Duration = Duration::from_millis(2000);
/// When the "not found" note is written (it never stops searching).
const MAX_SEARCHES: u32 = 8;
/// A copy unreadable or implausible for this many ticks is dropped.
const MAX_MISSES: u32 = 30;

fn special(class: &str) -> bool {
    class.contains("LevelUp") || class.contains("StatusSimulator") || class.contains("StatusCalculator")
}

struct Candidate {
    address: usize,
    path: String,
    misses: u32,
    /// Found under one of the level-up classes.
    special: bool,
    /// Trusted once it held exactly your confirmed attributes, or once it
    /// changed after it was found (a stray array of numbers does neither).
    anchored: bool,
    found: [u32; 8],
}

struct Node {
    address: usize,
    depth: u8,
    parent: Option<usize>,
    offset: u32,
    class: Option<String>,
    special: bool,
}

#[derive(Default)]
struct Search {
    nodes: Vec<Node>,
    /// What the level-up classes own is walked first.
    first: VecDeque<usize>,
    queue: VecDeque<usize>,
    seen: HashSet<usize>,
}

#[derive(Default)]
pub struct LevelPreview {
    candidates: Vec<Candidate>,
    search: Option<Search>,
    last_search: Option<Instant>,
    searches: u32,
    classes: HashMap<usize, Option<String>>,
    /// Interesting classes met while searching (for the log when nothing is found).
    met: Vec<String>,
    live_logged: bool,
    found_logged: bool,
    missing_logged: bool,
    /// 3.3: confirmed attributes seen last tick on this page, whether a copy
    /// rose since the page opened, and how many misses were logged.
    last_committed: Option<[u32; 8]>,
    live_this_open: bool,
    misses_logged: u32,
}

impl LevelPreview {
    /// The page closed: forget the copies (the game frees its simulation).
    pub fn reset(&mut self) {
        self.last_committed = None;
        self.live_this_open = false;
        self.candidates.clear();
        self.search = None;
        self.last_search = None;
        self.searches = 0;
        self.met.clear();
    }

    /// Once per tick while the Level Up page is open. `committed` is your
    /// confirmed Vigor..Arcane. Returns what the page shows now, when it shows
    /// more than that; log notes go to `notes`.
    ///
    /// # Safety
    /// Native game task; reads game memory (checked) and nothing else.
    pub unsafe fn tick(&mut self, committed: [u32; 8], level: u32, now: Instant, notes: &mut Vec<String>) -> Option<[u32; 8]> {
        let mut memory = Readable::default();
        let mut best: Option<([u32; 8], u32, usize)> = None;
        for (index, candidate) in self.candidates.iter_mut().enumerate() {
            match unsafe { read_values(candidate.address, &mut memory) }.filter(|v| raised(*v, committed).is_some()) {
                Some(values) => {
                    candidate.misses = 0;
                    if values == committed || values != candidate.found {
                        candidate.anchored = true;
                    }
                    let gain = raised(values, committed).unwrap_or(0);
                    if candidate.anchored && gain > 0 && best.is_none_or(|(_, g, _)| gain > g) {
                        best = Some((values, gain, index));
                    }
                }
                None => candidate.misses += 1,
            }
        }
        // 3.3: you confirmed points and no copy ever rose: say where we looked.
        if let Some(previous) = self.last_committed {
            if previous != committed && !self.live_this_open && best.is_none() && self.misses_logged < 3 {
                self.misses_logged += 1;
                let added: u32 = committed.iter().zip(previous).map(|(a, b)| a.saturating_sub(b)).sum();
                let paths: Vec<&str> = self.candidates.iter().take(3).map(|c| c.path.as_str()).collect();
                notes.push(format!(
                    "Level Up preview missed {added} confirmed points: {} searches, {} copies watched ({}); level-up classes met: {}",
                    self.searches,
                    self.candidates.len(),
                    paths.join(" | "),
                    if self.met.is_empty() { "none".to_owned() } else { self.met.join(", ") }
                ));
            }
        }
        self.last_committed = Some(committed);
        if best.is_some() {
            self.live_this_open = true;
        }
        if let Some((_, _, index)) = best {
            if !self.live_logged {
                self.live_logged = true;
                notes.push(format!(
                    "Level Up preview live: the panel follows the page ({})",
                    self.candidates[index].path
                ));
            }
        }
        self.candidates.retain(|c| c.misses < MAX_MISSES);
        // Keep looking while no copy has risen yet: the one the page uses may
        // be made only when you first press, or sit deeper than the last walk.
        let due = self.last_search.is_none_or(|at| now.duration_since(at) >= SEARCH_EVERY);
        if best.is_none() && self.search.is_none() && due {
            if let Some(search) = unsafe { start() } {
                self.search = Some(search);
                self.searches += 1;
                self.last_search = Some(now);
            }
        }
        if let Some(mut search) = self.search.take() {
            let real = unsafe { real_attributes() };
            let finished = unsafe { self.step(&mut search, committed, level, real, &mut memory) };
            if finished {
                if !self.candidates.is_empty() && !self.found_logged {
                    self.found_logged = true;
                    let paths: Vec<&str> = self.candidates.iter().take(3).map(|c| c.path.as_str()).collect();
                    notes.push(format!(
                        "Level Up preview: watching {} copies of your attributes ({} objects read; {})",
                        self.candidates.len(),
                        search.nodes.len(),
                        paths.join(" | ")
                    ));
                }
                if self.candidates.is_empty() && self.searches >= MAX_SEARCHES && !self.missing_logged {
                    self.missing_logged = true;
                    self.met.sort();
                    self.met.dedup();
                    notes.push(format!(
                        "Level Up preview: the page's points were not found ({} objects read; classes met: {}); the panel shows what one more point adds",
                        search.nodes.len(),
                        if self.met.is_empty() { "none of the level-up classes".to_owned() } else { self.met.join(", ") }
                    ));
                }
            } else {
                self.search = Some(search);
            }
        }
        best.map(|(values, _, _)| values)
    }

    /// Walk part of the object graph. Returns true when the walk is done.
    unsafe fn step(&mut self, search: &mut Search, committed: [u32; 8], level: u32, real: usize, memory: &mut Readable) -> bool {
        let program = Program::current();
        let (image_start, image_end) = image_range();
        let mut budget = NODES_PER_TICK;
        while budget > 0 {
            let Some(index) = search.first.pop_front().or_else(|| search.queue.pop_front()) else {
                return true;
            };
            budget -= 1;
            let (address, depth, parent_special) = {
                let node = &search.nodes[index];
                (node.address, node.depth, node.special)
            };
            // Only two levels below a level-up class count as "inside" it.
            let parent_special = parent_special && {
                let mut hops = 0;
                let mut at = search.nodes[index].parent;
                while let Some(i) = at {
                    if search.nodes[i].class.as_deref().is_some_and(special) {
                        break;
                    }
                    hops += 1;
                    at = search.nodes[i].parent;
                }
                // 3.2: the whole subtree (the 3.1 finds sat 4-5 levels deep).
                hops < 8
            };
            let first = if memory.check(address, 8) {
                unsafe { std::ptr::read_unaligned(address as *const usize) }
            } else {
                continue;
            };
            let class = if first >= image_start + 0x1000 && first < image_end {
                self.classes
                    .entry(first)
                    .or_insert_with(|| fromsoftware_shared::vftable_classname(&program, first))
                    .clone()
            } else {
                None
            };
            let is_special = class.as_deref().is_some_and(special);
            if is_special {
                let name = class.clone().unwrap_or_default();
                if !self.met.contains(&name) && self.met.len() < 12 {
                    self.met.push(name);
                }
            }
            search.nodes[index].class = class;
            // The level-up classes and what they own are read further.
            let inside = is_special || parent_special;
            let wanted = if depth == 0 {
                ROOT_BYTES
            } else if inside {
                SPECIAL_BYTES
            } else {
                NODE_BYTES
            };
            let length = memory.readable_len(address, wanted);
            if length < 16 {
                continue;
            }
            let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, length) };
            let word = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            let mut at = 0;
            while at + 32 <= length {
                let values: [u32; 8] = std::array::from_fn(|i| word(at + i * 4));
                let exact = values == committed;
                // 3.2: a copy that already went up counts inside the level-up
                // classes, or anywhere when it looks like your character
                // data (its level, 0x2C after Arcane's slot... after Vigor,
                // is your level or your level plus the new points).
                let gain = raised(values, committed).unwrap_or(0);
                let level_matches = at + 48 <= length && {
                    let at_level = word(at + 0x2C);
                    level > 0 && (at_level == level || at_level == level + gain)
                };
                let risen = gain > 0 && (inside || level_matches);
                let spot = address + at;
                if (exact || risen) && spot != real && !self.candidates.iter().any(|c| c.address == spot) {
                    let candidate = Candidate {
                        address: spot,
                        path: path_of(search, index, at as u32),
                        misses: 0,
                        special: inside,
                        // Character-data shape with your level: trusted at once.
                        anchored: exact || (risen && level_matches),
                        found: values,
                    };
                    if self.candidates.len() < MAX_CANDIDATES {
                        self.candidates.push(candidate);
                    } else if inside {
                        // A copy under the level-up classes beats a stray one.
                        if let Some(slot) = self.candidates.iter_mut().find(|c| !c.special) {
                            *slot = candidate;
                        }
                    }
                }
                at += 4;
            }
            if depth >= MAX_DEPTH {
                continue;
            }
            let mut offset = 0;
            while offset + 8 <= length {
                let pointer = usize::from_le_bytes(std::array::from_fn(|i| bytes[offset + i]));
                if pointer > 0x10000
                    && pointer < 0x7fff_ffff_0000
                    && pointer % 8 == 0
                    && !(image_start..image_end).contains(&pointer)
                    && search.nodes.len() < MAX_NODES
                    && search.seen.insert(pointer)
                {
                    search.nodes.push(Node {
                        address: pointer,
                        depth: depth + 1,
                        parent: Some(index),
                        offset: offset as u32,
                        class: None,
                        // Children of the level-up classes inherit the deep read.
                        special: inside,
                    });
                    let child = search.nodes.len() - 1;
                    if inside {
                        search.first.push_back(child);
                    } else {
                        search.queue.push_back(child);
                    }
                }
                offset += 8;
            }
        }
        false
    }
}

/// The walk starts at CSMenuMan.
unsafe fn start() -> Option<Search> {
    let menu = unsafe { CSMenuManImp::instance() }.ok()?;
    let address = menu as *const CSMenuManImp as usize;
    let mut search = Search::default();
    search.seen.insert(address);
    search.nodes.push(Node {
        address,
        depth: 0,
        parent: None,
        offset: 0,
        class: Some("CSMenuMan".into()),
        special: false,
    });
    search.queue.push_back(0);
    Some(search)
}

/// The game executable's mapped image (vtables live there, objects do not).
fn image_range() -> (usize, usize) {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let Ok(module) = (unsafe { GetModuleHandleW(None) }) else {
        return (0, 0);
    };
    let base = module.0 as usize;
    // DOS header -> NT headers -> OptionalHeader64.SizeOfImage.
    let nt = base + unsafe { std::ptr::read_unaligned((base + 0x3c) as *const u32) } as usize;
    let size = unsafe { std::ptr::read_unaligned((nt + 0x18 + 0x38) as *const u32) } as usize;
    (base, base + size)
}

/// Where your real Vigor..Arcane live (never a preview).
unsafe fn real_attributes() -> usize {
    unsafe { GameDataMan::instance() }
        .map(|data| {
            data.main_player_game_data.as_ptr() as usize
                + std::mem::offset_of!(eldenring::cs::PlayerGameData, vigor)
        })
        .unwrap_or(0)
}

/// "CSMenuMan+0x10 > CS::PlayerStatusCalculator+0x58 > ...+0x1a4".
fn path_of(search: &Search, index: usize, at: u32) -> String {
    let mut steps = Vec::new();
    let mut current = Some(index);
    let mut offset = at;
    while let Some(i) = current {
        let node = &search.nodes[i];
        let name = node.class.clone().unwrap_or_else(|| "?".into());
        steps.push(format!("{name}+{offset:#x}"));
        offset = node.offset;
        current = node.parent;
    }
    steps.reverse();
    steps.join(" > ")
}

/// Eight attributes at `address`, if readable.
unsafe fn read_values(address: usize, memory: &mut Readable) -> Option<[u32; 8]> {
    if address % 4 != 0 || !memory.check(address & !7, 40) {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(address as *const [u32; 8]) })
}

/// Points above `committed` when every value is a possible page state (none
/// below your confirmed value, none above 99, at most 99 new points).
fn raised(values: [u32; 8], committed: [u32; 8]) -> Option<u32> {
    let mut gain = 0;
    for (v, c) in values.iter().zip(committed) {
        if *v < c || *v > 99 {
            return None;
        }
        gain += v - c;
    }
    (gain <= 99).then_some(gain)
}

/// VirtualQuery-checked reads (as world.rs; confirmed regions are cached).
#[derive(Default)]
struct Readable {
    ranges: Vec<(usize, usize)>,
    queries: u32,
}
impl Readable {
    fn region(&mut self, p: usize) -> Option<(usize, usize)> {
        use windows::Win32::System::Memory::{
            MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
        };
        if let Some(&range) = self.ranges.iter().find(|&&(a, b)| p >= a && p < b) {
            return Some(range);
        }
        if self.queries >= 8192 {
            return None;
        }
        self.queries += 1;
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let n = unsafe {
            VirtualQuery(
                Some(p as *const std::ffi::c_void),
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if n == 0
            || info.State != MEM_COMMIT
            || info.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) != 0
            || info.Protect.0 == 0
        {
            return None;
        }
        let range = (info.BaseAddress as usize, info.BaseAddress as usize + info.RegionSize);
        if range.1 <= p {
            return None;
        }
        if self.ranges.len() < 4096 {
            self.ranges.push(range);
        }
        Some(range)
    }
    fn check(&mut self, p: usize, len: usize) -> bool {
        if p < 0x10000 || p % 8 != 0 {
            return false;
        }
        self.readable_len(p, len) >= len
    }
    /// How many of `len` bytes from `p` can be read (whole regions only).
    fn readable_len(&mut self, p: usize, len: usize) -> usize {
        if p < 0x10000 {
            return 0;
        }
        let Some(end) = p.checked_add(len) else {
            return 0;
        };
        let mut at = p;
        while at < end {
            match self.region(at) {
                Some((_, b)) => at = b,
                None => break,
            }
        }
        at.min(end) - p
    }
}
