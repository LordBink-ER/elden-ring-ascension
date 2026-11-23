//! Hostile-world observation for event-driven powers. Native game task only.
//! Each tick reads the loaded characters once; between ticks only handles and
//! plain values are kept, never pointers into the game.
use crate::powers::{STATUS_KINDS, StatusKind};
use eldenring::cs::{
    AtkParam_Pc, CSChrActionFlagModule, CSChrDataModule, CSChrPhysicsModule,
    CSChrSuperArmorModule, CSChrThrowModule, CSChrToughnessModule, CSChrTimeActModule, CSChrTimeActModuleAnim,
    CSThrowNode, ChrIns, ChrInsModuleContainer, ChrSet, ChrSetEntry, FieldInsHandle,
    FieldInsType, SoloParamRepository, WorldChrMan,
};
use fromsoftware_shared::FromStatic;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

/// Characters farther than this are not tracked at all.
pub const SCAN_RADIUS: f32 = 60.0;
/// A hit only counts when it removes at least this share of the target's HP
/// (or `MIN_HIT_POINTS`): poison/rot ticks keep `last_hit_by` but are not hits.
const MIN_HIT_SHARE: f32 = 0.02;
const MIN_HIT_POINTS: i32 = 25;
const ENGAGED_FOR: Duration = Duration::from_secs(20);
/// Hits count toward a streak for 6 s (a katana R1 chain lands about 1.5 hits
/// a second; 4 s made six-hit cards practically unreachable).
const STREAK_WINDOW: Duration = Duration::from_secs(6);
/// Hit times are kept this long so a card can use its own streak window.
const HIT_MEMORY: Duration = Duration::from_secs(15);
const KILL_WINDOW: Duration = Duration::from_secs(8);
/// 0.6: enemies this close are read for attacks aimed at the player.
const THREAT_RADIUS: f32 = 9.0;
/// 0.6: engaged enemies this close are read for stance and status changes.
const DETAIL_RADIUS: f32 = 35.0;
/// The resist module (module container +0x20, right after time_act): seven
/// current status values at +0x10 and their seven maxima at +0x2C, i32, in
/// STATUS_KINDS order. Measured on 2.7.2.0 by the CS2 conversion (player at
/// rest: values equal the maxima, they count down as buildup grows).
const RESIST_VALUES: usize = 0x10;
const RESIST_SIZE: usize = 0x48;
/// One status proc per enemy and kind within this time.
const STATUS_COOLDOWN: Duration = Duration::from_millis(2500);
/// 0.7 parry: how close the enemy stands, and how recently its attack was in
/// its parryable window (TAE action 5) before breaking off.
const PARRY_RADIUS: f32 = 4.5;
const PARRY_WINDOW: Duration = Duration::from_millis(350);
const PARRY_MIN_LEFT: f32 = 0.3;
/// 2.2 stance breaks: an engaged enemy's stance gauge (super armor or
/// toughness, both read) emptied or snapped back to full right after your
/// hit, once worn below 95%. Gauges smaller than this are ignored.
const MIN_STANCE: f32 = 10.0;
const STANCE_HIT_WINDOW: Duration = Duration::from_millis(800);
const STANCE_COOLDOWN: Duration = Duration::from_secs(3);
/// 2.2: HP lost by enemies next to you this long after your critical
/// (riposte, backstab) is yours.
const CRITICAL_CREDIT: Duration = Duration::from_secs(3);


/// Team types of the player, spirit summons, Torrent and friendly NPCs. They are
/// never auto-targeted (they still count when the player actually hits them).
fn protected_team(team: u8) -> bool {
    matches!(team, 0 | 1 | 2 | 8 | 10 | 26 | 27 | 28 | 30 | 47)
}

#[derive(Clone, Copy, Debug)]
pub struct Foe {
    pub handle: FieldInsHandle,
    pub position: [f32; 3],
    pub hp: i32,
    pub max_hp: i32,
    pub distance: f32,
    /// Hit by the player, or hit the player, within the last 20 s.
    pub engaged: bool,
    /// Never chosen automatically (friendly team) unless engaged.
    pub protected: bool,
    /// 0.7: how much the enemy faces the player (1 straight at, -1 away).
    pub facing: f32,
    /// 0.8: shows a boss health bar (the Tide leaves bosses to the boss scaler).
    pub boss: bool,
    /// 2.3: seconds since your last confirmed hit on it, and since its attack
    /// was last in its parryable window (tells a stance-break critical from a
    /// riposte after a parry); its animation now (0 = unknown).
    pub hit_age: Option<f32>,
    pub parryable_age: Option<f32>,
    pub anim: i32,
    /// 3.2: it cannot be hit right now (TAE invincibility, a throw victim,
    /// event invincibility): spells wait for it (procs.rs).
    pub invulnerable: bool,
    /// 3.2: your usual hit on it (median of your last ordinary hits), 0 =
    /// none yet.
    pub typical_hit: i32,
}

/// 0.7.1: status gauges of an engaged enemy, in resist-module order
/// (poison, scarlet rot, blood loss, death blight, frostbite, sleep, madness).
#[derive(Clone, Copy, Debug)]
pub struct Gauge {
    pub handle: FieldInsHandle,
    pub max: [i32; 7],
}

/// 0.7: an enemy attack that stopped early right after its parryable
/// window, close in front of the player (the reflex tracker decides).
#[derive(Clone, Copy, Debug)]
pub struct ParryCandidate {
    pub target: EventTarget,
    pub character_id: u32,
    /// Attack animation, how far it had played (0..1), what replaced it.
    pub from: i32,
    pub progress: f32,
    pub to: i32,
    pub distance: f32,
}

