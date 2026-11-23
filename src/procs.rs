//! Proc engine: owned proc powers fire real Elden Ring bullets (cloned player
//! spell chains with Ascension's own fixed-damage AtkParam rows). Damage is
//! written into those rows right before each spawn from the card level,
//! character level, attributes and NG+ cycle. Native game task only.
//!
//! Every spawn creates a bullet *emitter* (CSBulletManager's second pool, 64 +
//! 192 records of 0x9d0). The game frees an emitter only through ReleaseEmitter
//! when the attack that made it ends; spawns made outside an attack would leak
//! until a loading screen and then nothing could spawn (error 4). The engine
//! records each emitter id and releases it once none of its bullets is alive.
//! Layout and RVAs are from eldenring.exe 2.7.2.0 (verified against the exe on
//! disk; the function heads are checked again before first use).
use crate::{
    powers::{Attribute, AttributeValues, ProcSpec, ProcTrigger},
    world::{WorldScan, WorldTracker},
};
use eldenring::cs::{AtkParam_Pc, BulletSpawnData, CSBulletManager, FieldInsHandle, SoloParamRepository};
use fromsoftware_shared::FromStatic;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    time::{Duration, Instant},
};

const SPAWN_RVA: usize = 0x3a2cb0;
const SPAWN_HEAD: [u8; 12] = [0x40, 0x53, 0x55, 0x56, 0x57, 0x48, 0x81, 0xEC, 0x98, 0x07, 0x00, 0x00];
const RELEASE_RVA: usize = 0x3a3960;
const RELEASE_HEAD: [u8; 19] = [
    0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x0F, 0xB6, 0xC2, 0x48, 0x8B, 0xF1,
    0x83, 0xF8, 0x40,
];
/// Emitters in use (all owners) above which the engine stops spawning, so the
/// enemies' own spells and arrows always keep free records.
const POOL_GUARD: u32 = 200;
/// Spawns per rolling second across all procs.
const SPAWN_BUDGET: usize = 12;
/// A proc timer only runs while the player is fighting.
const COMBAT_WINDOW: f32 = 8.0;

type FnSpawn = unsafe extern "C" fn(*mut CSBulletManager, *mut i32, *const BulletSpawnData, *mut i32);
type FnRelease = unsafe extern "C" fn(*mut CSBulletManager, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Spawn {
    Shoulder,
    Chest,
    AtSelf,
    AtTarget,
    AboveTarget,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Aim {
    Target,
    Forward,
    Down,
    Up,
}

#[derive(Deserialize)]
struct CatalogFile {
    schema: u32,
    procs: Vec<CatalogProc>,
}
#[derive(Deserialize)]
struct CatalogProc {
    id: String,
    name: String,
    element: BTreeMap<String, f32>,
    scaling: BTreeMap<Attribute, f32>,
    spawn: Spawn,
    aim: Aim,
    homing: bool,
    range: f32,
    #[allow(dead_code)]
    root: i32,
    #[serde(default)]
    #[allow(dead_code)]
    overrides: serde_json::Value,
}
#[derive(Deserialize)]
struct RowsFile {
    schema: u32,
    procs: Vec<RowsProc>,
}
#[derive(Deserialize)]
struct RowsProc {
    id: String,
    root_bullet: i32,
    atk_rows: Vec<(u32, f32)>,
    speed: f32,
}

#[derive(Debug, Clone)]
pub struct ProcDef {
    pub id: String,
    pub name: String,
    root_bullet: i32,
    atk_rows: Vec<(u32, f32)>,
    /// physical, magic, fire, lightning, holy
    pub element: [f32; 5],
    scaling: Vec<(Attribute, f32)>,
    spawn: Spawn,
    aim: Aim,
    homing: bool,
    range: f32,
    speed: f32,
}

/// One owned, currently active proc power.
pub struct ActiveProc<'a> {
    pub power: &'a str,
    pub level: u8,
    pub spec: &'a ProcSpec,
}

/// Per-tick character context for damage and triggers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcContext {
    pub character_level: u32,
    pub attributes: AttributeValues,
    pub ng_plus: u32,
    pub boss_fight: bool,
    pub dodge_started: bool,
    /// HP lost this tick as a share of max HP.
    pub hurt_share: f32,
    /// 3.3: damage multipliers of all your effects (physical, magic, fire,
    /// lightning, holy); 0 = unknown.
    pub damage_rates: [f32; 5],
}

#[derive(Debug, Clone, Default)]
pub struct ProcSummary {
    pub spawned: u32,
    pub failures: Vec<String>,
    /// 3.3 No Quarter: spell damage for a foe that cannot be hit yet,
    /// delivered by pursuit.rs when the spell lands: (foe, damage, when).
    pub deliveries: Vec<(FieldInsHandle, f32, Instant)>,
}

