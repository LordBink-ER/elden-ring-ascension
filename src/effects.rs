//! Owned SpEffect backend. Call only on the gated offline game's native task.
//! Stores param *values*, never pointers or borrowed player/game references.
use crate::{
    affliction_buffs::AfflictionBuffs,
    arts::Arts,
    critical_buffs::CriticalBuffs,
    paths::Boons,
    intervention::{Intervention, InterventionSummary},
    kill_buffs::KillBuffs,
    momentum::Momentum,
    powers::{
        CombatSnapshot, EffectField, EffectTrigger, Modifier, PowerLibrary, aggregate_modifiers,
    },
    procs::{ActiveProc, ProcContext, ProcEngine, ProcStatus, ProcSummary},
    status::StatusImpart,
    world::{WorldScan, WorldTracker},
};
use eldenring::{
    cs::{ChrInsExt, FieldInsHandle, GameMan, SoloParamRepository, SpEffectParam, WorldChrMan},
    param::SP_EFFECT_PARAM_ST,
};
use fromsoftware_shared::FromStatic;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const GENERAL_ROW: u32 = 95_020_000;
pub const RUNES_ROW: u32 = 95_020_001;
pub const PERIODIC_ROW: u32 = 95_020_002;
pub const ON_KILL_PARENT_ROW: u32 = 95_020_003;
pub const ON_KILL_CHILD_ROW: u32 = 95_020_004;
pub const CAPACITY_ROW: u32 = 95_020_005;
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;
const OWNED_ROW_END: u32 = 95_020_015;

#[derive(Debug, Clone, Default, Serialize)]
pub struct EffectSummary {
    pub active_powers: usize,
    pub supported_powers: usize,
    pub applied_rows: Vec<u32>,
    pub roll_extension_seconds: f32,
    pub status_channels: u8,
    pub warnings: Vec<String>,
    pub intervention: InterventionSummary,
    /// Card status buildup in effect: (status, buildup, listed in weapon menus).
    pub status_buildup: Vec<(String, i32, bool)>,
    /// 0.8: stats + cards: attack speed and lifesteal (fractions), and the
    /// charged / jump / critical damage bonuses in effect.
    pub attack_speed: f32,
    pub lifesteal: f32,
    pub arts: [f32; 3],
    /// 0.9 Command and ally cards: allies' damage, HP, damage-taken reduction.
    pub allies: [f32; 3],
}

