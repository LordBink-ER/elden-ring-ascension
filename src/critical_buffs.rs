//! Merciful Reversal deliberately enables the real Blade of Mercy blessing.
//! The game's critical hitboxes apply 20382301, gated by carrier state 483,
//! which grants native 20382302 for twenty seconds. Its damage bonus is not
//! copied, multiplied a second time, cancelled, or manually granted here.
//! Only our neutral carrier and posture followup rows are directly applied.
//! Source-derived; game-unverified under the user's no-test instruction.
use eldenring::{
    cs::{ChrInsExt, FieldInsHandle, GameMan, SoloParamRepository, SpEffectParam, WorldChrMan},
    param::SP_EFFECT_PARAM_ST,
};
use fromsoftware_shared::FromStatic;
use serde::Serialize;
use std::collections::BTreeSet;

pub const CRITICAL_CARRIER_BASE: u32 = 95_000_400;
pub const CRITICAL_POSTURE_BASE: u32 = 95_005_400;
pub const NATIVE_BLESSING: u32 = 20_382_302;
const NATIVE_CARRIER: u32 = 20_382_300;
const NATIVE_CRITICAL_RECEIPT: u32 = 20_382_301;
const RANKS: usize = 5;

#[derive(Clone, Copy)]
struct RankSpec {
    carrier: u32,
    posture: u32,
    posture_rate: f32,
}