/// UI-facing state of one proc power (cooldown fraction for the HUD).
#[derive(Debug, Clone, Default)]
pub struct ProcStatus {
    pub power: String,
    pub ready_in: f32,
    pub period: f32,
    pub fired: u32,
    pub last_damage: u32,
    /// Streak procs: (hits in the window now, hits needed).
    pub progress: Option<(u16, u16)>,
}

pub struct ProcEngine {
    defs: BTreeMap<String, ProcDef>,
    load_error: Option<String>,
    spawn: Option<FnSpawn>,
    release: Option<FnRelease>,
    ready: bool,
    cooldown: BTreeMap<String, Instant>,
    fired: BTreeMap<String, (u32, u32)>,
    budget: VecDeque<Instant>,
    emitters: Vec<(i32, Instant, bool)>,
    /// Whether emitter records keep the owner handle at +0x330, decided on the
    /// first spawn. On 2.7.2.0 they do not (CS2 conversion log): ids decide.
    owner_at_330: Option<bool>,
    rng: u64,
    pub total_spawned: u64,
    pub total_failed: u64,
    pub last_error_code: Option<i32>,
    /// 3.2: spells waiting for their target to be hittable.
    deferred: Vec<Deferred>,
}

/// 3.2: a spell from a critical, a backstab, a parry or a stance break. A
/// critical's victim cannot be hit while thrown and getting back up, and a
/// parried or stance-broken foe is usually critted at once, so the spell
/// waits: no critical playing, the foe not down and not invulnerable, at
/// least `min_delay` s; 8 s at most. A foe that died gives it to the nearest
/// other foe.
#[derive(Clone, Debug)]
struct Deferred {
    power: String,
    proc: String,
    spec_power: f32,
    count: u8,
    handle: FieldInsHandle,
    created: Instant,
    min_delay: f32,
}

