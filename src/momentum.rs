//! Relentless Momentum uses the engine's weighted successive-hit accumulator.
//! It does not infer hits from HP, runes, animation flags, or last_hit_by.
//!
//! Source graph: Godskin Swaddling Cloth 350400's decaying watcher and the
//! Crimson-Sapping Tear 20511022 -> 20511023 instantaneous reset/payload chain.
//! Our graph uses source-enumerated accumulator 9 (stateInfo 311), isolated
//! from Winged Sword's 303 and Millicent's 305. A live foreign 311 blocks it.
//! The native counter is never read, indexed, or assigned by this module.
//! Source-derived; game-unverified under the user's no-test instruction.
use eldenring::{
    cs::{ChrInsExt, FieldInsHandle, GameMan, SoloParamRepository, SpEffectParam, WorldChrMan},
    param::SP_EFFECT_PARAM_ST,
};
use fromsoftware_shared::FromStatic;
use serde::Serialize;
use std::collections::BTreeSet;

pub const MOMENTUM_PARENT_BASE: u32 = 95_000_408;
pub const MOMENTUM_RESET_BASE: u32 = 95_005_408;
pub const MOMENTUM_CHILD_BASE: u32 = 95_010_408;
pub const MOMENTUM_INITIAL_RESET: u32 = 95_015_408;
const RANKS: usize = 5;
const ACCUMULATOR_STATE: u16 = 311;

#[derive(Clone, Copy)]
struct RankSpec {
    parent: u32,
    reset: u32,
    child: u32,
    recovery: i32,
    cost: f32,
    duration: f32,
}

#[derive(Clone, PartialEq, Eq)]
struct OwnerKey {
    handle: FieldInsHandle,
    save_slot: i32,
    // Transient allocation discriminator only, never persistent save identity.
    player_data_address: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MomentumSummary {
    pub level: u8,
    pub parent_row: Option<u32>,
    pub child_row: Option<u32>,
    pub buff_active: bool,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct Momentum {
    ready: bool,
    ranks: Vec<RankSpec>,
    owner: Option<OwnerKey>,
    level: u8,
}

impl Momentum {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Verify the finite, mod-owned graph and the native all-accumulator feeds.
    /// This validates capability, not empirical behavior in the game.
    /// # Safety
    /// Gated native game callback only, pinned executable and regulation.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        self.ready = false;
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("momentum repository unavailable: {e}"))?;
        for index in 0..10_u32 {
            let id = 6900 + index;
            let row = repo
                .get::<SpEffectParam>(id)
                .ok_or_else(|| format!("native successive-hit feed {id} missing"))?;
            if row.state_info() != 314
                || row.accumu_val() != (index as i32 + 1) * 2
                || row.effect_endurance() != 0.0
            {
                return Err(format!(
                    "native successive-hit feed {id} changed; momentum disabled"
                ));
            }
        }
        let initial = repo
            .get::<SpEffectParam>(MOMENTUM_INITIAL_RESET)
            .ok_or("owned momentum initialization reset missing")?;
        if !valid_reset(initial, -1) {
            return Err("owned momentum initialization reset has incompatible semantics".into());
        }
        let mut ranks = Vec::with_capacity(RANKS);
        for index in 0..RANKS as u32 {
            let parent_id = MOMENTUM_PARENT_BASE + index;
            let reset_id = MOMENTUM_RESET_BASE + index;
            let child_id = MOMENTUM_CHILD_BASE + index;
            let parent = repo
                .get::<SpEffectParam>(parent_id)
                .ok_or_else(|| format!("momentum parent {parent_id} missing"))?;
            let reset = repo
                .get::<SpEffectParam>(reset_id)
                .ok_or_else(|| format!("momentum reset {reset_id} missing"))?;
            let child = repo
                .get::<SpEffectParam>(child_id)
                .ok_or_else(|| format!("momentum payload {child_id} missing"))?;
            if !neutral_resources(parent)
                || parent.state_info() != ACCUMULATOR_STATE
                || parent.effect_endurance() != -1.0
                || (parent.motion_interval() - 0.8).abs() > 0.0001
                || parent.accumu_val() != -1
                || parent.accumu_over_val() != 17
                || parent.accumu_over_fire_id() != reset_id as i32
                || parent.accumu_under_val() != -1
                || parent.accumu_under_fire_id() != -1
                || parent.cycle_occurrence_sp_effect_id() != -1
                || parent.stamina_recover_change_speed() != 0
                || parent.consume_stamina_rate() != 1.0
            {
                return Err(format!(
                    "momentum watcher {parent_id} has incompatible semantics"
                ));
            }
            if !valid_reset(reset, child_id as i32) {
                return Err(format!(
                    "momentum reset {reset_id} has incompatible semantics"
                ));
            }
            let duration = child.effect_endurance();
            let cost = child.consume_stamina_rate();
            let recovery = child.stamina_recover_change_speed();
            if !neutral_resources(child)
                || child.state_info() != 0
                || !duration.is_finite()
                || (duration - 4.0).abs() > 0.001
                || !cost.is_finite()
                || !(0.1..1.0).contains(&cost)
                || !(1..=100).contains(&recovery)
                || child.motion_interval() != 0.0
                || child.accumu_val() != 0
                || child.accumu_over_val() != -1
                || child.accumu_over_fire_id() != -1
                || child.accumu_under_val() != -1
                || child.accumu_under_fire_id() != -1
                || child.cycle_occurrence_sp_effect_id() != -1
            {
                return Err(format!("momentum payload {child_id} is invalid"));
            }
            ranks.push(RankSpec {
                parent: parent_id,
                reset: reset_id,
                child: child_id,
                recovery,
                cost,
                duration,
            });
        }
        self.ranks = ranks;
        // Capability advertisement must honor the same live ownership guard
        // as reconciliation. Merely validating param rows is insufficient.
        let world = unsafe { WorldChrMan::instance() }
            .map_err(|e| format!("momentum capability player unavailable: {e}"))?;
        let player = world
            .main_player
            .as_ref()
            .ok_or("momentum capability requires a loaded local player")?;
        if let Some(id) = foreign_accumulator9(&player.chr_ins) {
            return Err(format!(
                "momentum disabled: unowned/unknown accumulator-9 entry {id}"
            ));
        }
        self.ready = true;
        Ok(())
    }