#[derive(Clone, PartialEq, Eq)]
struct OwnerKey {
    handle: FieldInsHandle,
    character_id: u32,
    birth_stamp: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowSignature {
    state_info: u16,
    instant: bool,
    child: i32,
    fields: Vec<(EffectField, u32)>,
}
impl RowSignature {
    fn new(state_info: u16, instant: bool, child: i32, fields: &[Modifier]) -> Self {
        Self {
            state_info,
            instant,
            child,
            fields: fields
                .iter()
                .map(|m| (m.field, m.value.to_bits()))
                .collect(),
        }
    }
}

#[derive(Default)]
pub struct NativeEffects {
    neutral: Option<SP_EFFECT_PARAM_ST>,
    owner: Option<OwnerKey>,
    signatures: BTreeMap<u32, RowSignature>,
    ready: bool,
    has_equip_load: bool,
    status: StatusImpart,
    status_warning: Option<String>,
    kill_buffs: KillBuffs,
    kill_warning: Option<String>,
    momentum: Momentum,
    momentum_warning: Option<String>,
    affliction: AfflictionBuffs,
    affliction_warning: Option<String>,
    critical: CriticalBuffs,
    critical_warning: Option<String>,
    procs: ProcEngine,
    proc_warning: Option<String>,
    intervention: Intervention,
    intervention_warning: Option<String>,
    world_ready: bool,
    elites_enabled: bool,
    /// 0.6: the per-frame item animation speed task is installed.
    item_speed_ready: bool,
    /// 2.0: what the Paths, the vanilla attributes and the world do this tick.
    boons: Boons,
    arts: Arts,
    synergies: Option<std::sync::Arc<crate::synergies::SynergyBook>>,
    /// HP/FP/max seen last tick, to refill after the capacity bonus returns.
    last_vitals: Option<Vitals>,
    /// Vitals right before the capacity row was (re)applied.
    refill: Option<(std::time::Instant, Vitals)>,
}

/// The game refills HP/FP (rest at a grace, respawn, load) to the maximum it
/// knows at that moment; when our capacity bonus is (re)applied a moment later
/// the maximum grows and the character walks out below full. A character who
/// was full before the bonus grew is topped up to the new maximum.
#[derive(Clone, Copy, Debug)]
struct Vitals {
    hp: i32,
    max_hp: i32,
    fp: i32,
    max_fp: i32,
}
impl Vitals {
    fn hp_full(&self) -> bool {
        self.max_hp > 0 && self.hp >= self.max_hp - 1
    }
    fn fp_full(&self) -> bool {
        self.max_fp > 0 && self.fp >= self.max_fp - 1
    }
}

impl NativeEffects {
    /// Load the proc catalog and generated rows once at startup.
    pub fn load_procs(&mut self, root: &std::path::Path) {
        self.procs = ProcEngine::load(root);
    }
    pub fn set_synergies(&mut self, book: std::sync::Arc<crate::synergies::SynergyBook>) {
        self.synergies = Some(book);
    }
    pub fn set_elites_enabled(&mut self, enabled: bool) {
        self.elites_enabled = enabled;
    }
    /// The hostile-world scanner produced a valid scan this tick.
    pub fn set_world_ready(&mut self, ready: bool) {
        self.world_ready = ready;
    }
    /// 2.0: this tick's boons (paths.rs).
    pub fn set_boons(&mut self, boons: Boons) {
        self.boons = boons;
    }
    pub fn set_item_speed_ready(&mut self, ready: bool) {
        self.item_speed_ready = ready;
    }
    pub fn procs(&self) -> &ProcEngine {
        &self.procs
    }
    pub fn intervention_saves(&self) -> u32 {
        self.intervention.saves
    }
    pub fn intervention_cooldown(&self, now: std::time::Instant) -> f32 {
        self.intervention.cooldown_left(now)
    }
    /// A loading screen or character change invalidated bullet emitters.
    pub fn forget_world(&mut self) {
        self.procs.forget_world();
    }
    /// Fire owned proc powers from this tick's world scan.
    ///
    /// # Safety
    /// Native game task only, after `reconcile` succeeded this tick.
    pub unsafe fn run_procs(
        &mut self,
        library: &PowerLibrary,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        scan: &WorldScan,
        tracker: &mut WorldTracker,
        context: &ProcContext,
        now: std::time::Instant,
        window: Option<(eldenring::cs::FieldInsHandle, bool)>,
    ) -> (ProcSummary, Vec<ProcStatus>) {
        let capabilities = self.capabilities();
        let boosts = self
            .synergies
            .as_ref()
            .map(|book| book.proc_boosts(levels, &snapshot.attributes, library))
            .unwrap_or_default();
        // Resonance path: Echo and Chorus fire extra projectiles;
        // Intelligence: card spells recharge faster.
        let stats = &self.boons;
        let owned: Vec<(&str, u8, crate::powers::ProcSpec)> = library
            .active_procs(levels, snapshot, &capabilities)
            .into_iter()
            .map(|(power, level, spec)| {
                let mut spec = spec.clone();
                spec.power *= boosts.get(&spec.proc).copied().unwrap_or(1.0);
                spec.count = (spec.count + stats.spell_extra).min(6);
                spec.trigger = spec.trigger.scaled((1.0 - stats.spell_cooldown).clamp(0.4, 1.0));
                (power, level, spec)
            })
            .collect();
        let active: Vec<ActiveProc> = owned
            .iter()
            .map(|(power, level, spec)| ActiveProc {
                power,
                level: *level,
                spec,
            })
            .collect();
        let summary = unsafe { self.procs.tick(&active, scan, tracker, context, now, window) };
        let status = self.procs.status(&active, now, scan);
        (summary, status)
    }
    /// Configure channel preference once from the validated profile. Only
    /// already-owned, currently positive status requests can be selected.
    pub fn set_status_priority(&mut self, priority: &[EffectField]) -> Result<(), String> {
        self.status.set_priority(priority)
    }
    /// These names describe implemented adapter inputs/backends, not empirical
    /// verification in the game. Outgoing status capabilities require their
    /// dedicated parent/child rows; no hit-event capability is advertised.
    pub fn capabilities(&self) -> BTreeSet<String> {
        let mut result: BTreeSet<String> = [
            "meta_draw",
            "sampled_resources",
            "boss_combat",
            "action_flags",
            "elapsed_time",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if self.ready {
            result.extend(
                ["sp_effect_params", "periodic_resources", "native_on_kill"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if self.has_equip_load {
            result.insert("equip_load_snapshot".into());
        }
        if self.ready && self.status.is_ready() {
            result.insert("weapon_status_impart".into());
            result.insert("multi_status_channels".into());
        }
        if self.ready && self.kill_buffs.is_ready() {
            result.insert("native_timed_on_kill".into());
        }
        if self.ready && self.momentum.is_ready() {
            result.insert("native_weighted_momentum".into());
        }
        if self.ready && self.affliction.is_ready() {
            result.insert("native_nearby_affliction".into());
        }
        if self.ready && self.critical.is_ready() {
            result.insert("native_critical_blessing".into());
        }
        if self.world_ready {
            result.insert("event.hit_landed".into());
            result.insert("event.kill".into());
            if self.elites_enabled {
                result.insert("event.elite_kill".into());
            }
            // 0.6 reflexes, read from the same scan (reflexes.rs).
            for event in [
                "event.perfect_dodge",
                "event.block",
                "event.guard_counter",
                "event.critical_hit",
                "event.poise_break",
                "event.status_proc",
                "event.parry",
                "event.backstab",
            ] {

                result.insert(event.into());
            }
        }
        if self.ready && self.item_speed_ready {
            result.insert("native_item_speed".into());
            // 0.8: the same per-frame task speeds up attack animations.
            result.insert("native_attack_speed".into());
        }
        if self.ready && self.world_ready {
            // 0.8: lifesteal reads the damage you deal from the world scan;
            // charged/jump/critical bonuses use owned talisman copies.
            result.insert("native_lifesteal".into());
            result.insert("native_arts".into());
            // 0.9: allies get owned rows from the same scan (allies.rs).
            result.insert("native_allies".into());
            result.insert("event.execution".into());
            result.insert("event.rampage".into());

        }
        if self.ready && self.world_ready && self.procs.is_ready() {
            result.insert("native_procs".into());
        }
        // 3.2 No Quarter (pursuit.rs): strikes on a downed critical victim.
        if self.ready && self.world_ready {
            result.insert("native_downed_strike".into());
        }
        if self.ready && self.intervention.is_ready() {
            result.insert("native_intervention".into());
        }
        // 0.5: Blood Price reads the player's FP spending from the world scan.
        if self.ready && self.world_ready {
            result.insert("native_blood_price".into());
        }
        result
    }

    /// Validate every reserved runtime row before enabling the backend.
    ///
    /// # Safety
    /// Must run on the game's native task, after executable and regulation
    /// compatibility checks. No caller may concurrently access the repository.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        self.ready = false;
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("SpEffect repository unavailable: {e}"))?;
        let neutral = repo
            .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
            .ok_or_else(|| {
                "Ascension's neutral SpEffect template is missing; load its generated package"
                    .to_owned()
            })?;
        if neutral.state_info() != 0
            || neutral.effect_endurance() != -1.0
            || neutral.change_hp_point() != 0
            || neutral.change_mp_point() != 0
            || neutral.change_hp_rate() != 0.0
            || neutral.change_mp_rate() != 0.0
            || neutral.apply_id_on_get_soul() != 0
            || neutral.soul_rate() != 1.0
            || neutral.icon_id() != -1
        {
            self.ready = false;
            return Err(
                "Ascension neutral template is not neutral; refusing native effects".into(),
            );
        }
        for id in GENERAL_ROW..=OWNED_ROW_END {
            if repo.get::<SpEffectParam>(id).is_none() {
                self.ready = false;
                return Err(format!("required owned SpEffect row {id} is missing"));
            }
        }
        if self.neutral.is_none() {
            self.neutral = Some(neutral.clone());
        }
        self.ready = true;
        self.status_warning = unsafe { self.status.ensure_ready() }.err();
        if self.status_warning.is_some() {
            let _ = unsafe { self.status.clear() };
        }
        self.kill_warning = unsafe { self.kill_buffs.ensure_ready() }.err();
        if self.kill_warning.is_some() {
            let _ = unsafe { self.kill_buffs.clear() };
        }
        self.momentum_warning = unsafe { self.momentum.ensure_ready() }.err();
        if self.momentum_warning.is_some() {
            let _ = unsafe { self.momentum.clear() };
        }
        self.affliction_warning = unsafe { self.affliction.ensure_ready() }.err();
        if self.affliction_warning.is_some() {
            let _ = unsafe { self.affliction.clear() };
        }
        self.critical_warning = unsafe { self.critical.ensure_ready() }.err();
        if self.critical_warning.is_some() {
            let _ = unsafe { self.critical.clear() };
        }
        self.proc_warning = unsafe { self.procs.ensure_ready() }.err();
        self.intervention_warning = unsafe { self.intervention.ensure_ready() }.err();
        Ok(())
    }

    /// Reconcile owned rows and effects from committed progression. The caller
    /// supplies a freshly sampled snapshot and never speculative chosen levels.
    ///
    /// # Safety
    /// Native game task only, offline, with the pinned executable/regulation.
    /// All game references acquired here expire before this function returns.
    pub unsafe fn reconcile(
        &mut self,
        library: &PowerLibrary,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        strength: f32,
    ) -> Result<EffectSummary, String> {
        if !snapshot.in_game || !snapshot.hp_ratio.is_finite() || snapshot.hp_ratio <= 0.0 {
            unsafe {
                self.clear()?;
            }
            return Ok(EffectSummary::default());
        }
        let game =
            unsafe { GameMan::instance() }.map_err(|e| format!("game state unavailable: {e}"))?;
        if !crate::game::actual_offline(game) {
            unsafe {
                self.clear()?;
            }
            return Err("Ascension effects require offline mode".into());
        }
        if let Err(error) = unsafe { self.ensure_ready() } {
            let _ = unsafe { self.clear() };
            return Err(error);
        }
        self.has_equip_load = snapshot
            .equip_load_ratio
            .is_some_and(|r| r.is_finite() && r >= 0.0);
        let capabilities = self.capabilities();
        let _ = strength;
        let evolution = self.boons.clone();
        let active = library.active_effects(levels, snapshot, &capabilities);
        let mut passive = evolution.modifiers.clone();
        if let Some(book) = &self.synergies {
            passive.extend(book.active_modifiers(levels, snapshot, library));
        }
        let mut on_kill = Vec::new();
        let mut timed_rune_multiplier = 1.0_f32;
        let mut timed_duration = 0.0_f32;
        let mut momentum_recovery = 0_i32;
        let mut momentum_cost = 1.0_f32;
        let mut momentum_duration = 0.0_f32;
        let mut affliction_magic = 1.0_f32;
        let mut affliction_cost = 1.0_f32;
        let mut affliction_duration = 0.0_f32;
        let mut critical_posture = 1.0_f32;
        for effect in &active {
            match effect.trigger {
                EffectTrigger::Passive => passive.extend_from_slice(&effect.modifiers),
                EffectTrigger::OnKill => on_kill.extend_from_slice(&effect.modifiers),
                EffectTrigger::OnKillTimed => {
                    for modifier in &effect.modifiers {
                        if modifier.field == EffectField::SoulRate {
                            timed_rune_multiplier =
                                (timed_rune_multiplier * modifier.value).clamp(1.0, 1.5);
                        }
                    }
                    timed_duration = timed_duration.max(effect.duration_seconds).clamp(0.0, 60.0);
                }
                EffectTrigger::OnMomentum => {
                    for modifier in &effect.modifiers {
                        match modifier.field {
                            EffectField::StaminaRecoverChangeSpeed => {
                                momentum_recovery = modifier.value.round().clamp(0.0, 35.0) as i32
                            }
                            EffectField::ConsumeStaminaRate => {
                                momentum_cost = modifier.value.clamp(0.45, 1.0)
                            }
                            _ => {}
                        }
                    }
                    momentum_duration = effect.duration_seconds.clamp(0.0, 60.0);
                }
                EffectTrigger::OnNearbyAffliction => {
                    for modifier in &effect.modifiers {
                        match modifier.field {
                            EffectField::MagicAttackRate => {
                                affliction_magic = modifier.value.clamp(1.0, 1.5)
                            }
                            EffectField::MagicConsumptionRate => {
                                affliction_cost = modifier.value.clamp(0.35, 1.0)
                            }
                            _ => {}
                        }
                    }
                    affliction_duration = effect.duration_seconds.clamp(0.0, 60.0);
                }
                EffectTrigger::OnCriticalBlessing => {
                    for modifier in &effect.modifiers {
                        if modifier.field == EffectField::SaAttackPowerRate {
                            critical_posture = modifier.value.clamp(1.0, 2.5);
                        }
                    }
                }
            }
        }
        let mut general = Vec::new();
        let mut runes = Vec::new();
        let mut periodic = Vec::new();
        let mut capacities = Vec::new();
        let mut status_requested = BTreeMap::new();
        for modifier in aggregate_modifiers(passive.iter()) {
            // Neutral multiplicative factors are one, including rune income.
            // A zero soulRate would suppress the game's ordinary rune rewards.
            if modifier.value == modifier.field.neutral() {
                continue;
            }
            use EffectField::*;
            match modifier.field {
                SoulRate => runes.push(modifier),
                ChangeHpRate | ChangeHpPoint | ChangeMpRate | ChangeMpPoint => {
                    periodic.push(modifier)
                }
                MotionInterval => {} // One stable 1-second clock for all periodic sources.
                MaxHpRate | MaxMpRate | MaxStaminaRate | EquipWeightChangeRate => {
                    capacities.push(modifier)
                }
                // Outgoing buildup belongs to the weapon-impart carriers,
                // never to a directly applied effect on the player.
                PoizonAttackPower | DiseaseAttackPower | BloodAttackPower | FreezeAttackPower
                | SleepAttackPower | MadnessAttackPower | CurseAttackPower => {
                    status_requested.insert(modifier.field, modifier.value);
                }
                _ => general.push(modifier),
            }
        }
        let on_kill = aggregate_on_kill(&on_kill);
        let mut desired = BTreeMap::<u32, (RowSignature, Vec<Modifier>)>::new();
        if !general.is_empty() {
            desired.insert(
                GENERAL_ROW,
                (RowSignature::new(0, false, 0, &general), general),
            );
        }
        if !runes.is_empty() {
            desired.insert(RUNES_ROW, (RowSignature::new(76, false, 0, &runes), runes));
        }
        if !periodic.is_empty() {
            periodic.push(Modifier {
                field: EffectField::MotionInterval,
                value: 1.0,
            });
            desired.insert(
                PERIODIC_ROW,
                (RowSignature::new(0, false, 0, &periodic), periodic),
            );
        }
        if !capacities.is_empty() {
            desired.insert(
                CAPACITY_ROW,
                (RowSignature::new(0, false, 0, &capacities), capacities),
            );
        }
        let has_on_kill = !on_kill.is_empty();
        if has_on_kill {
            desired.insert(
                ON_KILL_CHILD_ROW,
                (RowSignature::new(0, true, 0, &on_kill), on_kill),
            );
            desired.insert(
                ON_KILL_PARENT_ROW,
                (
                    RowSignature::new(199, false, ON_KILL_CHILD_ROW as i32, &[]),
                    Vec::new(),
                ),
            );
        }

        // Complete the separate status operation before acquiring any mutable
        // player/repository references for the aggregate operation below.
        let mut status_applied = Vec::new();
        let mut status_warnings = Vec::new();
        let mut status_buildup = Vec::new();
        if let Some(warning) = &self.affliction_warning {
            status_warnings.push(warning.clone());
        }
        if self.affliction.is_ready() {
            match unsafe {
                self.affliction
                    .reconcile(affliction_magic, affliction_cost, affliction_duration)
            } {
                Ok(summary) => {
                    status_applied.extend(summary.parent_rows);
                    if summary.buff_active {
                        if let Some(id) = summary.child_row {
                            status_applied.push(id);
                        }
                    }
                    status_warnings.extend(summary.warnings);
                }
                Err(error) => {
                    status_warnings.push(format!("nearby affliction unavailable: {error}"));
                    let _ = unsafe { self.affliction.clear() };
                }
            }
        }
        if let Some(warning) = &self.critical_warning {
            status_warnings.push(warning.clone());
        }
        if self.critical.is_ready() {
            match unsafe { self.critical.reconcile(critical_posture) } {
                Ok(summary) => {
                    if let Some(id) = summary.carrier_row {
                        status_applied.push(id);
                    }
                    if summary.posture_active {
                        if let Some(id) = summary.posture_row {
                            status_applied.push(id);
                        }
                    }
                    status_warnings.extend(summary.warnings);
                }
                Err(error) => {
                    status_warnings.push(format!("critical blessing unavailable: {error}"));
                    let _ = unsafe { self.critical.clear() };
                }
            }
        }
        if let Some(warning) = &self.momentum_warning {
            status_warnings.push(warning.clone());
        }
        if self.momentum.is_ready() {
            match unsafe {
                self.momentum
                    .reconcile(momentum_recovery, momentum_cost, momentum_duration)
            } {
                Ok(summary) => {
                    if let Some(id) = summary.parent_row {
                        status_applied.push(id);
                    }
                    if summary.buff_active {
                        if let Some(id) = summary.child_row {
                            status_applied.push(id);
                        }
                    }
                    status_warnings.extend(summary.warnings);
                }
                Err(error) => {
                    status_warnings.push(format!("weighted momentum unavailable: {error}"));
                    let _ = unsafe { self.momentum.clear() };
                }
            }
        }
        if let Some(warning) = &self.kill_warning {
            status_warnings.push(warning.clone());
        }
        if self.kill_buffs.is_ready() {
            match unsafe {
                self.kill_buffs
                    .reconcile(timed_rune_multiplier, timed_duration)
            } {
                Ok(summary) => {
                    if let Some(id) = summary.parent_row {
                        status_applied.push(id);
                    }
                    if summary.buff_active {
                        if let Some(id) = summary.child_row {
                            status_applied.push(id);
                        }
                    }
                    status_warnings.extend(summary.warnings);
                }
                Err(error) => {
                    status_warnings.push(format!("timed kill rewards unavailable: {error}"));
                    let _ = unsafe { self.kill_buffs.clear() };
                }
            }
        }
        if let Some(warning) = &self.status_warning {
            status_warnings.push(warning.clone());
        }
        if self.status.is_ready() {
            match unsafe {
                self.status.reconcile(
                    &{
                        // 2.0 Malice path 15, Death's Herald: Death Blight joins.
                        let mut requested = status_requested.clone();
                        if evolution.death_blight > 0.0 {
                            *requested.entry(EffectField::CurseAttackPower).or_insert(0.0) += evolution.death_blight;
                        }
                        requested
                    },
                    7,
                    evolution.status_flat,
                    evolution.status_potency,
                    0.0,
                )

            } {
                Ok(summary) => {
                    for (channel, buildup) in &summary.buildup {
                        let listed = summary.native.iter().any(|n| &n.channel == channel);
                        status_buildup.push((channel.clone(), *buildup, listed));
                    }
                    status_applied.extend(summary.applied_rows);
                    status_warnings.extend(summary.warnings);
                }
                Err(error) => {
                    status_warnings.push(format!("weapon status effects unavailable: {error}"));
                    let _ = unsafe { self.status.clear() };
                }
            }
        }

        if let Some(warning) = &self.proc_warning {
            status_warnings.push(warning.clone());
        }
        if let Some(warning) = &self.intervention_warning {
            status_warnings.push(warning.clone());
        }
        let intervention_spec = library
            .active_intervention(levels, snapshot, &capabilities)
            .into_iter()
            .chain(evolution.intervention)
            .reduce(crate::powers::InterventionSpec::combine);
        let intervention = match unsafe {
            self.intervention
                .reconcile(intervention_spec, std::time::Instant::now())
        } {
            Ok(summary) => summary,
            Err(error) => {
                status_warnings.push(format!("divine intervention unavailable: {error}"));
                InterventionSummary::default()
            }
        };
        // Charged / jump / critical damage, attack speed, lifesteal and ally
        // boosts: the attributes (Strength, Dexterity, Vigor, Mind), the
        // Paths (Blur, Sanguine Lord, Phalanx) and the owned cards whose
        // condition holds.
        let card_arts = library.arts(levels, snapshot, &capabilities);
        // 3.4 balance: ceilings that keep stacks strong but sane.
        let attack_speed = (evolution.attack_speed + card_arts[0]).clamp(0.0, 0.35);
        let lifesteal = ((evolution.lifesteal + card_arts[1]) * evolution.lifesteal_mult).clamp(0.0, 0.2);
        let arts_bonus = [
            evolution.charge_damage + card_arts[2],
            evolution.jump_damage + card_arts[3],
            evolution.critical_damage + card_arts[4],
        ];
        let allies = [
            (evolution.ally_damage + card_arts[5]).clamp(0.0, 1.5),
            (evolution.ally_hp + card_arts[6]).clamp(0.0, 1.5),
            (evolution.ally_defense + card_arts[7]).clamp(0.0, 0.5),
        ];
        if capabilities.contains("native_arts") {
            if let Err(error) = unsafe { self.arts.reconcile(arts_bonus) } {
                status_warnings.push(format!("charged/jump/critical bonuses unavailable: {error}"));
            }
        }


        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("player world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.forget();
            return Ok(EffectSummary::default());
        };
        if player.chr_ins.modules.data.hp <= 0 {
            for id in GENERAL_ROW..=OWNED_ROW_END {
                player.chr_ins.remove_speffect(id as i32);
            }
            self.forget();
            let _ = unsafe { self.status.clear() };
            let _ = unsafe { self.kill_buffs.clear() };
            let _ = unsafe { self.momentum.clear() };
            let _ = unsafe { self.affliction.clear() };
            let _ = unsafe { self.critical.clear() };
            return Ok(EffectSummary::default());
        }
        let player_data = unsafe { player.player_game_data.as_ref() };
        if !unsafe { crate::game::native_offline_host(game, player_data) } {
            for id in GENERAL_ROW..=OWNED_ROW_END {
                player.chr_ins.remove_speffect(id as i32);
            }
            self.forget();
            return Err("effect target is not the verified offline local host".into());
        }
        let owner = OwnerKey {
            handle: player.chr_ins.field_ins_handle,
            character_id: player_data.character_id,
            birth_stamp: player_data.packed_time_stamp,
        };
        if self.owner.as_ref() != Some(&owner) {
            // Only the freshly resolved player's own aggregate IDs are touched.
            // No old player pointer is retained or dereferenced after a load.
            for id in GENERAL_ROW..=OWNED_ROW_END {
                player.chr_ins.remove_speffect(id as i32);
            }
            self.signatures.clear();
            self.owner = Some(owner);
        }
        let live_ids: BTreeSet<u32> = player
            .chr_ins
            .special_effect
            .entries()
            .filter_map(|e| u32::try_from(e.param_id).ok())
            .collect();
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("SpEffect repository unavailable: {e}"))?;
        let neutral = self
            .neutral
            .as_ref()
            .ok_or_else(|| "neutral template disappeared".to_owned())?;
        let mut changed = BTreeSet::new();
        // Child first: the parent must never point at an old or uninitialized
        // payload. It is an instant effect and is never applied directly.
        let order = [
            ON_KILL_CHILD_ROW,
            GENERAL_ROW,
            RUNES_ROW,
            PERIODIC_ROW,
            CAPACITY_ROW,
            ON_KILL_PARENT_ROW,
        ];
        for id in order {
            let Some((signature, fields)) = desired.get(&id) else {
                continue;
            };
            let parent_missing = id == ON_KILL_CHILD_ROW && !live_ids.contains(&ON_KILL_PARENT_ROW);
            let must_write = self.signatures.get(&id) != Some(signature)
                || (id != ON_KILL_CHILD_ROW && !live_ids.contains(&id))
                || parent_missing;
            if !must_write {
                continue;
            }
            let row = repo
                .get_mut::<SpEffectParam>(id)
                .ok_or_else(|| format!("owned SpEffect {id} disappeared"))?;
            *row = neutral.clone();
            row.set_effect_endurance(if signature.instant { 0.0 } else { -1.0 });
            row.set_state_info(signature.state_info);
            row.set_apply_id_on_get_soul(signature.child);
            row.set_motion_interval(if id == PERIODIC_ROW || signature.instant {
                1.0
            } else {
                0.0
            });
            row.set_sp_category(0);
            row.set_save_category(-1);
            for modifier in fields {
                modifier.field.write(row, modifier.value);
            }
            self.signatures.insert(id, signature.clone());
            changed.insert(id);
        }
        for id in GENERAL_ROW..=OWNED_ROW_END {
            if id == ON_KILL_CHILD_ROW {
                continue;
            }
            if !desired.contains_key(&id) {
                if live_ids.contains(&id) {
                    player.chr_ins.remove_speffect(id as i32);
                }
                self.signatures.remove(&id);
            }
        }
        if !has_on_kill {
            self.signatures.remove(&ON_KILL_CHILD_ROW);
        }
        let mut applied_rows = Vec::new();
        let vitals = {
            let data = &player.chr_ins.modules.data;
            Vitals {
                hp: data.hp,
                max_hp: data.max_hp,
                fp: data.fp,
                max_fp: data.max_fp,
            }
        };
        for id in [
            GENERAL_ROW,
            RUNES_ROW,
            PERIODIC_ROW,
            CAPACITY_ROW,
            ON_KILL_PARENT_ROW,
        ] {
            if !desired.contains_key(&id) {
                continue;
            }
            if changed.contains(&id) || !live_ids.contains(&id) {
                if live_ids.contains(&id) {
                    player.chr_ins.remove_speffect(id as i32);
                }
                if id == CAPACITY_ROW {
                    self.refill = Some((std::time::Instant::now(), vitals));
                }
                player.apply_speffect(id as i32, true);
            }
            applied_rows.push(id);
        }
        // Top up after the bonus took effect (it shows from the next frame).
        if desired.contains_key(&CAPACITY_ROW) && vitals.hp > 0 {
            let data = &mut player.chr_ins.modules.data;
            let mut refilled = false;
            for before in self
                .refill
                .filter(|(at, _)| at.elapsed() < std::time::Duration::from_secs(3))
                .map(|(_, v)| v)
                .into_iter()
                .chain(self.last_vitals)
            {
                if before.hp_full() && data.max_hp > before.max_hp && data.hp < data.max_hp {
                    data.hp = data.max_hp;
                    refilled = true;
                }
                if before.fp_full() && data.max_fp > before.max_fp && data.fp < data.max_fp {
                    data.fp = data.max_fp;
                    refilled = true;
                }
            }
            if refilled {
                self.refill = None;
            }
        }
        {
            let data = &player.chr_ins.modules.data;
            self.last_vitals = Some(Vitals {
                hp: data.hp,
                max_hp: data.max_hp,
                fp: data.fp,
                max_fp: data.max_fp,
            });
        }
        applied_rows.extend(status_applied);
        let mut warnings = status_warnings;
        for id in levels.keys() {
            match library.get(id) {
                Some(power) if !power.required().is_subset(&capabilities)=>warnings.push(format!("{}: required backend unavailable; effect is preserved in progression but inactive",power.name)),
                None=>warnings.push(format!("unknown owned power {id}; progression retained")),
                _=>{},
            }
        }
        Ok(EffectSummary {
            active_powers: active.len(),
            supported_powers: library
                .powers
                .iter()
                .filter(|p| p.required().is_subset(&capabilities))
                .count(),
            applied_rows,
            roll_extension_seconds: evolution.roll_extension_seconds,
            status_channels: 7,
            warnings,
            intervention,
            status_buildup,
            attack_speed,
            lifesteal,
            arts: arts_bonus,
            allies,
        })
    }

    /// Remove only this backend's reserved IDs from the current local player.
    /// The caller must run this on its native callback before disabling work.
    ///
    /// # Safety
    /// Same task/compatibility requirements as reconcile. Do not call from Drop
    /// on an arbitrary worker/render thread.
    pub unsafe fn clear(&mut self) -> Result<(), String> {
        let status_error = unsafe { self.status.clear() }.err();
        let kill_error = unsafe { self.kill_buffs.clear() }.err();
        let momentum_error = unsafe { self.momentum.clear() }.err();
        let affliction_error = unsafe { self.affliction.clear() }.err();
        let critical_error = unsafe { self.critical.clear() }.err();
        unsafe { self.intervention.clear() };
        unsafe { self.arts.clear() };

        if let Ok(world) = unsafe { WorldChrMan::instance_mut() } {
            if let Some(player) = world.main_player.as_mut() {
                for id in GENERAL_ROW..=OWNED_ROW_END {
                    player.chr_ins.remove_speffect(id as i32);
                }
            }
        }
        self.forget();
        let errors: Vec<_> = [
            status_error,
            kill_error,
            momentum_error,
            affliction_error,
            critical_error,
        ]
        .into_iter()
        .flatten()
        .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
    fn forget(&mut self) {
        self.owner = None;
        self.signatures.clear();
        self.has_equip_load = false;
        self.last_vitals = None;
    }
}

/// Instant kill rewards have their own limits. Periodic-per-second safety caps
/// must not silently truncate a card's displayed one-shot recovery amount.
fn aggregate_on_kill(modifiers: &[Modifier]) -> Vec<Modifier> {
    let mut totals = BTreeMap::<EffectField, f64>::new();
    for modifier in modifiers {
        if !modifier.value.is_finite() {
            continue;
        }
        if matches!(
            modifier.field,
            EffectField::ChangeHpRate
                | EffectField::ChangeHpPoint
                | EffectField::ChangeMpRate
                | EffectField::ChangeMpPoint
        ) {
            *totals.entry(modifier.field).or_default() += f64::from(modifier.value);
        }
    }
    totals
        .into_iter()
        .filter_map(|(field, value)| {
            let lower = match field {
                EffectField::ChangeHpRate | EffectField::ChangeMpRate => -15.0,
                EffectField::ChangeHpPoint => -150.0,
                EffectField::ChangeMpPoint => -40.0,
                _ => return None,
            };
            let value = value.clamp(lower, 0.0) as f32;
            if value == 0.0 {
                return None;
            }
            Some(Modifier {
                field,
                value: if field.is_integer() {
                    value.round()
                } else {
                    value
                },
            })
        })
        .collect()
}
