//! 0.8 The world grows with you.
//!
//! - Every regular enemy gets the row of the current Tide once, when first
//!   seen: more HP, damage and poise ([world]; rows 95_050_070 + Tide 0..10).
//!   Bosses keep the boss scaler's tiers. Runes are on the player (paths.rs).
//! - Arcane marks enemies you fight: blood loss, frostbite, sleep and madness
//!   hit them harder (row 95_050_081, the game's own status damage rates).
//! - Malice path 5, Lingering Curse: an enemy whose status just popped takes 10%
//!   more damage for 8 s (row 95_050_082).
//! - Malice path 10, Plague Bearer: a status that pops spreads half of the enemy's
//!   gauge as buildup to enemies within 5 m (status payload copies
//!   95_050_048.., the 0.7.1 surge rows); their resistance still counts.
//!
//! The rows are owned copies of the neutral template; values are written here.
//! Native game task only; source-derived, not verified in game.
use crate::{
    config::WorldSettings,
    powers::STATUS_KINDS,
    paths::Boons,
    world::WorldScan,
};
use eldenring::cs::{ChrInsExt, FieldInsHandle, SoloParamRepository, SpEffectParam, WorldChrMan};
use fromsoftware_shared::FromStatic;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const TIDE_BASE: u32 = 95_050_070;
pub const MALICE_MARK: u32 = 95_050_081;
pub const CURSE_MARK: u32 = 95_050_082;
/// Status payload copies in resist-module order (0.7.1 rows).
const PAYLOAD_BASE: u32 = 95_050_048;
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;
const PLAGUE_RADIUS: f32 = 5.0;
const PLAGUE_TARGETS: usize = 4;

pub struct Tide {
    settings: WorldSettings,
    installed: bool,
    /// Regular enemies with a Tide row: handle -> last seen.
    applied: HashMap<FieldInsHandle, Instant>,
    marked: HashMap<FieldInsHandle, Instant>,
    malice_written: Option<u8>,
    pub spread_total: u32,
    noted: u32,
}

impl Tide {
    pub fn new(settings: WorldSettings) -> Self {
        Self {
            settings,
            installed: false,
            applied: HashMap::new(),
            marked: HashMap::new(),
            malice_written: None,
            spread_total: 0,
            noted: 0,
        }
    }
    pub fn forget_world(&mut self) {
        self.applied.clear();
        self.marked.clear();
    }

    unsafe fn install(&mut self) -> Result<(), String> {
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let neutral = params
            .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
            .ok_or("neutral template missing")?
            .clone();
        let s = self.settings;
        let enemy_row = |row: &mut eldenring::param::SP_EFFECT_PARAM_ST| {
            *row = neutral.clone();
            row.set_effect_endurance(-1.0);
            row.set_sp_category(0);
            row.set_save_category(-1);
            row.set_state_info(0);
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_player(false);
            row.set_b_curr_hp_independe_max_hp(false);
        };
        for tide in 0..=10u32 {
            let row = params
                .get_mut::<SpEffectParam>(TIDE_BASE + tide)
                .ok_or("Tide row missing; regenerate the package")?;
            enemy_row(row);
            let t = tide as f32;
            let hp = (s.enemy_hp * (1.0 + s.tide_hp * t)).clamp(0.5, 5.0);
            let damage = (s.enemy_damage * (1.0 + s.tide_damage * t)).clamp(0.5, 4.0);
            let poise = (s.enemy_poise * (1.0 + s.tide_poise * t)).clamp(0.5, 4.0);
            row.set_max_hp_rate(hp);
            row.set_physics_attack_rate(damage);
            row.set_magic_attack_rate(damage);
            row.set_fire_attack_rate(damage);
            row.set_thunder_attack_rate(damage);
            row.set_dark_attack_rate(damage);
            row.set_toughness_damage_cut_rate((1.0 / poise).clamp(0.25, 2.0));
        }
        let mark = params
            .get_mut::<SpEffectParam>(MALICE_MARK)
            .ok_or("Malice mark row missing; regenerate the package")?;
        enemy_row(mark);
        let curse = params
            .get_mut::<SpEffectParam>(CURSE_MARK)
            .ok_or("Lingering Curse row missing; regenerate the package")?;
        enemy_row(curse);
        curse.set_effect_endurance(8.0);
        curse.set_slash_damage_cut_rate(1.1);
        curse.set_blow_damage_cut_rate(1.1);
        curse.set_thrust_damage_cut_rate(1.1);
        curse.set_neutral_damage_cut_rate(1.1);
        curse.set_magic_damage_cut_rate(1.1);
        curse.set_fire_damage_cut_rate(1.1);
        curse.set_thunder_damage_cut_rate(1.1);
        curse.set_dark_damage_cut_rate(1.1);
        self.installed = true;
        Ok(())
    }

