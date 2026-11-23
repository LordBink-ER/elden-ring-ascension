//! 0.8: charged attacks, jumping attacks and criticals (Ferocity and cards)
//! through owned copies of the game's own talisman effects, so the engine
//! applies them only to those attacks exactly like the talismans do:
//! - 95_050_060 copy of the Axe Talisman (charged attacks, attack rates),
//! - 95_050_061 copy of the Claw Talisman (jump attacks, PvE damage correction),
//! - 95_050_062 copy of the Dagger Talisman (critical hits, attack rates).
//! Values are written at runtime and the row is reapplied when they change.
//! Native game task only; source-derived, not verified in game.
use eldenring::cs::{ChrInsExt, SoloParamRepository, SpEffectParam, WorldChrMan};
use fromsoftware_shared::FromStatic;

pub const ROWS: [u32; 3] = [95_050_060, 95_050_061, 95_050_062];

#[derive(Default)]
pub struct Arts {
    written: [Option<f32>; 3],
}

impl Arts {
    /// `bonus`: [charged, jump, critical] damage bonus (0.25 = +25%).
    ///
    /// # Safety
    /// Native game task, offline, generated params loaded.
    pub unsafe fn reconcile(&mut self, bonus: [f32; 3]) -> Result<(), String> {
        let repo = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let Some(player) = world.main_player.as_mut() else {
            self.written = [None; 3];
            return Ok(());
        };
        if player.chr_ins.modules.data.hp <= 0 {
            self.written = [None; 3];
            return Ok(());
        }
        let live: Vec<i32> = player.chr_ins.special_effect.entries().map(|e| e.param_id).collect();
        for (index, id) in ROWS.iter().enumerate() {
            let value = if bonus[index].is_finite() { bonus[index].clamp(0.0, 1.5) } else { 0.0 };
            let present = live.contains(&(*id as i32));
            if value < 0.005 {
                if present {
                    player.chr_ins.remove_speffect(*id as i32);
                }
                self.written[index] = None;
                continue;
            }
            let changed = self.written[index].is_none_or(|w| (w - value).abs() > 1e-4);
            if changed {
                let row = repo
                    .get_mut::<SpEffectParam>(*id)
                    .ok_or_else(|| format!("owned talisman copy {id} is missing"))?;
                let rate = 1.0 + value;
                if index == 1 {
                    row.set_atk_enemy_dmg_correct_rate_physics(rate);
                    row.set_atk_enemy_dmg_correct_rate_magic(rate);
                    row.set_atk_enemy_dmg_correct_rate_fire(rate);
                    row.set_atk_enemy_dmg_correct_rate_thunder(rate);
                    row.set_atk_enemy_dmg_correct_rate_dark(rate);
                } else {
                    row.set_physics_attack_rate(rate);
                    row.set_magic_attack_rate(rate);
                    row.set_fire_attack_rate(rate);
                    row.set_thunder_attack_rate(rate);
                    row.set_dark_attack_rate(rate);
                }
                self.written[index] = Some(value);
            }
            if changed || !present {
                if present {
                    player.chr_ins.remove_speffect(*id as i32);
                }
                player.apply_speffect(*id as i32, true);
            }
        }
        Ok(())
    }

    /// # Safety
    /// Native game task.
    pub unsafe fn clear(&mut self) {
        if let Ok(world) = unsafe { WorldChrMan::instance_mut() } {
            if let Some(player) = world.main_player.as_mut() {
                for id in ROWS {
                    player.chr_ins.remove_speffect(id as i32);
                }
            }
        }
        self.written = [None; 3];
    }
}
