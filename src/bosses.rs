//! Ascended World: apply one owned SpEffect to actual native boss-HUD actors.
//! There are no NPC-ID guesses, global enemy edits, or HP assignments here.
//! Native current/max HP coupling uses bCurrHPIndependeMaxHP=false. Behavior
//! is source-derived and game-unverified under the user's no-test instruction.
use eldenring::cs::{
    CSFeManImp, ChrInsExt, FieldInsHandle, GameMan, SoloParamRepository, SpEffectParam, WorldChrMan,
};
use crate::fate::{BOSS_AFFIXES, BossAffix};
use fromsoftware_shared::FromStatic;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, String>;
/// 0.4: one owned row per boss tier (effect_id + tier index: minor, greater,
/// major, remembrance). The tier of a boss on screen comes from its health
/// bar's NpcName id (config/boss-tiers-by-name.json); unknown names = greater.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BossScalingConfig {
    pub effect_id: i32,
    /// (max HP multiplier, damage multiplier) per tier.
    pub tiers: [(f32, f32); 4],
    pub names: BTreeMap<i32, u8>,
    /// 0.8 [world] boss_poise (1.2 = 20% more poise).
    pub poise: f32,
}
const DEFAULT_TIER: u8 = 1;
impl BossScalingConfig {
    fn validate(&self) -> Result<()> {
        if self.effect_id <= 0
            || self.tiers.iter().any(|(hp, dmg)| {
                !hp.is_finite() || !dmg.is_finite() || !(0.1..=10.0).contains(hp) || !(0.1..=10.0).contains(dmg)
            })
            || self.names.values().any(|tier| *tier > 3)
        {
            return Err("invalid owned boss effect/multipliers".into());
        }
        Ok(())
    }
    fn row(&self, tier: u8) -> i32 {
        self.effect_id + i32::from(tier.min(3))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BossScaleEvent {
    pub kind: String,
    pub actor: String,
    pub npc_param_id: i32,
    pub before_hp: i32,
    pub before_max_hp: i32,
    pub current_hp: i32,
    pub current_max_hp: i32,
    pub message: String,
    /// 3.0: the affix names of an Ascended Boss ("ascended", "ascended_felled").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affixes: Vec<String>,
    /// How many of them are the boss's own Ascended affixes.
    #[serde(default)]
    pub ascended: u8,
}
struct TrackedBoss {
    /// 0.5: NG+ affix rows applied to this actor so far (bounded).
    affix_attempts: u8,
    /// 3.0: this boss's affixes (the NG+ cycle's plus its own Ascended ones),
    /// how many are Ascended ones, and whether its fall was reported.
    affixes: Vec<BossAffix>,
    ascended: u8,
    felled: bool,
    hp_multiplier: f32,
    address: usize,
    npc_param_id: i32,
    base_hp: i32,
    before_hp: i32,
    before_max: i32,
    attempts: u8,
    last_attempt: u64,
    verified: bool,
    warning_sent: bool,
}
pub struct BossScaler {
    config: BossScalingConfig,
    tracked: BTreeMap<FieldInsHandle, TrackedBoss>,
    tick: u64,
    installed: bool,
    /// 0.5: affixes of the current NG+ cycle (set by the runtime each tick).
    affixes: Vec<BossAffix>,
    /// 0.6 Adaptive: damage kind of the player's weapon (index into ELEMENTS).
    adaptive_element: usize,
    affixes_installed: bool,
    /// 3.0 Ascended Bosses: (character seed, NG+ cycle, Tide) when enabled.
    ascension: Option<(u64, u32, u8)>,
    /// Affixes of the bosses on screen now (for the HUD).
    fight: Vec<BossAffix>,
}
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;
impl BossScaler {
    pub fn new(config: BossScalingConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            tracked: BTreeMap::new(),
            tick: 0,
            installed: false,
            affixes: Vec::new(),
            adaptive_element: 0,
            affixes_installed: false,
            ascension: None,
            fight: Vec::new(),
        })
    }
    /// The NG+ affixes every boss should carry now (empty = none).
    pub fn set_affixes(&mut self, affixes: Vec<BossAffix>) {
        if affixes != self.affixes {
            self.affixes = affixes;
            for tracked in self.tracked.values_mut() {
                tracked.affix_attempts = 0;
                for affix in &self.affixes {
                    if !tracked.affixes.contains(affix) {
                        tracked.affixes.push(*affix);
                    }
                }
            }
        }
    }
    pub fn affixes(&self) -> &[BossAffix] {
        &self.affixes
    }
    /// 3.0: Ascended Bosses on (seed, cycle, Tide) or off (None). Applies to
    /// bosses met from now on.
    pub fn set_ascension(&mut self, ascension: Option<(u64, u32, u8)>) {
        self.ascension = ascension;
    }
    /// 3.0: the affixes of the bosses on screen now.
    pub fn fight_affixes(&self) -> &[BossAffix] {
        &self.fight
    }
    /// 0.6 Adaptive: what the next boss adapts to (applies to new bosses).
    pub fn set_adaptive_element(&mut self, element: usize) {
        self.adaptive_element = element.min(crate::fate::ELEMENTS.len() - 1);
    }
    pub fn adaptive_element(&self) -> usize {
        self.adaptive_element
    }
    /// Boss affix rows from the neutral template (no vanilla row edited).
    /// # Safety
    /// Main native game thread after the executable and package gates.
    unsafe fn install_affixes(&mut self) -> Result<()> {
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let neutral = params
            .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
            .ok_or("neutral template missing")?
            .clone();
        for affix in BOSS_AFFIXES {
            let row = params
                .get_mut::<SpEffectParam>(affix.row())
                .ok_or("boss affix row missing; regenerate the package")?;
            *row = neutral.clone();
            row.set_effect_endurance(-1.0);
            row.set_sp_category(0);
            row.set_save_category(-1);
            row.set_state_info(0);
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_player(false);
            row.set_b_curr_hp_independe_max_hp(false);
            match affix {
                BossAffix::Relentless => {
                    row.set_toughness_damage_cut_rate(0.6);
                    row.set_vfx_id(8570);
                }
                BossAffix::Ironclad => {
                    row.set_slash_damage_cut_rate(0.85);
                    row.set_blow_damage_cut_rate(0.85);
                    row.set_thrust_damage_cut_rate(0.85);
                    row.set_neutral_damage_cut_rate(0.85);
                    row.set_vfx_id(8620);
                }
                BossAffix::Spellward => {
                    row.set_magic_damage_cut_rate(0.85);
                    row.set_fire_damage_cut_rate(0.85);
                    row.set_thunder_damage_cut_rate(0.85);
                    row.set_dark_damage_cut_rate(0.85);
                }
                BossAffix::Frenzied => {
                    row.set_physics_attack_rate(1.12);
                    row.set_magic_attack_rate(1.12);
                    row.set_fire_attack_rate(1.12);
                    row.set_thunder_attack_rate(1.12);
                    row.set_dark_attack_rate(1.12);
                    row.set_vfx_id(8600);
                }
                BossAffix::Regenerating => {
                    // Negative = healing, % of max HP per motion interval.
                    row.set_change_hp_rate(-0.25);
                    row.set_motion_interval(1.0);
                    row.set_vfx_id(1_643_100);
                }
                // Adaptive's own row is its physical variant, written below.
                BossAffix::Adaptive => {}
            }
        }
        // 0.6 Adaptive: one row per damage kind.
        for (element, _) in crate::fate::ELEMENTS.iter().enumerate() {
            let row = params
                .get_mut::<SpEffectParam>(BossAffix::Adaptive.row_for(element))
                .ok_or("adaptive affix row missing; regenerate the package")?;
            *row = neutral.clone();
            row.set_effect_endurance(-1.0);
            row.set_sp_category(0);
            row.set_save_category(-1);
            row.set_state_info(0);
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_player(false);
            row.set_b_curr_hp_independe_max_hp(false);
            match element {
                0 => {
                    row.set_slash_damage_cut_rate(0.75);
                    row.set_blow_damage_cut_rate(0.75);
                    row.set_thrust_damage_cut_rate(0.75);
                    row.set_neutral_damage_cut_rate(0.75);
                }
                1 => row.set_magic_damage_cut_rate(0.75),
                2 => row.set_fire_damage_cut_rate(0.75),
                3 => row.set_thunder_damage_cut_rate(0.75),
                _ => row.set_dark_damage_cut_rate(0.75),
            }
            row.set_vfx_id(8620);
        }
        self.affixes_installed = true;
        Ok(())
    }