impl ProcEngine {
    /// Load the proc catalog and the generated row manifest. A missing or
    /// inconsistent manifest disables proc powers (they stay owned, inactive).
    pub fn load(root: &Path) -> Self {
        let mut engine = Self::default();
        engine.load_error = None;
        match load_defs(root) {
            Ok(defs) => engine.defs = defs,
            Err(error) => engine.load_error = Some(error),
        }
        engine
    }
}
impl Default for ProcEngine {
    fn default() -> Self {
        Self {
            defs: BTreeMap::new(),
            load_error: Some("proc catalog not loaded".into()),
            spawn: None,
            release: None,
            ready: false,
            cooldown: BTreeMap::new(),
            fired: BTreeMap::new(),
            budget: VecDeque::new(),
            emitters: Vec::new(),
            owner_at_330: None,
            rng: seed(),
            total_spawned: 0,
            total_failed: 0,
            last_error_code: None,
            deferred: Vec::new(),
        }
    }
}
impl ProcEngine {
    pub fn is_ready(&self) -> bool {
        self.ready
    }
    pub fn defs(&self) -> &BTreeMap<String, ProcDef> {
        &self.defs
    }
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Verify both engine functions and every owned AtkParam row.
    ///
    /// # Safety
    /// Native game task, pinned executable, generated package loaded.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        if self.ready {
            return Ok(());
        }
        if let Some(error) = &self.load_error {
            return Err(format!("proc catalog unavailable: {error}"));
        }
        let base = exe_base();
        if base == 0 {
            return Err("executable base unavailable".into());
        }
        let head = |rva: usize, len: usize| unsafe { std::slice::from_raw_parts((base + rva) as *const u8, len) };
        if head(SPAWN_RVA, SPAWN_HEAD.len()) != SPAWN_HEAD {
            return Err("bullet spawn function signature differs; procs disabled".into());
        }
        if head(RELEASE_RVA, RELEASE_HEAD.len()) != RELEASE_HEAD {
            return Err("bullet emitter release signature differs; procs disabled".into());
        }
        let repo = unsafe { SoloParamRepository::instance() }
            .map_err(|e| format!("param repository unavailable: {e}"))?;
        for def in self.defs.values() {
            if repo.get::<eldenring::cs::Bullet>(def.root_bullet as u32).is_none() {
                return Err(format!("proc {} bullet {} missing; regenerate the package", def.id, def.root_bullet));
            }
            for (id, _) in &def.atk_rows {
                if repo.get::<AtkParam_Pc>(*id).is_none() {
                    return Err(format!("proc {} attack row {id} missing", def.id));
                }
            }
        }
        self.spawn = Some(unsafe { std::mem::transmute::<usize, FnSpawn>(base + SPAWN_RVA) });
        self.release = Some(unsafe { std::mem::transmute::<usize, FnRelease>(base + RELEASE_RVA) });
        self.ready = true;
        Ok(())
    }

    /// A loading screen rebuilt the bullet manager: ids and timers are stale.
    pub fn forget_world(&mut self) {
        self.emitters.clear();
        self.cooldown.clear();
        self.budget.clear();
        self.deferred.clear();
    }

    pub fn status(&self, active: &[ActiveProc], now: Instant, scan: &WorldScan) -> Vec<ProcStatus> {
        active
            .iter()
            .map(|proc| {
                let period = proc.spec.trigger.period();
                let ready_in = self
                    .cooldown
                    .get(proc.power)
                    .map(|at| (period - now.duration_since(*at).as_secs_f32()).max(0.0))
                    .unwrap_or(0.0);
                let (fired, last_damage) = self.fired.get(proc.power).copied().unwrap_or_default();
                ProcStatus {
                    power: proc.power.to_owned(),
                    ready_in,
                    period,
                    fired,
                    last_damage,
                    progress: match proc.spec.trigger {
                        ProcTrigger::Streak { hits, window, .. } => {
                            Some((scan.hits_within(window).min(hits), hits))
                        }
                        _ => None,
                    },
                }
            })
            .collect()
    }

    /// # Safety
    /// Native game task only; `scan` is this tick's world scan.
    /// `window`: No Quarter's open window (victim, real hits land on it).
    pub unsafe fn tick(
        &mut self,
        active: &[ActiveProc],
        scan: &WorldScan,
        tracker: &mut WorldTracker,
        ctx: &ProcContext,
        now: Instant,
        window: Option<(FieldInsHandle, bool)>,
    ) -> ProcSummary {
        let mut summary = ProcSummary::default();
        unsafe { self.release_emitters() };
        if !self.ready || !scan.ready || scan.player.is_none() || active.is_empty() {
            return summary;
        }
        while self
            .budget
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
        {
            self.budget.pop_front();
        }
        let in_combat = ctx.boss_fight || scan.seconds_since_combat.is_some_and(|s| s < COMBAT_WINDOW);
        for proc in active {
            let Some(def) = self.defs.get(&proc.spec.proc).cloned() else {
                continue;
            };
            let since = self
                .cooldown
                .get(proc.power)
                .map(|at| now.duration_since(*at).as_secs_f32())
                .unwrap_or(f32::INFINITY);
            if since < proc.spec.trigger.period() {
                continue;
            }
            // (anchor position, homing target)
            let mut shots: Vec<([f32; 3], Option<FieldInsHandle>)> = Vec::new();
            let mut defer: Option<(crate::world::EventTarget, f32)> = None;
            match proc.spec.trigger {
                ProcTrigger::Interval { .. } => {
                    if in_combat {
                        // `count` bolts spread round-robin over the engaged
                        // targets; a lone boss receives the whole volley.
                        let targets = pick_targets(scan, def.range, proc.spec.count.max(1));
                        if !targets.is_empty() {
                            for i in 0..proc.spec.count.max(1) as usize {
                                let foe = targets[i % targets.len()];
                                shots.push((foe.position, Some(foe.handle)));
                            }
                        }
                    }
                }
                ProcTrigger::Hit { chance, .. } => {
                    if scan.hits > 0 && self.roll(chance) {
                        if let Some(foe) = scan
                            .last_hit_target
                            .and_then(|h| scan.foes.iter().find(|f| f.handle == h))
                        {
                            shots.push((foe.position, Some(foe.handle)));
                        }
                    }
                }
                ProcTrigger::Streak { hits, window, .. } => {
                    if scan.hits > 0 && scan.hits_within(window) >= hits {
                        if let Some(foe) = scan
                            .last_hit_target
                            .and_then(|h| scan.foes.iter().find(|f| f.handle == h))
                        {
                            shots.push((foe.position, Some(foe.handle)));
                        }
                    }
                }
                ProcTrigger::Kill { chance, .. } => {
                    if let Some(corpse) = scan.kills.first() {
                        if self.roll(chance) {
                            let homing = nearest_to(scan, *corpse, 12.0).map(|f| f.handle);
                            shots.push((*corpse, homing));
                        }
                    }
                }
                ProcTrigger::Dodge { .. } => {
                    if ctx.dodge_started {
                        shots.push((scan.player_position, None));
                    }
                }
                ProcTrigger::Hurt { min_share, .. } => {
                    if ctx.hurt_share >= min_share {
                        shots.push((scan.player_position, None));
                    }
                }
                // 0.6 reflexes: at the enemy the event happened to.
                ProcTrigger::PerfectDodge { .. } => {
                    if let Some(target) = scan.reflex.perfect_dodge {
                        shots.push(aim_at(&def, scan, target));
                    }
                }
                ProcTrigger::Block { chance, .. } => {
                    if let Some(target) = scan.reflex.block {
                        if self.roll(chance) {
                            shots.push(aim_at(&def, scan, target));
                        }
                    }
                }
                ProcTrigger::GuardCounter { .. } => {
                    if let Some(target) = scan.reflex.guard_counter {
                        shots.push(aim_at(&def, scan, target));
                    }
                }
                // 3.2: these wait until the foe can be hit (Deferred).
                ProcTrigger::Critical { .. } => {
                    if let Some(target) = scan.reflex.critical {
                        defer = Some((target, 0.4));
                    }
                }
                ProcTrigger::StanceBreak { .. } => {
                    if let Some(target) = scan.reflex.stance_breaks.first() {
                        defer = Some((*target, 1.2));
                    }
                }
                // 0.7: parry and backstab, at the enemy.
                ProcTrigger::Parry { .. } => {
                    if let Some(target) = scan.reflex.parry {
                        defer = Some((target, 1.0));
                    }
                }
                ProcTrigger::Backstab { .. } => {
                    if let Some(target) = scan.reflex.backstab {
                        defer = Some((target, 0.4));
                    }
                }
                // 0.9: a kill with a critical, three kills in a row: at the body.
                ProcTrigger::Execution { .. } => {
                    if let Some(target) = scan.reflex.execution {
                        shots.push(aim_at(&def, scan, target));
                    }
                }
                ProcTrigger::Rampage { .. } => {
                    if let Some(target) = scan.reflex.rampage {
                        shots.push(aim_at(&def, scan, target));
                    }
                }
                ProcTrigger::Status { status, .. } => {
                    if let Some((_, target)) = scan
                        .reflex
                        .status_procs
                        .iter()
                        .find(|(kind, _)| status.matches(*kind))
                    {
                        shots.push(aim_at(&def, scan, *target));
                    }
                }
            }
            if let Some((target, min_delay)) = defer {
                match target.handle {
                    Some(handle) => {
                        if self.deferred.len() < 16 {
                            self.deferred.push(Deferred {
                                power: proc.power.to_owned(),
                                proc: proc.spec.proc.clone(),
                                spec_power: proc.spec.power,
                                count: proc.spec.count,
                                handle,
                                created: now,
                                min_delay,
                            });
                        }
                        // The cooldown starts now; the spell flies later.
                        self.cooldown.insert(proc.power.to_owned(), now);
                        continue;
                    }
                    None => shots.push(aim_at(&def, scan, target)),
                }
            }
            if shots.is_empty() {
                continue;
            }
            let interval = matches!(proc.spec.trigger, ProcTrigger::Interval { .. });
            unsafe {
                self.cast(&def, proc.power, proc.spec.power, proc.spec.count, interval, true, &shots, scan, tracker, ctx, now, window, &mut summary)
            };
        }
        // 3.2: spells waiting for their target.
        let throwing = scan.player_state.in_critical();
        let mut due = Vec::new();
        self.deferred.retain(|d| {
            let age = now.duration_since(d.created).as_secs_f32();
            let foe = scan.foes.iter().find(|f| f.handle == d.handle);
            // Down after a critical: wait until it can be hit, unless No
            // Quarter's window is open on it (then the spell lands natively
            // or its damage is delivered).
            let quarter = window.is_some_and(|(h, _)| h == d.handle);
            let downed = scan.reflex.downed.is_some_and(|x| x.handle == d.handle && !x.landed) && !quarter;
            let guarded = foe.is_some_and(|f| f.invulnerable) && !quarter;
            let hittable = foe.is_some() && !guarded && !throwing && !downed && age >= d.min_delay;
            if foe.is_none() || hittable || age >= 8.0 {
                due.push(d.clone());
                false
            } else {
                true
            }
        });
        for d in due {
            let Some(def) = self.defs.get(&d.proc).cloned() else {
                continue;
            };
            let target = scan
                .foes
                .iter()
                .find(|f| f.handle == d.handle)
                .or_else(|| {
                    scan.foes
                        .iter()
                        .filter(|f| f.engaged && !f.protected && f.distance <= def.range)
                        .min_by(|a, b| a.distance.total_cmp(&b.distance))
                })
                .copied();
            let Some(foe) = target else {
                continue;
            };
            let shots = [(foe.position, Some(foe.handle))];
            unsafe { self.cast(&def, &d.power, d.spec_power, d.count, false, false, &shots, scan, tracker, ctx, now, window, &mut summary) };
        }
        summary
    }

    /// Spawn one proc's bullets at `shots` (anchor, homing target).
    #[allow(clippy::too_many_arguments)]
    unsafe fn cast(
        &mut self,
        def: &ProcDef,
        power_id: &str,
        spec_power: f32,
        count: u8,
        interval: bool,
        start_cooldown: bool,
        shots: &[([f32; 3], Option<FieldInsHandle>)],
        scan: &WorldScan,
        tracker: &mut WorldTracker,
        ctx: &ProcContext,
        now: Instant,
        window: Option<(FieldInsHandle, bool)>,
        summary: &mut ProcSummary,
    ) {
        let damage = proc_damage(def, spec_power, ctx);
        if let Err(error) = unsafe { write_damage(def, damage) } {
            summary.failures.push(error);
            return;
        }
        let bullets = if interval {
            // Interval procs spread `count` over distinct targets; one each.
            1
        } else {
            count.max(1)
        };
            let mut fired = false;
            let mut spawned_here = 0u32;
            let mut index: u8 = 0;
            for (anchor, target) in shots {
                for _ in 0..bullets {
                    if self.budget.len() >= SPAWN_BUDGET {
                        break;
                    }
                    let (origin, dir) = geometry(def, scan, *anchor, index);
                    index = index.wrapping_add(1);
                    match unsafe { self.spawn_one(def, origin, dir, target.filter(|_| def.homing)) } {
                        Ok(()) => {
                            fired = true;
                            spawned_here += 1;
                            summary.spawned += 1;
                            self.budget.push_back(now);
                        }
                        Err(code) => {
                            self.total_failed += 1;
                            self.last_error_code = Some(code);
                            summary
                                .failures
                                .push(format!("{} spawn failed with code {code}", def.id));
                        }
                    }
                }
                // Damage on foes near this anchor before the bullet lands is
                // proc damage, not a player hit.
                let (travel, radius) = match def.spawn {
                    Spawn::Shoulder | Spawn::Chest => {
                        let d = distance(scan.player_position, *anchor);
                        ((d / def.speed.max(5.0)).min(3.0) + 0.6, 2.5)
                    }
                    Spawn::AboveTarget => (1.6, 3.0),
                    Spawn::AtTarget => (1.0, 5.0),
                    Spawn::AtSelf => (0.6, 4.5),
                };
                let until = now + Duration::from_secs_f32(travel);
                // 3.3 No Quarter: a downed victim whose invincibility the game
                // keeps gets this spell's damage delivered when it lands.
                if let Some((victim, false)) = window {
                    if *target == Some(victim) || scan.foes.iter().any(|f| f.handle == victim && distance(f.position, *anchor) < radius) {
                        let boost: f32 = def
                            .element
                            .iter()
                            .zip(ctx.damage_rates)
                            .map(|(share, rate)| share * if rate > 0.0 { rate } else { 1.0 })
                            .sum::<f32>()
                            .max(0.1);
                        summary.deliveries.push((victim, damage * boost * f32::from(bullets), until));
                    }
                }
                for foe in &scan.foes {
                    if distance(foe.position, *anchor) < radius || Some(foe.handle) == *target {
                        tracker.mark_echo(foe.handle, until);
                    }
                }
            }
            if fired {
                if start_cooldown {
                    self.cooldown.insert(power_id.to_owned(), now);
                }
                let entry = self.fired.entry(power_id.to_owned()).or_default();
                entry.0 = entry.0.saturating_add(1);
                entry.1 = damage.round().clamp(0.0, u32::MAX as f32) as u32;
                self.total_spawned += u64::from(spawned_here);
            }
    }

    fn roll(&mut self, chance: f32) -> bool {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        ((self.rng >> 11) as f64 / (1u64 << 53) as f64) < f64::from(chance.clamp(0.0, 1.0))
    }

    unsafe fn spawn_one(
        &mut self,
        def: &ProcDef,
        origin: [f32; 3],
        dir: [f32; 3],
        target: Option<FieldInsHandle>,
    ) -> Result<(), i32> {
        let spawn = self.spawn.ok_or(-100)?;
        let world = unsafe { eldenring::cs::WorldChrMan::instance() }.map_err(|_| -101)?;
        let player = world.main_player.as_ref().ok_or(-102)?;
        let owner = player.chr_ins.field_ins_handle;
        let manager = unsafe { CSBulletManager::instance_mut() }.map_err(|_| -103)?;
        let base = manager as *mut CSBulletManager as *mut u8;
        let in_use = unsafe { *(base.add(0x1d4) as *const u32) };
        if in_use >= POOL_GUARD {
            return Err(-200);
        }
        let mut buffer = SpawnBuf::new();
        buffer.put_u64(0x00, handle_bits(owner));
        buffer.put_i32(0x08, -1);
        buffer.put_i32(0x0C, -1);
        buffer.put_i32(0x14, def.root_bullet);
        buffer.put_i32(0x18, -1);
        buffer.put_i32(0x1C, -1);
        if let Some(target) = target {
            buffer.put_u64(0x20, handle_bits(target));
        }
        buffer.put_i32(0x44, 0);
        // The game's own spawns encode the direction as (horizontal unit, tan(pitch)).
        let h = (dir[0] * dir[0] + dir[2] * dir[2]).sqrt().max(1e-3);
        let d = [dir[0] / h, dir[1] / h, dir[2] / h];
        buffer.put_f32(0x50, d[2]);
        buffer.put_f32(0x58, -d[0]);
        buffer.put_u64(0x60, 0);
        buffer.put_f32(0x68, d[1]);
        buffer.put_f32(0x70, d[0]);
        buffer.put_f32(0x74, d[1]);
        buffer.put_f32(0x78, d[2]);
        buffer.put_f32(0x80, origin[0]);
        buffer.put_f32(0x84, origin[1]);
        buffer.put_f32(0x88, origin[2]);
        buffer.put_f32(0x8C, 1.0);
        let mut emitter = 0i32;
        let mut error = 0i32;
        unsafe {
            spawn(
                manager,
                &mut emitter,
                buffer.0.as_ptr() as *const BulletSpawnData,
                &mut error,
            )
        };
        if emitter == -1 {
            return Err(error);
        }
        if self.owner_at_330.is_none() {
            let base = manager as *mut CSBulletManager as *const u8;
            self.owner_at_330 = unsafe { emitter_ptr(base, emitter) }
                .map(|record| unsafe { *(record.add(0x330) as *const u64) } == handle_bits(owner));
        }
        self.emitters.push((emitter, Instant::now(), false));
        Ok(())
    }

    /// Release every emitter of ours whose bullets are gone (or that never
    /// showed one within 2 s; 30 s at most). Only emitters owned by the player.
    unsafe fn release_emitters(&mut self) {
        if self.emitters.is_empty() {
            return;
        }
        let Some(release) = self.release else {
            self.emitters.clear();
            return;
        };
        let Ok(manager) = (unsafe { CSBulletManager::instance_mut() }) else {
            return;
        };
        let base = manager as *mut CSBulletManager as *mut u8;
        if unsafe { *(base.add(0x1d4) as *const u32) } == 0 {
            // A loading screen emptied the pool: the ids are stale.
            self.emitters.clear();
            return;
        }
        let me = unsafe { eldenring::cs::WorldChrMan::instance() }
            .ok()
            .and_then(|w| w.main_player.as_ref().map(|p| handle_bits(p.chr_ins.field_ins_handle)));
        let alive: Vec<i32> = manager
            .bullets()
            .map(|b| unsafe { *((b as *const _ as *const u8).add(0x54) as *const i32) })
            .collect();
        let manager = manager as *mut CSBulletManager;
        let owner_check = self.owner_at_330 == Some(true);
        self.emitters.retain_mut(|(id, at, seen)| {
            let Some(record) = (unsafe { emitter_ptr(manager as *const u8, *id) }) else {
                return false;
            };
            if unsafe { *record.add(0x328) } == 0 {
                return false;
            }
            if owner_check && me.is_some_and(|me| unsafe { *(record.add(0x330) as *const u64) } != me) {
                return false;
            }
            let flying = alive.contains(id);
            *seen |= flying;
            let age = at.elapsed().as_secs_f32();
            if (!flying && (*seen || age > 2.0)) || age > 30.0 {
                unsafe { release(manager, *id) };
                return false;
            }
            true
        });
    }
}

