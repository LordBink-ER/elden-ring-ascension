//! Ascended Elites (0.4). A regular enemy met for the first time may be
//! empowered: owned SpEffect rows (one per affix plus a base row that raises
//! its rune drop) are applied to the actor, the same way the boss scaler
//! scales bosses. Never elites: bosses (boss health bar, NpcParam getSoul 0 or
//! boss-style runes), friendly teams and critters. Killing an elite gives
//! Ascension Embers. Native game task only; plain values between ticks.
use crate::{config::EliteSettings, world::WorldScan};
use eldenring::cs::{
    CSFeManImp, ChrInsExt, FieldInsHandle, NpcParam, SoloParamRepository, SpEffectParam,
    WorldChrMan,
};
use fromsoftware_shared::FromStatic;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

pub const ELITE_BASE_ROW: u32 = 95_050_007;
const AFFIX_ROWS: [u32; 5] = [95_050_002, 95_050_003, 95_050_004, 95_050_005, 95_050_006];
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;
/// Rats, birds and other critters stay ordinary.
const MIN_MAX_HP: i32 = 150;
const ROLL_RADIUS: f32 = 40.0;
const FORGET_AFTER: Duration = Duration::from_secs(45);
const RETRY_EVERY: Duration = Duration::from_secs(2);
const MAX_APPLIES: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Affix {
    Hulking,
    Savage,
    Warded,
    Regenerating,
    Unstoppable,
}
const AFFIXES: [Affix; 5] = [
    Affix::Hulking,
    Affix::Savage,
    Affix::Warded,
    Affix::Regenerating,
    Affix::Unstoppable,
];
impl Affix {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hulking => "Hulking",
            Self::Savage => "Savage",
            Self::Warded => "Warded",
            Self::Regenerating => "Regenerating",
            Self::Unstoppable => "Unstoppable",
        }
    }
    fn row(self) -> u32 {
        AFFIX_ROWS[self as usize]
    }
}

struct Decision {
    affixes: Vec<Affix>,
    verified: bool,
    applies: u8,
    last_apply: Option<Instant>,
    seen: Instant,
    dead: bool,
}

#[derive(Debug, Clone, Default)]
pub struct EliteSummary {
    /// Embers earned this tick (elite kills).
    pub embers: u32,
    pub killed: u32,
    pub spawned: u32,
    /// Living elites in range: (affix names, distance), nearest first.
    pub nearby: Vec<(String, f32)>,
}

pub struct Elites {
    settings: EliteSettings,
    decided: HashMap<FieldInsHandle, Decision>,
    installed: bool,
    rng: u64,
    pub total_spawned: u32,
    pub total_killed: u32,
}