#[derive(Clone, PartialEq, Eq)]
struct OwnerKey {
    handle: FieldInsHandle,
    save_slot: i32,
    // A transient native allocation discriminator, never save identity.
    player_data_address: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CriticalBuffSummary {
    pub level: u8,
    pub carrier_row: Option<u32>,
    pub posture_row: Option<u32>,
    pub native_blessing_active: bool,
    pub native_blessing_remaining_seconds: Option<f32>,
    pub posture_active: bool,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct CriticalBuffs {
    ready: bool,
    ranks: Vec<RankSpec>,
    owner: Option<OwnerKey>,
    level: u8,
}

impl CriticalBuffs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Validate the existing native critical graph and our neutral/posture rows.
    /// The native blessing is read only: the talisman keeps its exact behavior.
    /// # Safety
    /// Gated native callback only, pinned executable and generated regulation.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        self.ready = false;
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("critical blessing repository unavailable: {e}"))?;
        let native_carrier = repo
            .get::<SpEffectParam>(NATIVE_CARRIER)
            .ok_or("native Blade of Mercy carrier missing")?;
        if !valid_carrier(native_carrier, 1.0) {
            return Err("native Blade of Mercy carrier has incompatible semantics".into());
        }
        let receipt = repo
            .get::<SpEffectParam>(NATIVE_CRITICAL_RECEIPT)
            .ok_or("native Blade of Mercy critical receipt missing")?;
        if receipt.state_info() != 0
            || receipt.effect_endurance() != 1.0
            || receipt.motion_interval() != 0.0
            || receipt.invocation_conditions_state_change1() != 483
            || receipt.invocation_conditions_state_change2() != 0
            || receipt.invocation_conditions_state_change3() != 0
            || receipt.cycle_occurrence_sp_effect_id() != NATIVE_BLESSING as i32
            || receipt.replace_sp_effect_id() != -1
            || receipt.atk_occurrence_sp_effect_id() != -1
            || !receipt.effect_target_attacker()
            || !neutral_resources(receipt, 1.0)
            || !neutral_damage(receipt)
            || receipt.sa_attack_power_rate() != 1.0
        {
            return Err("native Blade of Mercy critical receipt changed".into());
        }
        let blessing = repo
            .get::<SpEffectParam>(NATIVE_BLESSING)
            .ok_or("native Blade of Mercy finite blessing missing")?;
        if blessing.state_info() != 0
            || blessing.effect_endurance() != 20.0
            || blessing.motion_interval() != 0.0
            || !no_links(blessing)
            || !neutral_resources(blessing, 1.0)
            || !base_attack_rates(blessing).into_iter().all(|r| r == 1.0)
            || !enemy_rates(blessing).into_iter().all(|r| close(r, 1.2))
            || !player_rates(blessing).into_iter().all(|r| close(r, 1.12))
            || blessing.sa_attack_power_rate() != 1.0
            || !blessing.effect_target_attacker()
        {
            return Err(
                "native Blade of Mercy blessing differs from the documented 20s damage bonus"
                    .into(),
            );
        }
        let mut ranks = Vec::with_capacity(RANKS);
        for index in 0..RANKS as u32 {
            let carrier_id = CRITICAL_CARRIER_BASE + index;
            let posture_id = CRITICAL_POSTURE_BASE + index;
            let carrier = repo
                .get::<SpEffectParam>(carrier_id)
                .ok_or_else(|| format!("owned critical carrier {carrier_id} missing"))?;
            let posture = repo
                .get::<SpEffectParam>(posture_id)
                .ok_or_else(|| format!("owned critical posture row {posture_id} missing"))?;
            if !valid_carrier(carrier, 1.0) {
                return Err(format!(
                    "owned critical carrier {carrier_id} is not neutral"
                ));
            }
            let rate = posture.sa_attack_power_rate();
            if posture.state_info() != 0
                || posture.effect_endurance() != -1.0
                || posture.motion_interval() != 0.0
                || !no_links(posture)
                || !neutral_resources(posture, 1.0)
                || !neutral_damage(posture)
                || !rate.is_finite()
                || !(1.0..=2.0).contains(&rate)
                || rate == 1.0
                || posture.effect_target_enemy()
            {
                return Err(format!(
                    "owned critical posture row {posture_id} is incompatible"
                ));
            }
            ranks.push(RankSpec {
                carrier: carrier_id,
                posture: posture_id,
                posture_rate: rate,
            });
        }
        self.ranks = ranks;
        self.ready = true;
        Ok(())
    }

    /// Maintain a neutral state483 carrier; follow only actual live native
    /// blessing receipt. No roll event or inferred critical event is produced.
    /// # Safety
    /// Gated offline native game task only. No game references escape or I/O.
    pub unsafe fn reconcile(&mut self, posture_rate: f32) -> Result<CriticalBuffSummary, String> {
        let game = unsafe { GameMan::instance() }
            .map_err(|e| format!("critical blessing game state unavailable: {e}"))?;
        if !crate::game::actual_offline(game) {
            unsafe {
                self.clear()?;
            }
            return Err("Merciful Reversal requires offline mode".into());
        }
        if game.warp_requested || game.new_game_plus_requested {
            return Ok(CriticalBuffSummary::default());
        }
        let save_slot = game.save_slot;
        if !posture_rate.is_finite() {
            unsafe {
                self.clear()?;
            }
            return Err("critical posture rate must be finite".into());
        }
        if posture_rate <= 1.0 {
            unsafe {
                self.clear()?;
            }
            return Ok(CriticalBuffSummary::default());
        }
        if let Err(error) = unsafe { self.ensure_ready() } {
            unsafe {
                self.clear()?;
            }
            return Err(error);
        }
        let Some(index) = self
            .ranks
            .iter()
            .position(|rank| close(rank.posture_rate, posture_rate))
        else {
            unsafe {
                self.clear()?;
            }
            return Err(
                "Merciful Reversal definition differs from generated posture rows; rebuild params"
                    .into(),
            );
        };
        let spec = self.ranks[index];
        let level = index as u8 + 1;
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("critical blessing world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.forget();
            return Ok(CriticalBuffSummary::default());
        };
        let data = unsafe { player.player_game_data.as_ref() };
        if player.chr_ins.modules.data.hp <= 0
            || !unsafe { crate::game::native_offline_host(game, data) }
        {
            remove_owned(&mut player.chr_ins);
            self.forget();
            return Ok(CriticalBuffSummary::default());
        }
        let key = OwnerKey {
            handle: player.chr_ins.field_ins_handle,
            save_slot,
            player_data_address: data as *const _ as usize,
        };
        let mut live: BTreeSet<u32> = player
            .chr_ins
            .special_effect
            .entries()
            .filter_map(|entry| u32::try_from(entry.param_id).ok())
            .collect();
        let remaining = player
            .chr_ins
            .special_effect
            .entries()
            .filter(|entry| entry.param_id == NATIVE_BLESSING as i32)
            .map(|entry| entry.removal_timer)
            .filter(|time| time.is_finite() && *time > 0.0)
            .max_by(f32::total_cmp);
        if self.owner.as_ref() != Some(&key) || self.level != level {
            remove_owned(&mut player.chr_ins);
            for index in 0..RANKS as u32 {
                live.remove(&(CRITICAL_CARRIER_BASE + index));
                live.remove(&(CRITICAL_POSTURE_BASE + index));
            }
            self.owner = Some(key);
            self.level = level;
        }
        for other in &self.ranks {
            if other.carrier != spec.carrier && live.contains(&other.carrier) {
                player.chr_ins.remove_speffect(other.carrier as i32);
            }
            if other.posture != spec.posture && live.contains(&other.posture) {
                player.chr_ins.remove_speffect(other.posture as i32);
            }
        }
        if !live.contains(&spec.carrier) {
            player.apply_speffect(spec.carrier as i32, true);
        }
        if remaining.is_some() {
            if !live.contains(&spec.posture) {
                player.apply_speffect(spec.posture as i32, true);
            }
        } else if live.contains(&spec.posture) {
            player.chr_ins.remove_speffect(spec.posture as i32);
        }
        Ok(CriticalBuffSummary {
            level,
            carrier_row: Some(spec.carrier),
            posture_row: Some(spec.posture),
            native_blessing_active: remaining.is_some(),
            native_blessing_remaining_seconds: remaining,
            // Application is a request; report receipt only on a later live
            // observation rather than claiming the void native call succeeded.
            posture_active: remaining.is_some() && live.contains(&spec.posture),
            warnings: Vec::new(),
        })
    }

    /// Remove only our carriers/posture followups. A native Blade blessing
    /// already earned expires in the engine, including when this card is lost.
    /// # Safety
    /// Native callback only, after executable and regulation compatibility gates.
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