impl ProcTrigger {
    /// Minimum seconds between activations of one power.
    pub fn period(&self) -> f32 {
        match *self {
            Self::Interval { seconds } => seconds,
            Self::Hit { cooldown, .. }
            | Self::Streak { cooldown, .. }
            | Self::Kill { cooldown, .. }
            | Self::Dodge { cooldown }
            | Self::Hurt { cooldown, .. }
            | Self::PerfectDodge { cooldown }
            | Self::Block { cooldown, .. }
            | Self::GuardCounter { cooldown }
            | Self::Critical { cooldown }
            | Self::StanceBreak { cooldown }
            | Self::Parry { cooldown }
            | Self::Backstab { cooldown }
            | Self::Execution { cooldown }
            | Self::Rampage { cooldown }
            | Self::Status { cooldown, .. } => cooldown,


        }
    }
}

/// Reference damage of a proc: card power × character level curve × attribute
/// curve × NG+ cycle. Bounded; the engine's damage cuts and defenses apply.
pub fn proc_damage(def: &ProcDef, power: f32, ctx: &ProcContext) -> f32 {
    let level = ctx.character_level.clamp(1, 713) as f32;
    let level_curve = 90.0 + 4.2 * level;
    let attribute: f32 = def
        .scaling
        .iter()
        .map(|(attribute, weight)| weight * ctx.attributes.get(*attribute).min(150) as f32)
        .sum();
    let attribute_curve = 0.75 + attribute / 80.0;
    let cycle = 1.0 + 0.15 * ctx.ng_plus.min(7) as f32;
    (power.clamp(0.0, 10.0) * level_curve * attribute_curve * cycle).clamp(1.0, 60_000.0)
}