    /// Install only a watcher. The engine grants/reset/expires the timed buff
    /// after actual weighted hit accumulation; this method cannot start it.
    /// # Safety
    /// Gated offline native task only. No I/O or game references may escape.
    pub unsafe fn reconcile(
        &mut self,
        stamina_recovery_bonus: i32,
        stamina_cost_rate: f32,
        duration_seconds: f32,
    ) -> Result<MomentumSummary, String> {
        let game = unsafe { GameMan::instance() }
            .map_err(|e| format!("momentum game state unavailable: {e}"))?;
        if !crate::game::actual_offline(game) {
            unsafe {
                self.clear()?;
            }
            return Err("momentum requires offline mode".into());
        }
        if game.warp_requested || game.new_game_plus_requested {
            return Ok(MomentumSummary::default());
        }
        let save_slot = game.save_slot;
        if !stamina_cost_rate.is_finite() || !duration_seconds.is_finite() {
            unsafe {
                self.clear()?;
            }
            return Err("momentum requires finite modifiers and duration".into());
        }
        if stamina_recovery_bonus <= 0 || stamina_cost_rate >= 1.0 || duration_seconds <= 0.0 {
            unsafe {
                self.clear()?;
            }
            return Ok(MomentumSummary::default());
        }
        if let Err(error) = unsafe { self.ensure_ready() } {
            unsafe {
                self.clear()?;
            }
            return Err(error);
        }
        let Some(index) = self.ranks.iter().position(|rank| {
            rank.recovery == stamina_recovery_bonus
                && (rank.cost - stamina_cost_rate).abs() <= 0.0001
                && (rank.duration - duration_seconds).abs() <= 0.001
        }) else {
            unsafe {
                self.clear()?;
            }
            return Err("momentum definition differs from generated rows; rebuild params".into());
        };
        let spec = self.ranks[index];
        let level = index as u8 + 1;
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("momentum world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.forget();
            return Ok(MomentumSummary::default());
        };
        let data = unsafe { player.player_game_data.as_ref() };
        if player.chr_ins.modules.data.hp <= 0
            || !unsafe { crate::game::native_offline_host(game, data) }
        {
            remove_owned(&mut player.chr_ins);
            self.forget();
            return Ok(MomentumSummary::default());
        }
        let key = OwnerKey {
            handle: player.chr_ins.field_ins_handle,
            save_slot,
            player_data_address: data as *const _ as usize,
        };
        if let Some(id) = foreign_accumulator9(&player.chr_ins) {
            // Do not reset this counter: the other entry owns it. Removing
            // only our graph avoids adding decay or resets to their meter.
            remove_owned(&mut player.chr_ins);
            self.forget();
            return Err(format!(
                "momentum disabled: unowned/unknown accumulator-9 entry {id}"
            ));
        }
        let live: BTreeSet<u32> = player
            .chr_ins
            .special_effect
            .entries()
            .filter_map(|entry| u32::try_from(entry.param_id).ok())
            .collect();
        if self.owner.as_ref() != Some(&key) || self.level != level || !live.contains(&spec.parent)
        {
            remove_owned(&mut player.chr_ins);
            // This reset has no child, so equipping a card grants no free buff.
            player.apply_speffect(MOMENTUM_INITIAL_RESET as i32, true);
            player.apply_speffect(spec.parent as i32, true);
            self.owner = Some(key);
            self.level = level;
            return Ok(MomentumSummary {
                level,
                parent_row: Some(spec.parent),
                child_row: Some(spec.child),
                buff_active: false,
                warnings: Vec::new(),
            });
        }
        // Remove obsolete ranks, never any vanilla/non-owned effect.
        for other in &self.ranks {
            if other.parent != spec.parent && live.contains(&other.parent) {
                player.chr_ins.remove_speffect(other.parent as i32);
            }
            if other.reset != spec.reset && live.contains(&other.reset) {
                player.chr_ins.remove_speffect(other.reset as i32);
            }
            if other.child != spec.child && live.contains(&other.child) {
                player.chr_ins.remove_speffect(other.child as i32);
            }
        }
        Ok(MomentumSummary {
            level,
            parent_row: Some(spec.parent),
            child_row: Some(spec.child),
            buff_active: live.contains(&spec.child),
            warnings: Vec::new(),
        })
    }