/// 0.6: an enemy close to the player in the active part of an attack
/// animation (local TAE id 3000-3999) and facing the player.
#[derive(Clone, Copy, Debug)]
pub struct Threat {
    pub handle: FieldInsHandle,
    pub position: [f32; 3],
    pub distance: f32,
    pub anim: i32,
    pub character_id: u32,
}

/// Where an event happened: the enemy (when known) or a point.
#[derive(Clone, Copy, Debug)]
pub struct EventTarget {
    pub position: [f32; 3],
    pub handle: Option<FieldInsHandle>,
}

/// 0.6 combat reflexes detected this tick (see reflexes.rs).
#[derive(Clone, Debug, Default)]
pub struct ReflexEvents {
    pub perfect_dodge: Option<EventTarget>,
    pub block: Option<EventTarget>,
    pub guard_counter: Option<EventTarget>,
    pub critical: Option<EventTarget>,
    pub stance_breaks: Vec<EventTarget>,
    pub status_procs: Vec<(StatusKind, EventTarget)>,
    /// 0.7
    pub parry: Option<EventTarget>,
    pub backstab: Option<EventTarget>,
    /// 0.9
    pub execution: Option<EventTarget>,
    pub rampage: Option<EventTarget>,
    /// 2.3: what this tick's critical was, and how many kills the rampage had.
    pub critical_kind: Option<CriticalKind>,
    pub rampage_kills: u16,
    /// 3.2: the victim of your last critical while it is down (the throw,
    /// then getting back up), until it can be hit again.
    pub downed: Option<Downed>,
}

/// 3.2: a critical's victim on its way back up.
#[derive(Clone, Copy, Debug)]
pub struct Downed {
    pub handle: FieldInsHandle,
    /// Seconds since your critical animation ended (None while it plays).
    pub since_throw: Option<f32>,
    /// 3.3: one of your real hits (not a No Quarter strike) landed on it
    /// after the critical: it can be hit (spells stop waiting for it).
    pub landed: bool,
}

/// 2.3: the kind of a critical throw, for its label and sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CriticalKind {
    Backstab,
    /// After you broke its stance (detected, or worn down by your blows).
    StanceCritical,
    /// After a parry.
    Riposte,
    /// From the side, or nothing told which.
    Critical,
}