/// Element shares of a proc for the collection screen.
pub fn element_label(def: &ProcDef) -> String {
    const NAMES: [&str; 5] = ["physical", "magic", "fire", "lightning", "holy"];
    let parts: Vec<String> = def
        .element
        .iter()
        .zip(NAMES)
        .filter(|(share, _)| **share > 0.0)
        .map(|(share, name)| {
            if *share >= 0.999 {
                name.to_owned()
            } else {
                format!("{name} {:.0}%", share * 100.0)
            }
        })
        .collect();
    parts.join(" + ")
}

unsafe fn write_damage(def: &ProcDef, damage: f32) -> Result<(), String> {
    let repo = unsafe { SoloParamRepository::instance_mut() }
        .map_err(|e| format!("param repository unavailable: {e}"))?;
    for (id, weight) in &def.atk_rows {
        let row = repo
            .get_mut::<AtkParam_Pc>(*id)
            .ok_or_else(|| format!("proc attack row {id} disappeared"))?;
        let value = |share: f32| (damage * weight * share).round().clamp(0.0, 60_000.0) as u16;
        row.set_atk_phys(value(def.element[0]));
        row.set_atk_mag(value(def.element[1]));
        row.set_atk_fire(value(def.element[2]));
        row.set_atk_thun(value(def.element[3]));
        row.set_atk_dark(value(def.element[4]));
    }
    Ok(())
}

