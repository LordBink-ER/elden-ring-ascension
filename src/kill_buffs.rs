//! Rune Echo: a native OnGetSoul carrier grants an eight-second rune-rate buff.
//! No synthetic kill event, rune-count delta watch, or retroactive rune grant.
//! Only the owned parent is applied directly; the native event applies its child.
use eldenring::cs::{
    ChrInsExt, FieldInsHandle, GameMan, SoloParamRepository, SpEffectParam, WorldChrMan,
};
use fromsoftware_shared::FromStatic;
use serde::Serialize;
use std::collections::BTreeSet;

pub const RUNE_ECHO_PARENT_BASE: u32 = 95_000_424;
pub const RUNE_ECHO_CHILD_BASE: u32 = 95_005_424;
const RANKS: usize = 5;

#[derive(Clone, Copy)]
struct RankSpec {
    parent: u32,
    child: u32,
    rate: f32,
    duration: f32,
}

#[derive(Clone, PartialEq, Eq)]
struct OwnerKey {
    handle: FieldInsHandle,
    save_slot: i32,
    // A transient allocation tag, never dereferenced or used as save identity.
    player_data_address: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KillBuffSummary {
    pub level: u8,
    pub parent_row: Option<u32>,
    pub child_row: Option<u32>,
    pub buff_active: bool,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct KillBuffs {
    ready: bool,
    ranks: Vec<RankSpec>,
    owner: Option<OwnerKey>,
    level: u8,
}

impl KillBuffs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Validate the finite native event graph before exposing the capability.
    /// Rate/duration come from loaded rows, so regenerated settings do not
    /// require recompiling this module.
    /// # Safety
    /// Gated offline native task only; the generated param package must be loaded.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        self.ready = false;
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("timed kill-buff repository unavailable: {e}"))?;
        let mut ranks = Vec::with_capacity(RANKS);
        for index in 0..RANKS {
            let parent_id = RUNE_ECHO_PARENT_BASE + index as u32;
            let child_id = RUNE_ECHO_CHILD_BASE + index as u32;
            let parent = repo
                .get::<SpEffectParam>(parent_id)
                .ok_or_else(|| format!("Rune Echo parent {parent_id} missing"))?;
            let child = repo
                .get::<SpEffectParam>(child_id)
                .ok_or_else(|| format!("Rune Echo child {child_id} missing"))?;
            let duration = child.effect_endurance();
            let rate = child.soul_rate();
            if parent.state_info() != 199
                || parent.effect_endurance() != -1.0
                || parent.sp_category() != 0
                || parent.soul_rate() != 1.0
                || parent.apply_id_on_get_soul() != child_id as i32
                || parent.atk_occurrence_sp_effect_id() != -1
                || parent.cycle_occurrence_sp_effect_id() != -1
                || parent.change_hp_rate() != 0.0
                || parent.change_hp_point() != 0
                || parent.change_mp_rate() != 0.0
                || parent.change_mp_point() != 0
            {
                return Err(format!(
                    "Rune Echo carrier {parent_id} has incompatible semantics"
                ));
            }
            if child.state_info() != 76
                || child.sp_category() != 0
                || !duration.is_finite()
                || duration <= 0.0
                || duration > 60.0
                || !rate.is_finite()
                || rate <= 1.0
                || rate > 1.5
                || child.apply_id_on_get_soul() != 0
                || child.atk_occurrence_sp_effect_id() != -1
                || child.cycle_occurrence_sp_effect_id() != -1
                || child.change_hp_rate() != 0.0
                || child.change_hp_point() != 0
                || child.change_mp_rate() != 0.0
                || child.change_mp_point() != 0
            {
                return Err(format!("Rune Echo finite payload {child_id} is invalid"));
            }
            ranks.push(RankSpec {
                parent: parent_id,
                child: child_id,
                rate,
                duration,
            });
        }
        self.ranks = ranks;
        self.ready = true;
        Ok(())
    }

    /// Maintain the matching native parent. The child is granted by OnGetSoul
    /// and expires in the engine; this callback never starts its timer itself.
    /// # Safety
    /// Native game callback only, with the pinned executable and package.
    pub unsafe fn reconcile(
        &mut self,
        rune_multiplier: f32,
        duration_seconds: f32,
    ) -> Result<KillBuffSummary, String> {
        let game = unsafe { GameMan::instance() }
            .map_err(|e| format!("timed kill-buff game state unavailable: {e}"))?;
        if !crate::game::actual_offline(game) {
            unsafe {
                self.clear()?;
            }
            return Err("Rune Echo requires offline mode".into());
        }
        let save_slot = game.save_slot;
        if !rune_multiplier.is_finite() || !duration_seconds.is_finite() {
            unsafe {
                self.clear()?;
            }
            return Err("Rune Echo requires finite rate and duration".into());
        }
        if rune_multiplier <= 1.0 || duration_seconds <= 0.0 {
            unsafe {
                self.clear()?;
            }
            return Ok(KillBuffSummary::default());
        }
        if let Err(error) = unsafe { self.ensure_ready() } {
            unsafe {
                self.clear()?;
            }
            return Err(error);
        }
        let rank = self.ranks.iter().position(|rank| {
            (rank.rate - rune_multiplier).abs() <= 0.0001
                && (rank.duration - duration_seconds).abs() <= 0.001
        });
        let Some(index) = rank else {
            unsafe {
                self.clear()?;
            }
            return Err(
                "Rune Echo definition differs from its generated native rows; rebuild params"
                    .into(),
            );
        };
        let spec = self.ranks[index];
        let level = index as u8 + 1;
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("Rune Echo world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.forget();
            return Ok(KillBuffSummary::default());
        };
        let data = unsafe { player.player_game_data.as_ref() };
        if player.chr_ins.modules.data.hp <= 0
            || !unsafe { crate::game::native_offline_host(game, data) }
        {
            for offset in 0..RANKS as u32 {
                player
                    .chr_ins
                    .remove_speffect((RUNE_ECHO_PARENT_BASE + offset) as i32);
                player
                    .chr_ins
                    .remove_speffect((RUNE_ECHO_CHILD_BASE + offset) as i32);
            }
            self.forget();
            return Ok(KillBuffSummary::default());
        }
        let key = OwnerKey {
            handle: player.chr_ins.field_ins_handle,
            save_slot,
            player_data_address: data as *const _ as usize,
        };
        if self.owner.as_ref() != Some(&key) || self.level != level {
            // A new rank/character never inherits the old rank's live timer.
            for offset in 0..RANKS as u32 {
                player
                    .chr_ins
                    .remove_speffect((RUNE_ECHO_PARENT_BASE + offset) as i32);
                player
                    .chr_ins
                    .remove_speffect((RUNE_ECHO_CHILD_BASE + offset) as i32);
            }
            self.owner = Some(key);
            self.level = level;
        }
        let live: BTreeSet<u32> = player
            .chr_ins
            .special_effect
            .entries()
            .filter_map(|entry| u32::try_from(entry.param_id).ok())
            .collect();
        for offset in 0..RANKS as u32 {
            let parent = RUNE_ECHO_PARENT_BASE + offset;
            let child = RUNE_ECHO_CHILD_BASE + offset;
            if parent != spec.parent && live.contains(&parent) {
                player.chr_ins.remove_speffect(parent as i32);
            }
            if child != spec.child && live.contains(&child) {
                player.chr_ins.remove_speffect(child as i32);
            }
        }
        if !live.contains(&spec.parent) {
            player.apply_speffect(spec.parent as i32, true);
        }
        Ok(KillBuffSummary {
            level,
            parent_row: Some(spec.parent),
            child_row: Some(spec.child),
            buff_active: live.contains(&spec.child),
            warnings: Vec::new(),
        })
    }

    /// # Safety
    /// Native callback only. Only this module's five parent/child pairs change.
    pub unsafe fn clear(&mut self) -> Result<(), String> {
        if let Ok(world) = unsafe { WorldChrMan::instance_mut() } {
            if let Some(player) = world.main_player.as_mut() {
                for offset in 0..RANKS as u32 {
                    player
                        .chr_ins
                        .remove_speffect((RUNE_ECHO_PARENT_BASE + offset) as i32);
                    player
                        .chr_ins
                        .remove_speffect((RUNE_ECHO_CHILD_BASE + offset) as i32);
                }
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