/// The player's own combat state this tick (0.6 reflexes).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerState {
    pub hp: i32,
    pub max_hp: i32,
    pub stamina: i32,
    pub dodging: bool,
    /// Dodge invulnerability (event bit or PvE i-frames, ours included).
    pub iframes: bool,
    pub guarding: bool,
    /// AtkParam id of the last attack the player received.
    pub received: u32,
    /// Raw throw node state: 3/5 = the player performs a critical throw.
    pub throw_state: u32,
    /// 3.2: TAE "invincible during throw (attacker)": set through every
    /// critical animation, including stance-break criticals on big bosses
    /// whose throw node state is not 3 or 5.
    pub throw_attacker: bool,
    /// Current animation id (section prefix included).
    pub anim: i32,
}
impl PlayerState {
    /// 3.2: the player is performing a critical (riposte, backstab, stance
    /// break critical) right now.
    pub fn in_critical(&self) -> bool {
        matches!(self.throw_state, 3 | 5) || self.throw_attacker
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorldScan {
    pub ready: bool,
    pub player: Option<FieldInsHandle>,
    pub player_position: [f32; 3],
    /// Unit vector of the player's facing on the ground plane.
    pub player_forward: [f32; 3],
    /// Alive, visible characters within range, nearest first.
    pub foes: Vec<Foe>,
    /// Player hits this tick (proc echoes excluded).
    pub hits: u16,
    pub echo_hits: u16,
    pub last_hit_target: Option<FieldInsHandle>,
    pub kills: Vec<[f32; 3]>,
    /// Handles of the characters behind `kills`, same order.
    pub killed: Vec<FieldInsHandle>,
    /// Hits within the 6 s streak window and kills within the 8 s kill window.
    pub streak: u16,
    /// Seconds since each confirmed hit of the last 15 s, newest last.
    pub hit_ages: Vec<f32>,
    pub recent_kills: u16,
    pub seconds_since_hit: Option<f32>,
    pub seconds_since_kill: Option<f32>,
    /// Seconds since the player hit or was hit by anything engaged.
    pub seconds_since_combat: Option<f32>,
    /// Characters read this tick, and whether from the game's actor list.
    pub characters_read: u32,
    pub from_actor_list: bool,
    /// The first HP drops of a session, explained (for the log).
    pub diagnostics: Vec<String>,
    /// FP the player spent since the last tick (0.5 Blood Price); zero when
    /// maximum FP changed at the same time.
    pub fp_spent: i32,
    /// 0.6: enemies attacking the player at close range, nearest first.
    pub threats: Vec<Threat>,
    /// 0.6: engaged enemies whose stance broke this tick.
    pub stance_breaks: Vec<EventTarget>,
    /// 0.6: statuses that filled up on engaged enemies this tick.
    pub status_procs: Vec<(StatusKind, EventTarget)>,
    /// 0.6: engaged enemies that died this tick without a credited blow
    /// (a summon or a status finished them); elites still count them.
    pub engaged_deaths: Vec<FieldInsHandle>,
    pub player_state: PlayerState,
    /// 0.7: possible parries this tick.
    pub parry_candidates: Vec<ParryCandidate>,
    /// 0.7.1: status gauges of engaged enemies (Plague Bearer).
    pub gauges: Vec<Gauge>,
    /// 0.8: HP the player took from enemies this tick by hits, and by card
    /// spells (proc echoes); lifesteal heals a share of it.
    pub damage_dealt: i32,
    pub spell_damage_dealt: i32,
    /// Filled by the reflex tracker after the scan.
    pub reflex: ReflexEvents,
    /// First stance/status observations of a session, explained (for the log).
    pub detail_notes: Vec<String>,
    /// 3.2: your usual ordinary hit in the last two minutes (median), 0 = none.
    pub typical_hit: i32,
}

impl WorldScan {
    /// Confirmed hits within the last `seconds`.
    pub fn hits_within(&self, seconds: f32) -> u16 {
        self.hit_ages
            .iter()
            .filter(|age| **age <= seconds)
            .count()
            .min(u16::MAX as usize) as u16
    }
}

struct Track {
    hp: i32,
    seen: Instant,
    engaged_until: Option<Instant>,
    stance_broken: bool,
    /// 2.2: last (current, max) of the super armor and toughness gauges.
    gauges: [Option<(f32, f32)>; 2],
    /// 2.2: your last confirmed hit on it, and its last stance break.
    player_hit: Option<Instant>,
    broke_at: Option<Instant>,
    /// 3.2: your last ordinary hits on it (no criticals, no spells).
    hit_losses: VecDeque<i32>,
    status: Option<StatusTrack>,
    /// 0.7: attack animation playing at the last tick (id, progress,
    /// seconds left).
    attack: Option<(i32, f32, f32)>,
    /// 0.7: last time the attack was parryable.
    parryable_at: Option<Instant>,
}

/// Status fill (0 = clear, 1 = full) of one enemy, from its resist module.
struct StatusTrack {
    /// Values count down from the maximum (measured) or up from zero.
    down: bool,
    fill: [f32; 7],
    max: [i32; 7],
    cool: [Option<Instant>; 7],
}

#[derive(Default)]
pub struct WorldTracker {
    tracks: HashMap<FieldInsHandle, Track>,
    echo: HashMap<FieldInsHandle, Instant>,
    hits: VecDeque<Instant>,
    kills: VecDeque<Instant>,
    last_hit: Option<Instant>,
    last_kill: Option<Instant>,
    last_combat: Option<Instant>,
    player: Option<FieldInsHandle>,
    player_hp: Option<i32>,
    diagnosed: u32,
    last_stamina: Option<i32>,
    last_fp: Option<i32>,
    last_max_fp: Option<i32>,
    stamina_spent: Option<Instant>,
    fp_spent: Option<Instant>,
    detail_noted: u32,
    /// 2.2: the player's last critical throw, and stance gauges logged.
    critical_until: Option<Instant>,
    gauges_noted: u32,
    /// 3.2: your last ordinary hits on anything (time, damage).
    recent_hits: VecDeque<(Instant, i32)>,
}

impl WorldTracker {
    /// Forget everything; called on character change or world unload.
    pub fn reset(&mut self) {
        let diagnosed = self.diagnosed;
        *self = Self::default();
        self.diagnosed = diagnosed;
    }

    /// The proc engine marks a target before its bullet lands. HP drops on it
    /// until then are proc damage ("echo"): they never trigger on-hit powers,
    /// so procs cannot feed themselves.
    pub fn mark_echo(&mut self, target: FieldInsHandle, until: Instant) {
        let entry = self.echo.entry(target).or_insert(until);
        if *entry < until {
            *entry = until;
        }
    }

    /// # Safety
    /// Main game task only, with a loaded local world. No reference escapes.
    pub unsafe fn tick(&mut self, now: Instant) -> WorldScan {
        let mut scan = WorldScan::default();
        let Ok(world) = (unsafe { WorldChrMan::instance() }) else {
            self.reset();
            return scan;
        };
        let Some(player) = world.main_player.as_ref() else {
            self.reset();
            return scan;
        };
        let me = player.chr_ins.field_ins_handle;
        if self.player != Some(me) {
            self.reset();
            self.player = Some(me);
        }
        let origin = player.chr_ins.modules.physics.position;
        let origin = [origin.0, origin.1, origin.2];
        let q = &player.chr_ins.modules.physics.orientation;
        scan.player_forward = forward_from_quaternion([q.0, q.1, q.2, q.3]);
        scan.player = Some(me);
        scan.player_position = origin;

        // Who hurt the player: that attacker counts as engaged.
        let player_hp = player.chr_ins.modules.data.hp;
        let mut attacker = None;
        if self.player_hp.is_some_and(|hp| player_hp < hp) {
            let by = player.chr_ins.last_hit_by;
            if !by.is_empty() && by != me {
                attacker = Some(by);
                self.last_combat = Some(now);
            }
        }
        self.player_hp = Some(player_hp);

        // Enemies never record who hit them (`last_hit_by` stays empty on
        // them, 0.4.2 log), so a hit is credited from the attack that landed
        // (the AtkParam id each character keeps, player rows = AtkParam_Pc) or
        // from what the player was doing, unless an ally stood closer.
        let player_data = &player.chr_ins.modules.data;
        scan.player_state = unsafe { player_state(&player.chr_ins) };
        if self.last_stamina.is_some_and(|s| player_data.stamina < s - 3) {
            self.stamina_spent = Some(now);
        }
        if self.last_fp.is_some_and(|f| player_data.fp < f - 1) {
            self.fp_spent = Some(now);
        }
        let fp_spent = match (self.last_fp, self.last_max_fp) {
            (Some(fp), Some(max)) if max == player_data.max_fp && player_data.fp < fp => {
                fp - player_data.fp
            }
            _ => 0,
        };
        if scan.player_state.in_critical() {
            self.critical_until = Some(now + CRITICAL_CREDIT);
        }
        let throwing = scan.player_state.in_critical();
        let in_critical = self.critical_until.is_some_and(|until| now < until);
        self.last_stamina = Some(player_data.stamina);
        self.last_fp = Some(player_data.fp);
        self.last_max_fp = Some(player_data.max_fp);
        scan.fp_spent = fp_spent.clamp(0, 10_000);
        let swinging = self
            .stamina_spent
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(1500));
        let casting = self
            .fp_spent
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(3));
        let params = unsafe { SoloParamRepository::instance() }.ok();
        let mut memory = Readable::default();
        let (addresses, from_list) = unsafe { actor_addresses(world, &mut memory) };
        scan.from_actor_list = from_list;
        let characters: Vec<&ChrIns> = addresses
            .into_iter()
            .filter_map(|address| unsafe { validated(address, &mut memory) })
            .collect();
        scan.characters_read = characters.len() as u32;
        // Allies who could have dealt a blow. Torrent (and anything else
        // within 1.5 m of the player) moves with the player and never counts.
        let allies: Vec<[f32; 3]> = characters
            .iter()
            .filter(|c| {
                c.field_ins_handle != me && protected_team(c.team_type) && c.modules.data.hp > 0
            })
            .map(|c| {
                let p = c.modules.physics.position;
                [p.0, p.1, p.2]
            })
            .filter(|a| {
                ((a[0] - origin[0]).powi(2) + (a[1] - origin[1]).powi(2) + (a[2] - origin[2]).powi(2)).sqrt() > 1.5
            })
            .collect();
        let bosses: HashSet<FieldInsHandle> = unsafe { eldenring::cs::CSFeManImp::instance() }
            .map(|fe| {
                fe.boss_health_displays
                    .iter()
                    .filter(|d| !d.field_ins_handle.is_empty())
                    .map(|d| d.field_ins_handle)
                    .collect()
            })
            .unwrap_or_default();
        let mut visited = HashSet::new();
        let mut visit = |chr: &ChrIns, tracker: &mut Self, scan: &mut WorldScan| {
            let handle = chr.field_ins_handle;
            if handle == me || handle.is_empty() || !visited.insert(handle) {
                return;
            }
            let p = chr.modules.physics.position;
            let position = [p.0, p.1, p.2];
            let d = [
                position[0] - origin[0],
                position[1] - origin[1],
                position[2] - origin[2],
            ];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if !distance.is_finite() || distance > SCAN_RADIUS {
                return;
            }
            let data = &chr.modules.data;
            let (hp, max_hp) = (data.hp, data.max_hp);
            if max_hp <= 0 {
                return;
            }
            let source = chr.last_hit_by;
            let attack = chr.modules.action_flag.received_damage_type;
            let player_attack = attack > 0
                && params.is_some_and(|repo| repo.get::<AtkParam_Pc>(attack).is_some());
            let ally_closer = allies.iter().any(|a| {
                let d = ((a[0] - position[0]).powi(2) + (a[1] - position[1]).powi(2) + (a[2] - position[2]).powi(2)).sqrt();
                d < 4.0 && d + 0.75 < distance
            });
            // The attack row is the strongest evidence (summons use NPC rows),
            // so it wins over an ally standing closer.
            let (by_player, why) = if source == me {
                (true, "last hit by the player")
            } else if in_critical && distance <= 6.0 {
                (true, "the player's critical")
            } else if player_attack && distance <= 40.0 {
                (true, "a player attack")
            } else if ally_closer {
                (false, "an ally stood closer")
            } else if source.selector.field_ins_type() == Some(FieldInsType::Bullet) && distance <= 40.0 {
                (true, "a projectile")
            } else if swinging && distance <= 15.0 {
                (true, "the player was swinging")
            } else if casting && distance <= 40.0 {
                (true, "the player was casting")
            } else {
                (false, "no player activity")
            };
            let track = tracker.tracks.entry(handle).or_insert(Track {
                hp,
                seen: now,
                engaged_until: None,
                stance_broken: false,
                gauges: [None, None],
                player_hit: None,
                broke_at: None,
                hit_losses: VecDeque::new(),
                status: None,
                attack: None,
                parryable_at: None,
            });
            let damaged = track.hp > hp;
            if track.hp > hp && tracker.diagnosed < 8 {
                tracker.diagnosed += 1;
                scan.diagnostics.push(format!(
                    "HP drop {} on c{:04} at {:.0} m: attack {} ({}), last hit by {:#018x}, swinging {}, casting {} -> {} ({})",
                    track.hp - hp,
                    chr.character_id,
                    distance,
                    attack,
                    if player_attack { "player row" } else { "not a player row" },
                    handle_bits(source),
                    swinging,
                    casting,
                    if by_player { "counted" } else { "not counted" },
                    why
                ));
            }
            if track.hp > hp && by_player {
                track.player_hit = Some(now);
                let loss = track.hp - hp;
                let echo = tracker.echo.get(&handle).is_some_and(|until| now < *until);
                if loss >= MIN_HIT_POINTS || loss as f32 >= MIN_HIT_SHARE * max_hp as f32 || hp <= 0 {
                    let dealt = loss.min(track.hp.max(0)).max(0);
                    if echo {
                        scan.spell_damage_dealt = scan.spell_damage_dealt.saturating_add(dealt);
                    } else {
                        scan.damage_dealt = scan.damage_dealt.saturating_add(dealt);
                    }
                    if echo {
                        scan.echo_hits = scan.echo_hits.saturating_add(1);
                    } else {

                        scan.hits = scan.hits.saturating_add(1);
                        scan.last_hit_target = Some(handle);
                        tracker.hits.push_back(now);
                        tracker.last_hit = Some(now);
                        // 3.2: an ordinary hit (not a critical) for the
                        // usual-hit estimate (No Quarter, a downed foe).
                        if !throwing && hp > 0 && dealt > 0 {
                            track.hit_losses.push_back(dealt);
                            if track.hit_losses.len() > 6 {
                                track.hit_losses.pop_front();
                            }
                            tracker.recent_hits.push_back((now, dealt));
                            if tracker.recent_hits.len() > 12 {
                                tracker.recent_hits.pop_front();
                            }
                        }
                    }
                }
                track.engaged_until = Some(now + ENGAGED_FOR);
                tracker.last_combat = Some(now);
                if track.hp > 0 && hp <= 0 {
                    scan.kills.push(position);
                    scan.killed.push(handle);
                    tracker.kills.push_back(now);
                    tracker.last_kill = Some(now);
                }
            } else if track.hp > 0
                && hp <= 0
                && track.engaged_until.is_some_and(|until| now < until)
            {
                scan.engaged_deaths.push(handle);
            }
            if attacker == Some(handle) {
                track.engaged_until = Some(now + ENGAGED_FOR);
            }
            track.hp = hp;
            track.seen = now;
            let engaged = track.engaged_until.is_some_and(|until| now < until);
            let hidden = !chr.chr_flags1c5.enable_render()
                || chr.debug_flags.character_disabled()
                || chr.debug_flags.force_unloaded();
            let address = chr as *const ChrIns as usize;
            let foe_team = !protected_team(chr.team_type);
            let facing = {
                let q = &chr.modules.physics.orientation;
                let forward = forward_from_quaternion([q.0, q.1, q.2, q.3]);
                let to_player = [origin[0] - position[0], 0.0, origin[2] - position[2]];
                let flat = (to_player[0] * to_player[0] + to_player[2] * to_player[2]).sqrt().max(1e-3);
                let dot = (forward[0] * to_player[0] + forward[2] * to_player[2]) / flat;
                if dot.is_finite() { dot.clamp(-1.0, 1.0) } else { 0.0 }
            };
            // 0.6: an attack aimed at the player (perfect dodge, block).
            let mut attack_now = None;
            let mut anim_now = 0;
            if hp > 0 && !hidden && foe_team && distance <= THREAT_RADIUS {
                if chr.modules.action_flag.action_modifiers_flags.parryable() {
                    track.parryable_at = Some(now);
                }
                if let Some((anim, play, length)) = unsafe { current_anim(address, &mut memory) } {
                    anim_now = anim;
                    let local = anim.rem_euclid(1_000_000);
                    let progress = if length > 0.05 { play / length } else { 0.0 };
                    let attacking = (3000..=3999).contains(&local);
                    // 0.7 parry: last tick's attack is gone early, it was
                    // parryable a moment ago, the enemy took no damage and
                    // stands close in front of the player.
                    // A short attack that simply ended is not a parry: the
                    // attack must have had a real part left (0.3 s).
                    if let Some((from, from_progress, left)) = track.attack {
                        if anim != from
                            && !attacking
                            && from_progress < 0.8
                            && left >= PARRY_MIN_LEFT
                            && !damaged
                            && distance <= PARRY_RADIUS
                            && track.parryable_at.is_some_and(|t| now.duration_since(t) <= PARRY_WINDOW)
                        {
                            scan.parry_candidates.push(ParryCandidate {
                                target: EventTarget { position, handle: Some(handle) },
                                character_id: chr.character_id,
                                from,
                                progress: from_progress,
                                to: anim,
                                distance,
                            });
                        }
                    }
                    if attacking {
                        attack_now = Some((anim, progress, (length - play).max(0.0)));
                    }
                    let reach = if bosses.contains(&handle) || max_hp >= 4000 {
                        7.5
                    } else if max_hp >= 1500 {
                        5.0
                    } else {
                        3.8
                    };
                    if (3000..=3999).contains(&local)
                        && (0.12..=0.9).contains(&progress)
                        && facing >= 0.35
                        && distance <= reach
                    {
                        scan.threats.push(Threat {
                            handle,
                            position,
                            distance,
                            anim,
                            character_id: chr.character_id,
                        });
                    }
                }
            }
            track.attack = attack_now;
            // 0.6: stance breaks and status procs of engaged enemies.
            if engaged && hp > 0 && foe_team && distance <= DETAIL_RADIUS {
                let (flag, gauges) = unsafe { stance_state(address, &mut memory) };
                let hit_recently = track
                    .player_hit
                    .is_some_and(|at| now.duration_since(at) <= STANCE_HIT_WINDOW);
                let mut why = None;
                if let Some(broken) = flag {
                    if broken && !track.stance_broken {
                        why = Some("break flag".to_owned());
                    }
                    track.stance_broken = broken;
                }
                for (slot, gauge) in gauges.iter().enumerate() {
                    let (Some((now_value, max)), Some((before, before_max))) = (*gauge, track.gauges[slot]) else {
                        continue;
                    };
                    if !hit_recently || max < MIN_STANCE || (max - before_max).abs() > 0.5 || before >= 0.95 * max {
                        continue;
                    }
                    let gauge_name = if slot == 0 { "super armor" } else { "toughness" };
                    if before > 0.0 && now_value <= 0.0 {
                        why = why.or(Some(format!("{gauge_name} {before:.0} -> {now_value:.0} of {max:.0}")));
                    } else if before > 0.0 && before <= 0.6 * max && now_value >= 0.98 * max {
                        why = why.or(Some(format!("{gauge_name} {before:.0} -> full {max:.0} on a hit")));
                    }
                }
                if hit_recently && tracker.gauges_noted < 4 && gauges.iter().any(Option::is_some) {
                    tracker.gauges_noted += 1;
                    let show = |g: Option<(f32, f32)>| g.map_or("-".to_owned(), |(c, m)| format!("{c:.0}/{m:.0}"));
                    scan.detail_notes.push(format!(
                        "stance gauges on c{:04} after your hit: super armor {}, toughness {}",
                        chr.character_id,
                        show(gauges[0]),
                        show(gauges[1])
                    ));
                }
                track.gauges = gauges;
                if let Some(why) = why {
                    if track.broke_at.is_none_or(|at| now.duration_since(at) >= STANCE_COOLDOWN) {
                        track.broke_at = Some(now);
                        scan.stance_breaks.push(EventTarget { position, handle: Some(handle) });
                        if tracker.detail_noted < 6 {
                            tracker.detail_noted += 1;
                            scan.detail_notes.push(format!(
                                "stance broken on c{:04} at {:.0} m (max HP {max_hp}; {why})",
                                chr.character_id, distance
                            ));
                        }
                    }
                }
                if let Some(values) = unsafe { resist_values(address, &mut memory) } {
                    let procs = observe_status(&mut track.status, values, now);
                    if let Some(state) = &track.status {
                        scan.gauges.push(Gauge { handle, max: state.max });
                    }
                    for kind in procs {
                        scan.status_procs.push((kind, EventTarget { position, handle: Some(handle) }));
                        if tracker.detail_noted < 12 {
                            tracker.detail_noted += 1;
                            scan.detail_notes.push(format!(
                                "{} filled up on c{:04} at {:.0} m (resist values {:?})",
                                kind.name(),
                                chr.character_id,
                                distance,
                                values
                            ));
                        }
                    }
                }
            } else if !engaged {
                track.status = None;
            }
            if hp > 0 && !hidden {
                scan.foes.push(Foe {
                    handle,
                    position,
                    hp,
                    max_hp,
                    distance,
                    engaged,
                    protected: protected_team(chr.team_type),
                    facing,
                    boss: bosses.contains(&handle),
                    hit_age: track.player_hit.map(|at| now.duration_since(at).as_secs_f32()),
                    parryable_age: track.parryable_at.map(|at| now.duration_since(at).as_secs_f32()),
                    anim: anim_now,
                    invulnerable: {
                        let f = chr.modules.action_flag.action_modifiers_flags;
                        f.perfect_invincibility()
                            || f.invincible_excluding_throw_attacks_defender()
                            || f.pve_only_iframes()
                            || chr.modules.event.flags & 2 != 0
                            || chr.chr_flags1c5.is_invincible()
                    },
                    typical_hit: median(track.hit_losses.iter().copied()),
                });


            }
        };
        for chr in characters {
            visit(chr, self, &mut scan);
        }

        self.tracks
            .retain(|_, track| now.duration_since(track.seen) < Duration::from_secs(5));
        self.echo.retain(|_, until| now < *until);
        while self
            .hits
            .front()
            .is_some_and(|t| now.duration_since(*t) > HIT_MEMORY)
        {
            self.hits.pop_front();
        }
        while self
            .kills
            .front()
            .is_some_and(|t| now.duration_since(*t) > KILL_WINDOW)
        {
            self.kills.pop_front();
        }
        scan.foes
            .sort_by(|a, b| a.distance.total_cmp(&b.distance));
        scan.threats
            .sort_by(|a, b| a.distance.total_cmp(&b.distance));
        scan.hit_ages = self
            .hits
            .iter()
            .map(|t| now.duration_since(*t).as_secs_f32())
            .collect();
        scan.streak = scan.hits_within(STREAK_WINDOW.as_secs_f32());
        scan.recent_kills = self.kills.len().min(u16::MAX as usize) as u16;
        scan.seconds_since_hit = self.last_hit.map(|t| now.duration_since(t).as_secs_f32());
        scan.seconds_since_kill = self.last_kill.map(|t| now.duration_since(t).as_secs_f32());
        scan.seconds_since_combat = self.last_combat.map(|t| now.duration_since(t).as_secs_f32());
        self.recent_hits.retain(|(at, _)| now.duration_since(*at) <= Duration::from_secs(120));
        scan.typical_hit = median(self.recent_hits.iter().map(|(_, d)| *d));
        scan.ready = true;
        scan
    }
}