    /// # Safety
    /// Native game task, offline, generated params loaded.
    pub unsafe fn tick(&mut self, scan: &WorldScan, tide: u8, effects: &Boons, now: Instant) -> Result<Vec<String>, String> {
        let mut notes = Vec::new();
        if !scan.ready {
            return Ok(notes);
        }
        if !self.installed {
            unsafe { self.install() }?;
        }
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        // Status mark: status damage on enemies you fight (percent, the
        // game's unit), raised by Arcane.
        let percent = (100.0 * (1.0 + effects.status_damage.clamp(0.0, 1.5))).round() as u8;
        if self.malice_written != Some(percent) {
            if let Some(row) = params.get_mut::<SpEffectParam>(MALICE_MARK) {
                row.set_blood_damage_rate(percent);
                row.set_freeze_damage_rate(percent);
                row.set_sleep_damage_rate(percent);
                row.set_madness_damage_rate(percent);
            }
            // New values reach enemies marked from now on.
            self.marked.clear();
            self.malice_written = Some(percent);
        }
        let row = TIDE_BASE + u32::from(tide.min(10));
        for foe in &scan.foes {
            if foe.protected || foe.hp <= 0 {
                continue;
            }
            if self.settings.enabled && !foe.boss && !self.applied.contains_key(&foe.handle) {
                if let Some(actor) = world.chr_ins_by_handle_mut(&foe.handle) {
                    // 3.4: never a second Tide row (one from an older Tide
                    // would stack: HP x1.3 twice).
                    let tide_rows = TIDE_BASE as i32..=(TIDE_BASE + 10) as i32;
                    if !actor.special_effect.entries().any(|e| tide_rows.contains(&e.param_id)) {
                        actor.apply_speffect(row as i32, true);
                    }
                }
                self.applied.insert(foe.handle, now);
            } else if let Some(seen) = self.applied.get_mut(&foe.handle) {
                *seen = now;
            }
            if percent > 100 && foe.engaged && !self.marked.contains_key(&foe.handle) {
                if let Some(actor) = world.chr_ins_by_handle_mut(&foe.handle) {
                    actor.apply_speffect(MALICE_MARK as i32, true);
                }
                self.marked.insert(foe.handle, now);
            }
        }
        self.applied.retain(|_, seen| now.duration_since(*seen) < Duration::from_secs(180));
        self.marked.retain(|_, at| now.duration_since(*at) < Duration::from_secs(180));
        // Status procs: Lingering Curse and Plague Bearer.
        for (kind, target) in &scan.status_procs {
            let Some(handle) = target.handle else {
                continue;
            };
            if effects.lingering_curse {
                if let Some(actor) = world.chr_ins_by_handle_mut(&handle) {
                    actor.apply_speffect(CURSE_MARK as i32, true);
                }
            }
            if !effects.plague_bearer {
                continue;
            }
            let Some(index) = STATUS_KINDS.iter().position(|k| k == kind) else {
                continue;
            };
            let gauge_max = scan
                .gauges
                .iter()
                .find(|g| g.handle == handle)
                .map(|g| g.max[index])
                .unwrap_or(400);
            let payload = (gauge_max / 2).clamp(40, 600);
            let near: Vec<FieldInsHandle> = scan
                .foes
                .iter()
                .filter(|f| f.handle != handle && !f.protected && f.hp > 0)
                .filter(|f| {
                    let d = [f.position[0] - target.position[0], f.position[1] - target.position[1], f.position[2] - target.position[2]];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= PLAGUE_RADIUS
                })
                .map(|f| f.handle)
                .take(PLAGUE_TARGETS)
                .collect();
            if near.is_empty() {
                continue;
            }
            let row_id = PAYLOAD_BASE + index as u32;
            if let Some(row) = params.get_mut::<SpEffectParam>(row_id) {
                match index {
                    0 => row.set_poizon_attack_power(payload),
                    1 => row.set_disease_attack_power(payload),
                    2 => row.set_blood_attack_power(payload),
                    3 => row.set_curse_attack_power(payload),
                    4 => row.set_freeze_attack_power(payload),
                    5 => row.set_sleep_attack_power(payload),
                    _ => row.set_madness_attack_power(payload),
                }
            }
            for other in &near {
                if let Some(actor) = world.chr_ins_by_handle_mut(other) {
                    actor.apply_speffect(row_id as i32, true);
                    self.spread_total += 1;
                }
            }
            if self.noted < 8 {
                self.noted += 1;
                notes.push(format!(
                    "Plague Bearer: {} spread to {} enemies ({payload} buildup each)",
                    kind.name(),
                    near.len()
                ));
            }
        }
        Ok(notes)
    }
}
