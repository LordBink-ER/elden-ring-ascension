//! Constellation Chorus watches the native nearby Blood Loss and Poison/Rot
//! conditions used by Lord of Blood's / Kindred of Rot's Exultation. Any source
//! can trigger them, including the player or allies. No proc attribution is made.
//! Only owned watchers are applied directly; their native condition grants a
//! finite owned magic/FP-cost buff. Source-derived and game-unverified.
use eldenring::cs::{
    ChrInsExt, FieldInsHandle, GameMan, SoloParamRepository, SpEffectParam, WorldChrMan,
};
use fromsoftware_shared::FromStatic;
use serde::Serialize;
use std::collections::BTreeSet;

pub const BLOOD_WATCHER_BASE: u32 = 95_000_416;
pub const ROT_WATCHER_BASE: u32 = 95_005_416;
pub const CHORUS_CHILD_BASE: u32 = 95_010_416;
const RANKS: usize = 5;

#[derive(Clone, Copy)]
struct RankSpec {
    blood: u32,
    rot: u32,
    child: u32,
    magic_rate: f32,
    fp_cost: f32,
    duration: f32,
}

#[derive(Clone, PartialEq, Eq)]
struct OwnerKey {
    handle: FieldInsHandle,
    save_slot: i32,
    player_data_address: usize, // transient tag only; never dereferenced later
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AfflictionSummary {
    pub level: u8,
    pub parent_rows: Vec<u32>,
    pub child_row: Option<u32>,
    pub buff_active: bool,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct AfflictionBuffs {
    ready: bool,
    ranks: Vec<RankSpec>,
    owner: Option<OwnerKey>,
    level: u8,
}

impl AfflictionBuffs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Validate the native nearby-event conditions and finite, nonrecursive buff.
    /// # Safety
    /// Gated offline native game callback; loaded generated param package.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        self.ready = false;
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("nearby-affliction repository unavailable: {e}"))?;
        // Native presence carriers, not afflicted enemies' current buildup.
        for (id, state) in [(501_u32, 379_u16), (506, 380)] {
            let presence = repo
                .get::<SpEffectParam>(id)
                .ok_or_else(|| format!("native nearby-affliction presence {id} missing"))?;
            if presence.state_info() != state || presence.effect_endurance() != 0.0 {
                return Err(format!("native presence event {id} changed"));
            }
        }
        let mut ranks = Vec::with_capacity(RANKS);
        for index in 0..RANKS as u32 {
            let blood = BLOOD_WATCHER_BASE + index;
            let rot = ROT_WATCHER_BASE + index;
            let child_id = CHORUS_CHILD_BASE + index;
            for (id, condition) in [(blood, 379_u16), (rot, 380_u16)] {
                let parent = repo
                    .get::<SpEffectParam>(id)
                    .ok_or_else(|| format!("owned affliction watcher {id} missing"))?;
                if parent.state_info() != 0
                    || parent.sp_category() != 0
                    || parent.effect_endurance() != -1.0
                    || parent.motion_interval() != 0.0
                    || parent.invocation_conditions_state_change1() != condition
                    || parent.invocation_conditions_state_change2() != 0
                    || parent.invocation_conditions_state_change3() != 0
                    || parent.cycle_occurrence_sp_effect_id() != child_id as i32
                    || parent.atk_occurrence_sp_effect_id() != -1
                    || parent.apply_id_on_get_soul() != 0
                    || parent.magic_attack_rate() != 1.0
                    || parent.magic_consumption_rate() != 1.0
                    || parent.miracle_consumption_rate() != 1.0
                    || parent.change_hp_rate() != 0.0
                    || parent.change_hp_point() != 0
                    || parent.change_mp_rate() != 0.0
                    || parent.change_mp_point() != 0
                {
                    return Err(format!(
                        "owned affliction watcher {id} has incompatible semantics"
                    ));
                }
            }
            let child = repo
                .get::<SpEffectParam>(child_id)
                .ok_or_else(|| format!("owned affliction payload {child_id} missing"))?;
            let magic_rate = child.magic_attack_rate();
            let fp_cost = child.magic_consumption_rate();
            let duration = child.effect_endurance();
            if child.state_info() != 0
                || child.sp_category() != 0
                || !duration.is_finite()
                || duration <= 0.0
                || duration > 60.0
                || !magic_rate.is_finite()
                || !(1.0..=1.5).contains(&magic_rate)
                || !fp_cost.is_finite()
                || !(0.1..1.0).contains(&fp_cost)
                || (child.miracle_consumption_rate() - fp_cost).abs() > 0.0001
                || child.motion_interval() != 0.0
                || child.invocation_conditions_state_change1() != 0
                || child.invocation_conditions_state_change2() != 0
                || child.invocation_conditions_state_change3() != 0
                || child.cycle_occurrence_sp_effect_id() != -1
                || child.atk_occurrence_sp_effect_id() != -1
                || child.apply_id_on_get_soul() != 0
                || child.change_hp_rate() != 0.0
                || child.change_hp_point() != 0
                || child.change_mp_rate() != 0.0
                || child.change_mp_point() != 0
            {
                return Err(format!("owned affliction payload {child_id} is invalid"));
            }
            ranks.push(RankSpec {
                blood,
                rot,
                child: child_id,
                magic_rate,
                fp_cost,
                duration,
            });
        }
        self.ranks = ranks;
        self.ready = true;
        Ok(())
    }

    /// Keep only the selected rank's two event watchers. The engine starts and
    /// expires the child; this callback never polls an enemy or grants a buff.
    /// # Safety
    /// Gated native game task only, offline with the pinned executable/package.
    pub unsafe fn reconcile(
        &mut self,
        magic_rate: f32,
        fp_cost: f32,
        duration_seconds: f32,
    ) -> Result<AfflictionSummary, String> {
        let game = unsafe { GameMan::instance() }
            .map_err(|e| format!("nearby-affliction game state unavailable: {e}"))?;
        if !crate::game::actual_offline(game) {
            unsafe {
                self.clear()?;
            }
            return Err("Constellation Chorus requires offline mode".into());
        }
        let save_slot = game.save_slot;
        if !magic_rate.is_finite() || !fp_cost.is_finite() || !duration_seconds.is_finite() {
            unsafe {
                self.clear()?;
            }
            return Err("Constellation Chorus requires finite modifiers and duration".into());
        }
        if magic_rate <= 1.0 || fp_cost >= 1.0 || duration_seconds <= 0.0 {
            unsafe {
                self.clear()?;
            }
            return Ok(AfflictionSummary::default());
        }
        if let Err(error) = unsafe { self.ensure_ready() } {
            unsafe {
                self.clear()?;
            }
            return Err(error);
        }
        let Some(index) = self.ranks.iter().position(|rank| {
            (rank.magic_rate - magic_rate).abs() <= 0.0001
                && (rank.fp_cost - fp_cost).abs() <= 0.0001
                && (rank.duration - duration_seconds).abs() <= 0.001
        }) else {
            unsafe {
                self.clear()?;
            }
            return Err(
                "Constellation Chorus differs from generated native rows; rebuild params".into(),
            );
        };
        let spec = self.ranks[index];
        let level = index as u8 + 1;
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("nearby-affliction world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.forget();
            return Ok(AfflictionSummary::default());
        };
        let data = unsafe { player.player_game_data.as_ref() };
        if player.chr_ins.modules.data.hp <= 0
            || !unsafe { crate::game::native_offline_host(game, data) }
        {
            remove_owned(&mut player.chr_ins);
            self.forget();
            return Ok(AfflictionSummary::default());
        }
        let key = OwnerKey {
            handle: player.chr_ins.field_ins_handle,
            save_slot,
            player_data_address: data as *const _ as usize,
        };
        if self.owner.as_ref() != Some(&key) || self.level != level {
            remove_owned(&mut player.chr_ins);
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
            for id in [
                BLOOD_WATCHER_BASE + offset,
                ROT_WATCHER_BASE + offset,
                CHORUS_CHILD_BASE + offset,
            ] {
                if id != spec.blood && id != spec.rot && id != spec.child && live.contains(&id) {
                    player.chr_ins.remove_speffect(id as i32);
                }
            }
        }
        for parent in [spec.blood, spec.rot] {
            if !live.contains(&parent) {
                player.apply_speffect(parent as i32, true);
            }
        }
        Ok(AfflictionSummary {
            level,
            parent_rows: vec![spec.blood, spec.rot],
            child_row: Some(spec.child),
            buff_active: live.contains(&spec.child),
            warnings: Vec::new(),
        })
    }

    /// # Safety
    /// Native callback only; removes only this module's fifteen owned IDs.
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

fn remove_owned(chr: &mut eldenring::cs::ChrIns) {
    for offset in 0..RANKS as u32 {
        for id in [
            BLOOD_WATCHER_BASE + offset,
            ROT_WATCHER_BASE + offset,
            CHORUS_CHILD_BASE + offset,
        ] {
            chr.remove_speffect(id as i32);
        }
    }
}