/// Readable-memory cache for one tick (VirtualQuery is slow under Wine, so
/// confirmed regions are remembered and queries are capped).
#[derive(Default)]
struct Readable {
    ranges: Vec<(usize, usize)>,
    queries: u32,
}
impl Readable {
    fn check(&mut self, p: usize, len: usize) -> bool {
        use windows::Win32::System::Memory::{
            MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
        };
        if p < 0x10000 || p % 8 != 0 {
            return false;
        }
        let Some(end) = p.checked_add(len) else {
            return false;
        };
        let mut at = p;
        while at < end {
            if let Some(&(_, b)) = self.ranges.iter().find(|&&(a, b)| at >= a && at < b) {
                at = b;
                continue;
            }
            if self.queries >= 2048 {
                return false;
            }
            self.queries += 1;
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let n = unsafe {
                VirtualQuery(
                    Some(at as *const std::ffi::c_void),
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if n == 0
                || info.State != MEM_COMMIT
                || info.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) != 0
                || info.Protect.0 == 0
            {
                return false;
            }
            let region_end = info.BaseAddress as usize + info.RegionSize;
            if region_end <= at {
                return false;
            }
            self.ranges.push((info.BaseAddress as usize, region_end));
            at = region_end;
        }
        true
    }
}

/// Candidate ChrIns addresses: the game's complete actor list (WorldChrMan
/// +0x1f1c0, rebuilt by WorldChrMan_Prepare; the CS2 conversion reads it on
/// 2.7.2.0), else every non-null entry of the character sets.
unsafe fn actor_addresses(world: &WorldChrMan, memory: &mut Readable) -> (Vec<usize>, bool) {
    const ACTOR_LIST: usize = 0x1f1c0;
    let base = world as *const WorldChrMan as usize;
    let mut out = Vec::new();
    if memory.check(base + ACTOR_LIST, 32) {
        let [_, first, last, end] = unsafe { std::ptr::read_unaligned((base + ACTOR_LIST) as *const [usize; 4]) };
        if first != 0
            && last >= first
            && end >= last
            && (last - first) % 8 == 0
            && (last - first) / 8 <= 16_384
            && memory.check(first, last - first)
        {
            let list = unsafe { std::slice::from_raw_parts(first as *const usize, (last - first) / 8) };
            out.extend(list.iter().copied().filter(|a| *a != 0));
        }
    }
    if !out.is_empty() {
        out.sort_unstable();
        out.dedup();
        return (out, true);
    }
    let stride = std::mem::size_of::<ChrSetEntry<ChrIns>>();
    let mut sets: Vec<&ChrSet<ChrIns>> = world.chr_sets.iter().flatten().map(|set| &**set).collect();
    sets.push(&world.open_field_chr_set.base);
    for set in sets {
        let capacity = set.capacity as usize;
        let entries = set.entries.as_ptr() as usize;
        if capacity == 0 || capacity > 8192 || !memory.check(entries, capacity * stride) {
            continue;
        }
        for index in 0..capacity {
            let address = unsafe { std::ptr::read_unaligned((entries + index * stride) as *const usize) };
            if address != 0 {
                out.push(address);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    (out, false)
}

/// A live, fully built character: readable, and its data and physics modules
/// point back to it. Nothing is dereferenced before these checks.
unsafe fn validated<'a>(address: usize, memory: &mut Readable) -> Option<&'a ChrIns> {
    if address % std::mem::align_of::<ChrIns>() != 0 || !memory.check(address, std::mem::size_of::<ChrIns>()) {
        return None;
    }
    let read = |at: usize| unsafe { std::ptr::read_unaligned(at as *const usize) };
    let modules = read(address + std::mem::offset_of!(ChrIns, modules));
    if !memory.check(modules, std::mem::size_of::<ChrInsModuleContainer>()) {
        return None;
    }
    let data = read(modules + std::mem::offset_of!(ChrInsModuleContainer, data));
    let physics = read(modules + std::mem::offset_of!(ChrInsModuleContainer, physics));
    let action = read(modules + std::mem::offset_of!(ChrInsModuleContainer, action_flag));
    if !memory.check(data, std::mem::size_of::<CSChrDataModule>())
        || !memory.check(physics, std::mem::size_of::<CSChrPhysicsModule>())
        || !memory.check(action, std::mem::size_of::<CSChrActionFlagModule>())
        || read(data + std::mem::offset_of!(CSChrDataModule, owner)) != address
        || read(physics + std::mem::offset_of!(CSChrPhysicsModule, owner)) != address
        || read(action + std::mem::offset_of!(CSChrActionFlagModule, owner)) != address
    {
        return None;
    }
    Some(unsafe { &*(address as *const ChrIns) })
}

fn handle_bits(handle: FieldInsHandle) -> u64 {
    unsafe { std::mem::transmute_copy::<FieldInsHandle, u64>(&handle) }
}

/// Model forward is local -Z (fromsoftware-rs debug-line convention), projected
/// to the ground plane. Falls back to +Z for a degenerate rotation.
fn forward_from_quaternion([x, y, z, w]: [f32; 4]) -> [f32; 3] {
    // Rotate (0, 0, -1) by the quaternion.
    let fx = -(2.0 * (x * z + w * y));
    let fz = -(1.0 - 2.0 * (x * x + y * y));
    let length = (fx * fx + fz * fz).sqrt();
    if !length.is_finite() || length < 1e-4 {
        return [0.0, 0.0, 1.0];
    }
    [fx / length, 0.0, fz / length]
}

/// 0.6: one status' fill changes; a proc is a full meter or a near-full meter
/// that resets within one tick (natural decay is a few percent a second).
/// Returns the statuses that filled up this tick.
fn observe_status(track: &mut Option<StatusTrack>, values: [i32; 14], now: Instant) -> Vec<StatusKind> {
    let mut current = [0i32; 7];
    let mut max = [0i32; 7];
    current.copy_from_slice(&values[..7]);
    max.copy_from_slice(&values[7..]);
    if max.iter().any(|m| !(1..=100_000).contains(m))
        || current.iter().zip(&max).any(|(c, m)| *c < -10 || *c > m + 10)
    {
        *track = None;
        return Vec::new();
    }
    let fill_of = |down: bool, i: usize| {
        let m = max[i].max(1) as f32;
        if down {
            (m - current[i] as f32) / m
        } else {
            current[i] as f32 / m
        }
        .clamp(0.0, 1.0)
    };
    let Some(state) = track.as_mut() else {
        // The first reading only sets the baseline. At rest every value
        // equals its maximum (counts down) or is zero (counts up); a mixed
        // first reading keeps the measured direction.
        let down = !current.iter().all(|c| *c == 0);
        *track = Some(StatusTrack {
            down,
            fill: std::array::from_fn(|i| fill_of(down, i)),
            max,
            cool: [None; 7],
        });
        return Vec::new();
    };
    let mut procs = Vec::new();
    for i in 0..7 {
        let fill = fill_of(state.down, i);
        let before = state.fill[i];
        let reset = before >= 0.6 && fill <= 0.1;
        let full = fill >= 0.999 && before < 0.999;
        let raised = max[i] > state.max[i] + state.max[i] / 20 && before >= 0.4;
        let cooled = state.cool[i].is_none_or(|t| now.duration_since(t) >= STATUS_COOLDOWN);
        if (reset || full || raised) && cooled {
            state.cool[i] = Some(now);
            procs.push(STATUS_KINDS[i]);
        }
        state.fill[i] = fill;
        state.max[i] = max[i];
    }
    procs
}

/// A module of a validated character: readable for `size` bytes and owned by
/// it (every ChrIns module keeps its owner right after the vtable).
unsafe fn module_at(chr: usize, slot: usize, size: usize, memory: &mut Readable) -> Option<usize> {
    let read = |at: usize| unsafe { std::ptr::read_unaligned(at as *const usize) };
    let modules = read(chr + std::mem::offset_of!(ChrIns, modules));
    if !memory.check(modules + slot, 8) {
        return None;
    }
    let module = read(modules + slot);
    if !memory.check(module, size.max(16)) || read(module + 8) != chr {
        return None;
    }
    Some(module)
}

/// (animation id, play time, length) of the animation playing now.
unsafe fn current_anim(chr: usize, memory: &mut Readable) -> Option<(i32, f32, f32)> {
    let module = unsafe {
        module_at(
            chr,
            std::mem::offset_of!(ChrInsModuleContainer, time_act),
            std::mem::size_of::<CSChrTimeActModule>(),
            memory,
        )
    }?;
    let index = unsafe {
        std::ptr::read_unaligned((module + std::mem::offset_of!(CSChrTimeActModule, read_idx)) as *const u32)
    } as usize
        % 10;
    let entry = module
        + std::mem::offset_of!(CSChrTimeActModule, anim_queue)
        + index * std::mem::size_of::<CSChrTimeActModuleAnim>();
    let anim = unsafe { std::ptr::read_unaligned((entry + std::mem::offset_of!(CSChrTimeActModuleAnim, anim_id)) as *const i32) };
    let play = unsafe { std::ptr::read_unaligned((entry + std::mem::offset_of!(CSChrTimeActModuleAnim, play_time)) as *const f32) };
    let length = unsafe { std::ptr::read_unaligned((entry + std::mem::offset_of!(CSChrTimeActModuleAnim, anim_length)) as *const f32) };
    (anim > 0 && play.is_finite() && length.is_finite()).then_some((anim, play, length))
}

/// 2.2: the poise-break flag and both stance gauges, (current, max).
unsafe fn stance_state(chr: usize, memory: &mut Readable) -> (Option<bool>, [Option<(f32, f32)>; 2]) {
    let gauge = |current: f32, max: f32| {
        (current.is_finite() && max.is_finite() && max > 0.0 && max < 1.0e5 && current > -1.0e4 && current <= max * 1.5)
            .then_some((current, max))
    };
    let mut flag = None;
    let mut gauges = [None, None];
    if let Some(module) = unsafe {
        module_at(
            chr,
            std::mem::offset_of!(ChrInsModuleContainer, super_armor),
            std::mem::size_of::<CSChrSuperArmorModule>(),
            memory,
        )
    } {
        let read = |offset: usize| unsafe { std::ptr::read_unaligned((module + offset) as *const f32) };
        let raw = unsafe {
            std::ptr::read_unaligned((module + std::mem::offset_of!(CSChrSuperArmorModule, poise_broken_state)) as *const u8)
        };
        flag = (raw <= 1).then_some(raw == 1);
        gauges[0] = gauge(
            read(std::mem::offset_of!(CSChrSuperArmorModule, sa_durability)),
            read(std::mem::offset_of!(CSChrSuperArmorModule, sa_durability_max)),
        );
    }
    if let Some(module) = unsafe {
        module_at(
            chr,
            std::mem::offset_of!(ChrInsModuleContainer, toughness),
            std::mem::size_of::<CSChrToughnessModule>(),
            memory,
        )
    } {
        let read = |offset: usize| unsafe { std::ptr::read_unaligned((module + offset) as *const f32) };
        gauges[1] = gauge(
            read(std::mem::offset_of!(CSChrToughnessModule, toughness)),
            read(std::mem::offset_of!(CSChrToughnessModule, toughness_max)),
        );
    }
    (flag, gauges)
}

/// The resist module sits right after time_act in the module container.
const RESIST_SLOT: usize = std::mem::offset_of!(ChrInsModuleContainer, time_act) + 8;
const _: () = assert!(std::mem::offset_of!(ChrInsModuleContainer, behavior) == RESIST_SLOT + 8);

unsafe fn resist_values(chr: usize, memory: &mut Readable) -> Option<[i32; 14]> {
    let module = unsafe { module_at(chr, RESIST_SLOT, RESIST_SIZE, memory) }?;
    Some(unsafe { std::ptr::read_unaligned((module + RESIST_VALUES) as *const [i32; 14]) })
}

/// The main player's own combat state; every pointer is checked first.
unsafe fn player_state(player: &ChrIns) -> PlayerState {
    let mut memory = Readable::default();
    let address = player as *const ChrIns as usize;
    let flags = player.modules.action_flag.action_modifiers_flags;
    let data = &player.modules.data;
    let mut state = PlayerState {
        hp: data.hp,
        max_hp: data.max_hp,
        stamina: data.stamina,
        dodging: flags.dodging(),
        iframes: player.modules.event.flags & 2 != 0 || flags.pve_only_iframes(),
        guarding: flags.guard_type_set(),
        received: player.modules.action_flag.received_damage_type,
        throw_state: 0,
        throw_attacker: flags.invincible_during_throw_attacker(),
        anim: 0,
    };
    if let Some((anim, _, _)) = unsafe { current_anim(address, &mut memory) } {
        state.anim = anim;
    }
    if let Some(module) = unsafe {
        module_at(
            address,
            std::mem::offset_of!(ChrInsModuleContainer, throw),
            std::mem::size_of::<CSChrThrowModule>(),
            &mut memory,
        )
    } {
        let node = unsafe {
            std::ptr::read_unaligned((module + std::mem::offset_of!(CSChrThrowModule, throw_node)) as *const usize)
        };
        let at = node + std::mem::offset_of!(CSThrowNode, throw_state);
        if node != 0 && memory.check(at, 4) {
            state.throw_state = unsafe { std::ptr::read_unaligned(at as *const u32) };
        }
    }
    state
}

/// 3.2: the median of some damage values (0 when there are none).
fn median(values: impl Iterator<Item = i32>) -> i32 {
    let mut all: Vec<i32> = values.filter(|v| *v > 0).collect();
    if all.is_empty() {
        return 0;
    }
    all.sort_unstable();
    all[all.len() / 2]
}