/// Engaged foes in range first, then unprotected foes within 10 m (so an
/// automatic proc never starts a fight with a sleeping camp far away).
fn pick_targets(scan: &WorldScan, range: f32, count: u8) -> Vec<crate::world::Foe> {
    let mut picked: Vec<crate::world::Foe> = scan
        .foes
        .iter()
        .filter(|f| f.engaged && f.distance <= range)
        .copied()
        .collect();
    if picked.len() < count as usize {
        for foe in scan
            .foes
            .iter()
            .filter(|f| !f.engaged && !f.protected && f.distance <= range.min(10.0))
        {
            if picked.len() >= count as usize {
                break;
            }
            picked.push(*foe);
        }
    }
    picked.truncate(count as usize);
    picked
}

/// Anchor and homing target for a reflex proc: spells cast around the player
/// stay at the player; everything else goes to the enemy (its live position
/// when it is still in the scan).
fn aim_at(def: &ProcDef, scan: &WorldScan, target: crate::world::EventTarget) -> ([f32; 3], Option<FieldInsHandle>) {
    if def.spawn == Spawn::AtSelf {
        return (scan.player_position, None);
    }
    let live = target
        .handle
        .and_then(|h| scan.foes.iter().find(|f| f.handle == h));
    match live {
        Some(foe) if foe.distance <= def.range => (foe.position, Some(foe.handle)),
        _ => match nearest_to(scan, target.position, 6.0) {
            Some(foe) if foe.distance <= def.range => (foe.position, Some(foe.handle)),
            // No enemy known: aim a few metres ahead instead of at the player.
            _ if distance(target.position, scan.player_position) < 1.0 => {
                let p = scan.player_position;
                let f = scan.player_forward;
                ([p[0] + f[0] * 6.0, p[1], p[2] + f[2] * 6.0], None)
            }
            _ => (target.position, None),
        },
    }
}
fn nearest_to(scan: &WorldScan, point: [f32; 3], radius: f32) -> Option<&crate::world::Foe> {
    scan.foes
        .iter()
        .filter(|f| f.engaged || !f.protected)
        .map(|f| (distance(f.position, point), f))
        .filter(|(d, _)| *d <= radius && *d > 0.3)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f)| f)
}