    /// Configure only the reserved row generated by this mod. The ID must come
    /// from the validated package manifest, never a discovered vanilla row.
    /// # Safety
    /// Main native game thread after the executable and package gates.
    pub unsafe fn install(&mut self) -> Result<()> {
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        for tier in 0..4u8 {
            let (hp, damage) = self.config.tiers[usize::from(tier)];
            let row = params
                .get_mut::<SpEffectParam>(self.config.row(tier) as u32)
                .ok_or("owned Ascended World SpEffect row is missing; regenerate the package")?;
            if row.change_hp_rate() != 0.0 || row.change_hp_point() != 0 {
                return Err("owned boss effect unexpectedly has periodic HP damage/healing".into());
            }
            if row.soul_rate() != 1.0 {
                return Err(
                    "owned boss effect must preserve normal rune rewards; rebuild Ascension params"
                        .into(),
                );
            }
            row.set_max_hp_rate(hp);
            // AttackRate adjusts calculated damage. AttackPowerRate would also
            // change defense interactions; applying both would double the bonus.
            row.set_physics_attack_rate(damage);
            row.set_magic_attack_rate(damage);
            row.set_fire_attack_rate(damage);
            row.set_thunder_attack_rate(damage);
            row.set_dark_attack_rate(damage);
            row.set_effect_endurance(-1.0);
            row.set_toughness_damage_cut_rate((1.0 / self.config.poise.clamp(0.5, 3.0)).clamp(0.3, 2.0));
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_player(false);
            // Official param description: whether current HP is unaffected when

            // maximum HP changes. Vanilla area-scaling rows use false as well.
            row.set_b_curr_hp_independe_max_hp(false);
        }
        self.installed = true;
        Ok(())
    }
    fn owned(&self, id: i32) -> bool {
        (self.config.effect_id..self.config.effect_id + 4).contains(&id)
    }