    /// Remove only this module's rows; never reset an unowned accumulator.
    /// # Safety
    /// Native game task only, after executable/version gating.
    pub unsafe fn clear(&mut self) -> Result<(), String> {
        if let Ok(world) = unsafe { WorldChrMan::instance_mut() } {
            if let Some(player) = world.main_player.as_mut() {
                remove_owned(&mut player.chr_ins);
            }
        }
        self.forget();
        Ok(())
    }

    fn forget(&mut self) {
        self.owner = None;
        self.level = 0;
    }
}

fn neutral_resources(row: &SP_EFFECT_PARAM_ST) -> bool {
    row.change_hp_rate() == 0.0
        && row.change_hp_point() == 0
        && row.change_mp_rate() == 0.0
        && row.change_mp_point() == 0
        && row.change_stamina_rate() == 0.0
        && row.change_stamina_point() == 0
        && row.soul_rate() == 1.0
        && row.apply_id_on_get_soul() == 0
        && row.atk_occurrence_sp_effect_id() == -1
        && row.replace_sp_effect_id() == -1
        && !row.effect_target_enemy()
}

fn valid_reset(row: &SP_EFFECT_PARAM_ST, child: i32) -> bool {
    neutral_resources(row)
        && row.state_info() == ACCUMULATOR_STATE
        && row.effect_endurance() == 0.0
        && row.motion_interval() == 0.0
        && row.accumu_val() == -999
        && row.accumu_over_val() == -1
        && row.accumu_over_fire_id() == -1
        && row.accumu_under_val() == -1
        && row.accumu_under_fire_id() == -1
        && row.cycle_occurrence_sp_effect_id() == child
        && row.stamina_recover_change_speed() == 0
        && row.consume_stamina_rate() == 1.0
}

fn owns(id: i32) -> bool {
    let Ok(id) = u32::try_from(id) else {
        return false;
    };
    id == MOMENTUM_INITIAL_RESET
        || (MOMENTUM_PARENT_BASE..MOMENTUM_PARENT_BASE + RANKS as u32).contains(&id)
        || (MOMENTUM_RESET_BASE..MOMENTUM_RESET_BASE + RANKS as u32).contains(&id)
        || (MOMENTUM_CHILD_BASE..MOMENTUM_CHILD_BASE + RANKS as u32).contains(&id)
}

fn foreign_accumulator9(player: &eldenring::cs::ChrIns) -> Option<i32> {
    for entry in player.special_effect.entries() {
        if owns(entry.param_id) {
            continue;
        }
        let Some(param) = entry.param_data.as_ref() else {
            // Unknown live entries cannot establish exclusive ownership.
            return Some(entry.param_id);
        };
        if unsafe { param.as_ref() }.state_info() == ACCUMULATOR_STATE {
            return Some(entry.param_id);
        }
    }
    None
}

fn remove_owned(player: &mut eldenring::cs::ChrIns) {
    for index in 0..RANKS as u32 {
        player.remove_speffect((MOMENTUM_PARENT_BASE + index) as i32);
        player.remove_speffect((MOMENTUM_RESET_BASE + index) as i32);
        player.remove_speffect((MOMENTUM_CHILD_BASE + index) as i32);
    }
    player.remove_speffect(MOMENTUM_INITIAL_RESET as i32);
}