fn valid_carrier(row: &SP_EFFECT_PARAM_ST, expected_soul_rate: f32) -> bool {
    row.state_info() == 483
        && row.effect_endurance() == -1.0
        && row.motion_interval() == 0.0
        && no_links(row)
        && neutral_resources(row, expected_soul_rate)
        && neutral_damage(row)
        && row.sa_attack_power_rate() == 1.0
        && !row.effect_target_enemy()
}

fn neutral_resources(row: &SP_EFFECT_PARAM_ST, expected_soul_rate: f32) -> bool {
    row.change_hp_rate() == 0.0
        && row.change_hp_point() == 0
        && row.change_mp_rate() == 0.0
        && row.change_mp_point() == 0
        && row.change_stamina_rate() == 0.0
        && row.change_stamina_point() == 0
        // Both native and owned neutral rows preserve rune income at one.
        // Zero is multiplicative suppression, not an unused-field sentinel.
        && row.soul_rate() == expected_soul_rate
        && row.apply_id_on_get_soul() == 0
}

fn no_links(row: &SP_EFFECT_PARAM_ST) -> bool {
    row.replace_sp_effect_id() == -1
        && row.cycle_occurrence_sp_effect_id() == -1
        && row.atk_occurrence_sp_effect_id() == -1
        && row.accumu_val() == 0
        && row.accumu_over_fire_id() == -1
        && row.accumu_under_fire_id() == -1
        && row.invocation_conditions_state_change1() == 0
        && row.invocation_conditions_state_change2() == 0
        && row.invocation_conditions_state_change3() == 0
}

fn base_attack_rates(row: &SP_EFFECT_PARAM_ST) -> [f32; 10] {
    [
        row.physics_attack_rate(),
        row.magic_attack_rate(),
        row.fire_attack_rate(),
        row.thunder_attack_rate(),
        row.dark_attack_rate(),
        row.physics_attack_power_rate(),
        row.magic_attack_power_rate(),
        row.fire_attack_power_rate(),
        row.thunder_attack_power_rate(),
        row.dark_attack_power_rate(),
    ]
}

fn enemy_rates(row: &SP_EFFECT_PARAM_ST) -> [f32; 5] {
    [
        row.atk_enemy_dmg_correct_rate_physics(),
        row.atk_enemy_dmg_correct_rate_magic(),
        row.atk_enemy_dmg_correct_rate_fire(),
        row.atk_enemy_dmg_correct_rate_thunder(),
        row.atk_enemy_dmg_correct_rate_dark(),
    ]
}

fn player_rates(row: &SP_EFFECT_PARAM_ST) -> [f32; 5] {
    [
        row.atk_player_dmg_correct_rate_physics(),
        row.atk_player_dmg_correct_rate_magic(),
        row.atk_player_dmg_correct_rate_fire(),
        row.atk_player_dmg_correct_rate_thunder(),
        row.atk_player_dmg_correct_rate_dark(),
    ]
}

fn neutral_damage(row: &SP_EFFECT_PARAM_ST) -> bool {
    base_attack_rates(row)
        .into_iter()
        .chain(enemy_rates(row))
        .chain(player_rates(row))
        .all(|rate| rate == 1.0)
}

fn close(left: f32, right: f32) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= 0.0001
}

fn remove_owned(player: &mut eldenring::cs::ChrIns) {
    for index in 0..RANKS as u32 {
        player.remove_speffect((CRITICAL_CARRIER_BASE + index) as i32);
        player.remove_speffect((CRITICAL_POSTURE_BASE + index) as i32);
    }
}