    /// Observe all three native boss health displays. Multiple UI bars pointing
    /// to one actor are deduplicated. The row composes with the actor's current
    /// native NG+/co-op corrections; those rows and base HP remain untouched.
    /// # Safety
    /// Main game task only, no concurrent game references/mutations. Caller
    /// passes false during unavailable/loading/remote states. No I/O here.
    pub unsafe fn tick(&mut self, allow_apply: bool) -> Result<Vec<BossScaleEvent>> {
        self.tick = self.tick.saturating_add(1);
        if !allow_apply {
            return Ok(Vec::new());
        }
        if !self.installed {
            unsafe { self.install() }?;
        }
        // 3.0: Ascended Bosses need the affix rows too, before any NG+.
        if (!self.affixes.is_empty() || self.ascension.is_some()) && !self.affixes_installed {
            unsafe { self.install_affixes() }?;
        }
        {
            let game = unsafe { GameMan::instance() }.map_err(|e| e.to_string())?;
            let world = unsafe { WorldChrMan::instance() }.map_err(|e| e.to_string())?;
            let Some(player) = world.main_player.as_ref() else {
                return Ok(Vec::new());
            };
            let player_data = unsafe { player.player_game_data.as_ref() };
            if !unsafe { crate::game::native_offline_host(game, player_data) }
                || game.warp_requested
                || game.new_game_plus_requested
            {
                return Ok(Vec::new());
            }
        }
        let handles: BTreeMap<FieldInsHandle, i32> = {
            let fe = unsafe { CSFeManImp::instance() }.map_err(|e| e.to_string())?;
            fe.boss_health_displays
                .iter()
                .filter(|display| display.fmg_id > 0 && !display.field_ins_handle.is_empty())
                .map(|display| (display.field_ins_handle, display.fmg_id))
                .collect()
        };
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let main_handle = world
            .main_player
            .as_ref()
            .map(|player| player.chr_ins.field_ins_handle);
        let mut events = Vec::new();
        let mut fight: Vec<BossAffix> = Vec::new();
        // 3.0: an Ascended Boss that fell (HP 0 or its death flag) is
        // reported once; the reward comes with the boss's defeat flag.
        for (handle, tracked) in self.tracked.iter_mut().filter(|(_, t)| t.ascended > 0 && !t.felled) {
            let Some(actor) = world.chr_ins_by_handle_mut(handle) else {
                continue;
            };
            if actor as *mut _ as usize != tracked.address {
                continue;
            }
            if actor.modules.data.hp <= 0 || actor.chr_flags1c5.death_flag() {
                tracked.felled = true;
                let mut felled = event(
                    "ascended_felled",
                    *handle,
                    tracked,
                    actor.modules.data.hp,
                    actor.modules.data.max_hp,
                    &format!("Ascended Boss felled ({} affixes)", tracked.ascended),
                );
                felled.affixes = tracked.affixes.iter().map(|a| a.name().to_owned()).collect();
                felled.ascended = tracked.ascended;
                events.push(felled);
            }
        }
        for (handle, name_id) in handles {
            if Some(handle) == main_handle {
                continue;
            }
            let tier = self.config.names.get(&name_id).copied().unwrap_or(DEFAULT_TIER);
            let effect_id = self.config.row(tier);
            let hp_multiplier = self.config.tiers[usize::from(tier.min(3))].0;
            let Some(actor) = world.chr_ins_by_handle_mut(&handle) else {
                continue;
            };
            if actor.modules.data.hp <= 0
                || actor.modules.data.max_hp <= 0
                || actor.chr_flags1c5.death_flag()
            {
                continue;
            }
            let address = actor as *mut _ as usize;
            let npc_param_id = actor.npc_param_id;
            let base_hp = actor.modules.data.base_hp;
            let existing = self.tracked.get(&handle);
            if existing.is_none_or(|tracked| {
                tracked.address != address
                    || tracked.npc_param_id != npc_param_id
                    || tracked.base_hp != base_hp
            }) {
                let effect_present = actor
                    .special_effect
                    .entries()
                    .any(|effect| self.owned(effect.param_id));
                // 3.0: the NG+ cycle's affixes plus this boss's own.
                let own = self
                    .ascension
                    .map(|(seed, cycle, tide)| crate::fate::ascended_affixes(seed, name_id, cycle, tide, tier))
                    .unwrap_or_default();
                let mut affixes = self.affixes.clone();
                let mut ascended = 0u8;
                for affix in own {
                    if !affixes.contains(&affix) {
                        affixes.push(affix);
                        ascended += 1;
                    }
                }
                if ascended > 0 {
                    let names: Vec<String> = affixes.iter().map(|a| a.name().to_owned()).collect();
                    events.push(BossScaleEvent {
                        kind: "ascended".into(),
                        actor: handle.to_string(),
                        npc_param_id,
                        before_hp: actor.modules.data.hp,
                        before_max_hp: actor.modules.data.max_hp,
                        current_hp: actor.modules.data.hp,
                        current_max_hp: actor.modules.data.max_hp,
                        message: format!("Ascended Boss (name {name_id}, tier {tier}): {}", names.join(" + ")),
                        affixes: names,
                        ascended,
                    });
                }
                self.tracked.insert(
                    handle,
                    TrackedBoss {
                        affix_attempts: 0,
                        affixes,
                        ascended,
                        felled: false,
                        hp_multiplier,
                        address,
                        npc_param_id,
                        base_hp,
                        before_hp: actor.modules.data.hp,
                        before_max: actor.modules.data.max_hp,
                        attempts: 0,
                        last_attempt: 0,
                        verified: effect_present,
                        warning_sent: false,
                    },
                );
                if effect_present {
                    events.push(BossScaleEvent {
                        kind: "already_active".into(),
                        actor: handle.to_string(),
                        npc_param_id,
                        before_hp: actor.modules.data.hp,
                        before_max_hp: actor.modules.data.max_hp,
                        current_hp: actor.modules.data.hp,
                        current_max_hp: actor.modules.data.max_hp,
                        message:
                            "Owned boss effect already present; no second multiplier or HP refill"
                                .into(),
                        affixes: Vec::new(),
                        ascended: 0,
                    });
                }
            }
            let present = actor
                .special_effect
                .entries()
                .any(|effect| self.owned(effect.param_id));
            let tracked = self.tracked.get_mut(&handle).expect("inserted above");
            for affix in &tracked.affixes {
                if !fight.contains(affix) {
                    fight.push(*affix);
                }
            }
            // NG+ and Ascended affixes: each missing row is applied, a few
            // bounded times.
            if present && self.affixes_installed && tracked.affix_attempts < 3 {
                let adaptive_rows: Vec<i32> = (0..crate::fate::ELEMENTS.len())
                    .map(|e| BossAffix::Adaptive.row_for(e) as i32)
                    .collect();
                let missing: Vec<i32> = tracked
                    .affixes
                    .iter()
                    .filter(|a| {
                        // An adapted boss keeps the kind it first adapted to.
                        **a != BossAffix::Adaptive
                            || !actor.special_effect.entries().any(|e| adaptive_rows.contains(&e.param_id))
                    })
                    .map(|a| a.row_for(self.adaptive_element) as i32)
                    .filter(|row| !actor.special_effect.entries().any(|e| e.param_id == *row))
                    .collect();
                if !missing.is_empty() && self.tick.saturating_sub(tracked.last_attempt) >= 30 {
                    for row in &missing {
                        actor.apply_speffect(*row, true);
                    }
                    tracked.affix_attempts += 1;
                    tracked.last_attempt = self.tick;
                    let names: Vec<String> = tracked
                        .affixes
                        .iter()
                        .map(|a| match a {
                            BossAffix::Adaptive => format!(
                                "Adaptive ({})",
                                crate::fate::ELEMENTS[self.adaptive_element]
                            ),
                            other => other.name().to_owned(),
                        })
                        .collect();
                    events.push(event(
                        "affix",
                        handle,
                        tracked,
                        actor.modules.data.hp,
                        actor.modules.data.max_hp,
                        &format!("Boss affix applied: {}", names.join(" + ")),
                    ));
                }
            }
            if !present {
                if tracked.attempts >= 3 {
                    if !tracked.warning_sent {
                        events.push(event("warning",handle,tracked,actor.modules.data.hp,actor.modules.data.max_hp,"Owned effect did not remain active after three bounded attempts; further automatic application stopped"));
                        tracked.warning_sent = true;
                    }
                    continue;
                }
                if tracked.attempts > 0 && self.tick.saturating_sub(tracked.last_attempt) < 60 {
                    continue;
                }
                tracked.before_hp = actor.modules.data.hp;
                tracked.before_max = actor.modules.data.max_hp;
                tracked.attempts += 1;
                tracked.last_attempt = self.tick;
                tracked.verified = false;
                tracked.warning_sent = false;
                actor.apply_speffect(effect_id, true);
                events.push(event(
                    "applied",
                    handle,
                    tracked,
                    actor.modules.data.hp,
                    actor.modules.data.max_hp,
                    "Applied owned SpEffect; current/max HP coupling is left to the native engine",
                ));
                continue;
            }
            if !tracked.verified {
                let elapsed = self.tick.saturating_sub(tracked.last_attempt);
                let hp = actor.modules.data.hp;
                let max = actor.modules.data.max_hp;
                let expected =
                    (tracked.before_max as f64 * tracked.hp_multiplier as f64).round();
                let tolerance = (expected.abs() * 0.005).max(2.0);
                if (max as f64 - expected).abs() <= tolerance {
                    tracked.verified = true;
                    let suspicious = tracked.hp_multiplier > 1.05
                        && hp <= tracked.before_hp
                        && tracked.before_hp == tracked.before_max;
                    events.push(event(if suspicious {"warning"}else{"native_scale_observed"},handle,tracked,hp,max,
                        if suspicious {"Native maximum HP matches configuration, but current HP did not rise from a full-health baseline; no HP overwrite was performed (damage may have occurred)"}
                        else {"Native maximum HP matches the configured multiplier; observation is telemetry, not a gameplay test assertion"}));
                } else if elapsed >= 30 && !tracked.warning_sent {
                    tracked.warning_sent = true;
                    events.push(event("warning",handle,tracked,hp,max,"Owned effect is active but observed maximum HP differs from the expected baseline ratio; no vanilla rows or actor HP were overwritten"));
                }
            }
        }
        fight.sort();
        self.fight = fight;
        Ok(events)
    }