impl Elites {
    pub fn new(settings: EliteSettings) -> Self {
        let rng = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5eed)
            | 1;
        Self {
            settings,
            decided: HashMap::new(),
            installed: false,
            rng,
            total_spawned: 0,
            total_killed: 0,
        }
    }
    /// A loading screen invalidated every handle; actors own their effects.
    pub fn forget_world(&mut self) {
        self.decided.clear();
        self.installed = false;
    }
    fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Write the affix rows from the neutral template (no vanilla row edited).
    unsafe fn install(&mut self) -> Result<(), String> {
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("elite repository unavailable: {e}"))?;
        let neutral = repo
            .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
            .ok_or("neutral template missing")?
            .clone();
        let mut rows = vec![ELITE_BASE_ROW];
        rows.extend(AFFIX_ROWS);
        for id in rows {
            let row = repo
                .get_mut::<SpEffectParam>(id)
                .ok_or_else(|| format!("elite row {id} missing; regenerate the package"))?;
            *row = neutral.clone();
            row.set_effect_endurance(-1.0);
            row.set_sp_category(0);
            row.set_save_category(-1);
            row.set_state_info(0);
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_player(false);
            row.set_b_curr_hp_independe_max_hp(false);
            if id == ELITE_BASE_ROW {
                // NG+ style "souls going out of this character" multiplier.
                row.set_have_soul_rate(self.settings.rune_multiplier);
                continue;
            }
            match AFFIXES[AFFIX_ROWS.iter().position(|r| *r == id).unwrap_or(0)] {
                Affix::Hulking => {
                    row.set_max_hp_rate(1.9);
                    row.set_toughness_damage_cut_rate(0.7);
                    row.set_vfx_id(8710);
                }
                Affix::Savage => {
                    for set in [
                        SP_EFFECT_SET_PHYSICS,
                        SP_EFFECT_SET_MAGIC,
                        SP_EFFECT_SET_FIRE,
                        SP_EFFECT_SET_THUNDER,
                        SP_EFFECT_SET_DARK,
                    ] {
                        set(row, 1.3);
                    }
                    row.set_vfx_id(8600);
                }
                Affix::Warded => {
                    row.set_slash_damage_cut_rate(0.65);
                    row.set_blow_damage_cut_rate(0.65);
                    row.set_thrust_damage_cut_rate(0.65);
                    row.set_neutral_damage_cut_rate(0.65);
                    row.set_magic_damage_cut_rate(0.65);
                    row.set_fire_damage_cut_rate(0.65);
                    row.set_thunder_damage_cut_rate(0.65);
                    row.set_dark_damage_cut_rate(0.65);
                    row.set_vfx_id(8620);
                }
                Affix::Regenerating => {
                    // Negative = healing, % of max HP per motion interval.
                    row.set_change_hp_rate(-1.5);
                    row.set_motion_interval(1.0);
                    row.set_vfx_id(1_643_100);
                }
                Affix::Unstoppable => {
                    row.set_toughness_damage_cut_rate(0.3);
                    row.set_vfx_id(8570);
                }
            }
        }
        self.installed = true;
        Ok(())
    }

    /// # Safety
    /// Native game task, verified offline local host, `scan` from this tick.
    pub unsafe fn tick(
        &mut self,
        scan: &WorldScan,
        ng_plus: u32,
        lure: f64,
        ember_bonus: u32,
        now: Instant,
    ) -> Result<EliteSummary, String> {
        let mut summary = EliteSummary::default();
        if !self.settings.enabled || !scan.ready {
            return Ok(summary);
        }
        if !self.installed {
            unsafe { self.install() }?;
        }
        // Elite kills by the player: embers. An engaged elite that dies to a
        // summon, a status or a proc still counts (0.5 log: 9 met, 0 slain).
        for handle in scan.killed.iter().chain(&scan.engaged_deaths) {
            if let Some(decision) = self.decided.get_mut(handle) {
                if !decision.affixes.is_empty() && decision.verified && !decision.dead {
                    decision.dead = true;
                    summary.killed += 1;
                    summary.embers += 1 + decision.affixes.len() as u32 + ember_bonus;
                }
            }
        }
        self.total_killed += summary.killed;
        let bosses: HashSet<FieldInsHandle> = unsafe { CSFeManImp::instance() }
            .map(|fe| {
                fe.boss_health_displays
                    .iter()
                    .filter(|d| !d.field_ins_handle.is_empty())
                    .map(|d| d.field_ins_handle)
                    .collect()
            })
            .unwrap_or_default();
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("elite world unavailable: {e}"))?;
        let repo = unsafe { SoloParamRepository::instance() }
            .map_err(|e| format!("elite repository unavailable: {e}"))?;
        let mut alive = self
            .decided
            .values()
            .filter(|d| !d.affixes.is_empty() && !d.dead && now.duration_since(d.seen) < FORGET_AFTER)
            .count();
        for foe in &scan.foes {
            if foe.protected || bosses.contains(&foe.handle) {
                continue;
            }
            // A dead elite seen alive again respawned (rest, death): new roll.
            if self.decided.get(&foe.handle).is_some_and(|d| d.dead) {
                self.decided.remove(&foe.handle);
            }
            if !self.decided.contains_key(&foe.handle) {
                if foe.distance > ROLL_RADIUS || foe.max_hp < MIN_MAX_HP {
                    continue;
                }
                let Some(actor) = world.chr_ins_by_handle(&foe.handle) else {
                    continue;
                };
                // Boss-like reward: never an elite. Unknown NpcParam: never.
                let eligible = u32::try_from(actor.npc_param_id)
                    .ok()
                    .and_then(|id| repo.get::<NpcParam>(id))
                    .is_some_and(|npc| npc.get_soul() > 0 && !npc.is_soul_get_by_boss());
                let chance = (self.settings.chance * (1.0 + lure.max(0.0))).clamp(0.0, 0.5);
                let roll = self.uniform();
                let mut affixes = Vec::new();
                if eligible && alive < usize::from(self.settings.max_nearby) && roll < chance {
                    let double = self.settings.double_affix_chance + 0.15 * f64::from(ng_plus.min(3));
                    let count = if self.uniform() < double { 2 } else { 1 };
                    while affixes.len() < count {
                        let pick = AFFIXES[(self.uniform() * AFFIXES.len() as f64) as usize % AFFIXES.len()];
                        if !affixes.contains(&pick) {
                            affixes.push(pick);
                        }
                    }
                    affixes.sort();
                    alive += 1;
                    summary.spawned += 1;
                }
                self.decided.insert(
                    foe.handle,
                    Decision {
                        affixes,
                        verified: false,
                        applies: 0,
                        last_apply: None,
                        seen: now,
                        dead: false,
                    },
                );
            }
            let Some(decision) = self.decided.get_mut(&foe.handle) else {
                continue;
            };
            decision.seen = now;
            if decision.affixes.is_empty() {
                continue;
            }
            let Some(actor) = world.chr_ins_by_handle_mut(&foe.handle) else {
                continue;
            };
            let present = actor
                .special_effect
                .entries()
                .any(|effect| effect.param_id == ELITE_BASE_ROW as i32);
            if present {
                decision.verified = true;
            } else if decision.applies < MAX_APPLIES
                && decision.last_apply.is_none_or(|t| now.duration_since(t) >= RETRY_EVERY)
            {
                actor.apply_speffect(ELITE_BASE_ROW as i32, true);
                for affix in &decision.affixes {
                    actor.apply_speffect(affix.row() as i32, true);
                }
                decision.applies += 1;
                decision.last_apply = Some(now);
            }
            if decision.verified {
                let label: Vec<&str> = decision.affixes.iter().map(|a| a.name()).collect();
                summary.nearby.push((label.join(" + "), foe.distance));
            }
        }
        self.total_spawned += summary.spawned;
        self.decided
            .retain(|_, d| now.duration_since(d.seen) < FORGET_AFTER);
        summary.nearby.sort_by(|a, b| a.1.total_cmp(&b.1));
        summary.nearby.truncate(3);
        Ok(summary)
    }
}

type RateSetter = fn(&mut eldenring::param::SP_EFFECT_PARAM_ST, f32);
const SP_EFFECT_SET_PHYSICS: RateSetter = |row, v| row.set_physics_attack_rate(v);
const SP_EFFECT_SET_MAGIC: RateSetter = |row, v| row.set_magic_attack_rate(v);
const SP_EFFECT_SET_FIRE: RateSetter = |row, v| row.set_fire_attack_rate(v);
const SP_EFFECT_SET_THUNDER: RateSetter = |row, v| row.set_thunder_attack_rate(v);
const SP_EFFECT_SET_DARK: RateSetter = |row, v| row.set_dark_attack_rate(v);
