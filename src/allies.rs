//! 0.9 Command: your allies (spirit summons, Torrent, friendly NPCs at your
//! side) grow with the Command stat and ally cards. One owned row (95_050_090,
//! a copy of the neutral template) carries their damage, HP, damage-taken
//! reduction, poise and regeneration; it is applied once to every friendly
//! character the world scan sees, and reapplied when the values change.
//! Never applied to the player. Native game task only; source-derived, not
//! verified in game.
use crate::world::WorldScan;
use eldenring::cs::{ChrInsExt, FieldInsHandle, SoloParamRepository, SpEffectParam, WorldChrMan};
use fromsoftware_shared::FromStatic;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const ALLY_ROW: u32 = 95_050_090;
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;

/// What allies get: damage and HP bonus, damage-taken reduction (fractions),
/// regeneration (% of max HP per second) and a poise multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct AllyBoost {
    pub damage: f32,
    pub hp: f32,
    pub defense: f32,
    pub regen: f32,
    pub poise: f32,
}
impl AllyBoost {
    fn active(&self) -> bool {
        self.damage > 0.001 || self.hp > 0.001 || self.defense > 0.001 || self.regen > 0.001 || (self.poise - 1.0).abs() > 0.001
    }
}

#[derive(Default)]
pub struct Allies {
    written: Option<AllyBoost>,
    version: u32,
    applied: HashMap<FieldInsHandle, (u32, Instant)>,
    pub buffed: usize,
}

impl Allies {
    pub fn forget_world(&mut self) {
        self.applied.clear();
    }

    /// # Safety
    /// Native game task, offline, generated params loaded.
    pub unsafe fn tick(&mut self, scan: &WorldScan, boost: AllyBoost, now: Instant) -> Result<(), String> {
        if !scan.ready {
            return Ok(());
        }
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let active = boost.active();
        if active && self.written != Some(boost) {
            let neutral = params
                .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
                .ok_or("neutral template missing")?
                .clone();
            let row = params
                .get_mut::<SpEffectParam>(ALLY_ROW)
                .ok_or("ally row missing; regenerate the package")?;
            *row = neutral;
            row.set_effect_endurance(-1.0);
            row.set_sp_category(0);
            row.set_save_category(-1);
            row.set_state_info(0);
            row.set_effect_target_self(true);
            row.set_effect_target_friend(true);
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_live(true);
            row.set_effect_target_ghost(true);
            row.set_effect_target_player(false);
            row.set_b_curr_hp_independe_max_hp(false);
            let damage = 1.0 + boost.damage.clamp(0.0, 3.0);
            row.set_max_hp_rate(1.0 + boost.hp.clamp(0.0, 3.0));
            row.set_physics_attack_rate(damage);
            row.set_magic_attack_rate(damage);
            row.set_fire_attack_rate(damage);
            row.set_thunder_attack_rate(damage);
            row.set_dark_attack_rate(damage);
            let cut = 1.0 - boost.defense.clamp(0.0, 0.7);
            row.set_slash_damage_cut_rate(cut);
            row.set_blow_damage_cut_rate(cut);
            row.set_thrust_damage_cut_rate(cut);
            row.set_neutral_damage_cut_rate(cut);
            row.set_magic_damage_cut_rate(cut);
            row.set_fire_damage_cut_rate(cut);
            row.set_thunder_damage_cut_rate(cut);
            row.set_dark_damage_cut_rate(cut);
            row.set_toughness_damage_cut_rate((1.0 / boost.poise.clamp(0.5, 3.0)).clamp(0.3, 2.0));
            if boost.regen > 0.0 {
                // Negative = healing, % of max HP per motion interval.
                row.set_change_hp_rate(-boost.regen.clamp(0.0, 3.0));
                row.set_motion_interval(1.0);
            }
            self.written = Some(boost);
            self.version = self.version.wrapping_add(1);
        }
        for ally in scan.foes.iter().filter(|f| f.protected && f.hp > 0 && Some(f.handle) != scan.player) {
            let current = self.applied.get(&ally.handle).map(|(v, _)| *v);
            if !active {
                if current.is_some() {
                    if let Some(actor) = world.chr_ins_by_handle_mut(&ally.handle) {
                        actor.remove_speffect(ALLY_ROW as i32);
                    }
                    self.applied.remove(&ally.handle);
                }
                continue;
            }
            if current != Some(self.version) {
                if let Some(actor) = world.chr_ins_by_handle_mut(&ally.handle) {
                    if current.is_some() {
                        actor.remove_speffect(ALLY_ROW as i32);
                    }
                    actor.apply_speffect(ALLY_ROW as i32, true);
                }
                self.applied.insert(ally.handle, (self.version, now));
            } else if let Some(entry) = self.applied.get_mut(&ally.handle) {
                entry.1 = now;
            }
        }
        self.applied.retain(|_, (_, seen)| now.duration_since(*seen) < Duration::from_secs(120));
        self.buffed = self.applied.len();
        Ok(())
    }
}