    /// Explicit feature-disable cleanup only. Do not call this on player death:
    /// surviving bosses keep their native scaling until their actor unloads.
    /// # Safety
    /// Main native task only, same version/package gates as tick.
    pub unsafe fn restore_owned(&mut self) -> Result<usize> {
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let mut removed = 0;
        for (handle, tracked) in &self.tracked {
            let Some(actor) = world.chr_ins_by_handle_mut(handle) else {
                continue;
            };
            if actor as *mut _ as usize != tracked.address
                || actor.npc_param_id != tracked.npc_param_id
                || actor.modules.data.base_hp != tracked.base_hp
            {
                continue;
            }
            let affix_rows = crate::fate::affix_rows().map(|row| row as i32);
            for id in (self.config.effect_id..self.config.effect_id + 4).chain(affix_rows) {
                if actor.special_effect.entries().any(|entry| entry.param_id == id) {
                    actor.remove_speffect(id);
                    removed += 1;
                }
            }
        }
        self.tracked.clear();
        Ok(removed)
    }
    /// A world unload invalidates handles. Never dereference them to clean up;
    /// the native actor lifetime owns the permanent effect's lifetime.
    pub fn forget_world(&mut self) {
        self.tracked.clear();
        self.installed = false;
        self.affixes_installed = false;
    }
}
fn event(
    kind: &str,
    handle: FieldInsHandle,
    tracked: &TrackedBoss,
    hp: i32,
    max: i32,
    message: &str,
) -> BossScaleEvent {
    BossScaleEvent {
        kind: kind.into(),
        actor: handle.to_string(),
        npc_param_id: tracked.npc_param_id,
        before_hp: tracked.before_hp,
        before_max_hp: tracked.before_max,
        current_hp: hp,
        current_max_hp: max,
        message: message.into(),
        affixes: Vec::new(),
        ascended: 0,
    }
}