/// Spawn point and direction for bullet `index` of one activation.
fn geometry(def: &ProcDef, scan: &WorldScan, anchor: [f32; 3], index: u8) -> ([f32; 3], [f32; 3]) {
    let p = scan.player_position;
    let f = scan.player_forward;
    let right = [f[2], 0.0, -f[0]];
    let side = if index % 2 == 0 { 1.0 } else { -1.0 } * (1.0 + (index / 2) as f32 * 0.6);
    let chest = [anchor[0], anchor[1] + 1.0, anchor[2]];
    let origin = match def.spawn {
        Spawn::Shoulder => [
            p[0] + right[0] * 0.45 * side,
            p[1] + 1.55 + 0.15 * (index / 2) as f32,
            p[2] + right[2] * 0.45 * side,
        ],
        Spawn::Chest => [p[0] + f[0] * 0.35, p[1] + 1.2, p[2] + f[2] * 0.35],
        Spawn::AtSelf => [p[0], p[1] + 0.5, p[2]],
        Spawn::AtTarget => {
            let spread = if index == 0 { 0.0 } else { 1.2 };
            [
                anchor[0] + right[0] * spread * side,
                anchor[1] + 0.4,
                anchor[2] + right[2] * spread * side,
            ]
        }
        Spawn::AboveTarget => [
            anchor[0] + right[0] * 0.8 * (index as f32) * side,
            anchor[1] + 7.0,
            anchor[2] + right[2] * 0.8 * (index as f32) * side,
        ],
    };
    let toward = normalize([chest[0] - origin[0], chest[1] - origin[1], chest[2] - origin[2]]);
    let flat = normalize([anchor[0] - p[0], 0.0, anchor[2] - p[2]]);
    let dir = match def.aim {
        Aim::Target => toward.unwrap_or(f),
        Aim::Forward => {
            if def.spawn == Spawn::AtTarget {
                flat.unwrap_or(f)
            } else {
                f
            }
        }
        Aim::Down => toward.unwrap_or([0.0, -1.0, 0.0]),
        Aim::Up => [0.0, 1.0, 0.0],
    };
    (origin, dir)
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length.is_finite() && length > 1e-3).then(|| [v[0] / length, v[1] / length, v[2] / length])
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
fn handle_bits(handle: FieldInsHandle) -> u64 {
    unsafe { std::mem::transmute_copy::<FieldInsHandle, u64>(&handle) }
}
fn exe_base() -> usize {
    use windows::Win32::System::LibraryLoader::GetModuleHandleA;
    unsafe { GetModuleHandleA(None) }
        .map(|h| h.0 as usize)
        .unwrap_or(0)
}
fn seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9);
    nanos | 1
}

/// The emitter record with this id, if it is still that emitter (pre-allocated
/// slots by index, the rest through the in-use list; ids carry a generation).
unsafe fn emitter_ptr(manager: *const u8, id: i32) -> Option<*const u8> {
    let index = (id as u32) & 0xFF;
    unsafe {
        if index < 0x40 {
            let buffer = *(manager.add(0x20) as *const *const u8);
            if buffer.is_null() {
                return None;
            }
            let record = buffer.add(index as usize * 0x9D0);
            return (*(record as *const i32) == id).then_some(record);
        }
        if index != 0xFE {
            return None;
        }
        let mut record = *(manager.add(0x28) as *const *const u8);
        let mut steps = 0;
        while !record.is_null() && steps < 1024 {
            if *(record as *const i32) == id {
                return Some(record);
            }
            record = *(record.add(0x998) as *const *const u8);
            steps += 1;
        }
    }
    None
}

/// Raw spawn buffer laid out like `BulletSpawnData` (0x110) with slack; the
/// defaults are a spawn the game itself produced (community capture,
/// naruto716/er), pointer values zeroed. Same layout the CS2 conversion uses.
#[repr(C, align(16))]
struct SpawnBuf([u8; 0x160]);
const TEMPLATE: [u64; 34] = [
    0xFFFFFFFF16900000, 0x00000FA0FFFFFFFF, 0x009EB10000000000, 0x00002774FFFFFFFF,
    0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0x3F8000003F8B851F, 0x3F8000003F898232,
    0x0000000900000000, 0x0000000000000001, 0x000000003F10C355, 0x00000000BF532395,
    0x3F80000180000000, 0x0000000000000000, 0x000000003F532395, 0x000000003F10C355,
    0x40A97E3FC0C8983B, 0x3F800000C08EB5B9, 0x0000000000000000, 0x0000000000000000,
    0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0xFFFFFFFFFFFFFFFF,
    0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0x0000000000000000,
    0x00000000FFFFFFFF, 0x0000000000000000, 0x3F800000FFFFFFFF, 0x3F8000003F898232,
    0x0000000000000000, 0x0000000000000000,
];
impl SpawnBuf {
    fn new() -> Self {
        let mut buffer = SpawnBuf([0; 0x160]);
        for (i, value) in TEMPLATE.iter().enumerate() {
            buffer.0[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
        }
        buffer
    }
    fn put_u64(&mut self, offset: usize, value: u64) {
        self.0[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put_i32(&mut self, offset: usize, value: i32) {
        self.0[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put_f32(&mut self, offset: usize, value: f32) {
        self.0[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn load_defs(root: &Path) -> Result<BTreeMap<String, ProcDef>, String> {
    let catalog: CatalogFile = serde_json::from_slice(
        &std::fs::read(root.join("config/procs.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let rows: RowsFile = serde_json::from_slice(
        &std::fs::read(root.join("config/procs-rows.json"))
            .map_err(|e| format!("procs-rows.json missing (run tools/build_params.py): {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    if catalog.schema != 1 || rows.schema != 1 {
        return Err("proc manifests require schema 1".into());
    }
    let mut defs = BTreeMap::new();
    for proc in catalog.procs {
        let resolved = rows
            .procs
            .iter()
            .find(|r| r.id == proc.id)
            .ok_or_else(|| format!("proc {} has no generated rows", proc.id))?;
        let mut element = [0.0f32; 5];
        for (kind, share) in &proc.element {
            let index = match kind.as_str() {
                "physical" => 0,
                "magic" => 1,
                "fire" => 2,
                "lightning" => 3,
                "holy" => 4,
                other => return Err(format!("proc {}: unknown element {other}", proc.id)),
            };
            element[index] = share.clamp(0.0, 1.0);
        }
        if (element.iter().sum::<f32>() - 1.0).abs() > 1e-3
            || resolved.atk_rows.is_empty()
            || resolved.atk_rows.iter().any(|(_, w)| !w.is_finite() || *w < 0.0 || *w > 20.0)
            || !proc.range.is_finite()
            || !(1.0..=60.0).contains(&proc.range)
        {
            return Err(format!("proc {} definition is invalid", proc.id));
        }
        defs.insert(
            proc.id.clone(),
            ProcDef {
                id: proc.id,
                name: proc.name,
                root_bullet: resolved.root_bullet,
                atk_rows: resolved.atk_rows.clone(),
                element,
                scaling: proc.scaling.into_iter().collect(),
                spawn: proc.spawn,
                aim: proc.aim,
                homing: proc.homing,
                range: proc.range,
                speed: if resolved.speed.is_finite() && resolved.speed > 0.0 {
                    resolved.speed
                } else {
                    15.0
                },
            },
        );
    }
    Ok(defs)
}
