//! Divine intervention. While armed, an owned SpEffect with the engine's
//! `noDead` flag keeps a lethal blow at 1 HP; the backend then restores part of
//! max HP, wards the character for a moment (Golden Vow aura) and starts the
//! cooldown. Scripted deaths (kill planes, falls into the abyss) still kill.
//! Native game task only; rows are rewritten from the neutral template.
use crate::powers::InterventionSpec;
use eldenring::cs::{ChrInsExt, FieldInsHandle, SoloParamRepository, SpEffectParam, WorldChrMan};
use fromsoftware_shared::FromStatic;
use std::time::Instant;

pub const NO_DEAD_ROW: u32 = 95_050_000;
pub const WARD_ROW: u32 = 95_050_001;
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;
/// Golden Vow's aura (incantation SpEffect 1660000).
const WARD_VFX: i32 = 1_660_000;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct InterventionSummary {
    pub armed: bool,
    pub cooldown_left: f32,
    pub triggered: bool,
}

#[derive(Default)]
pub struct Intervention {
    ready: bool,
    owner: Option<FieldInsHandle>,
    applied: bool,
    last_trigger: Option<Instant>,
    cooldown: f32,
    pub saves: u32,
}

impl Intervention {
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// # Safety
    /// Native game task, generated package loaded.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        let repo = unsafe { SoloParamRepository::instance() }
            .map_err(|e| format!("intervention repository unavailable: {e}"))?;
        for id in [NEUTRAL_TEMPLATE_ROW, NO_DEAD_ROW, WARD_ROW] {
            if repo.get::<SpEffectParam>(id).is_none() {
                self.ready = false;
                return Err(format!("intervention row {id} missing; regenerate the package"));
            }
        }
        self.ready = true;
        Ok(())
    }

    /// Keep the save armed while `spec` is owned and off cooldown; resolve a
    /// prevented death. `spec` is the combined spec of all active sources.
    ///
    /// # Safety
    /// Native game task, verified offline local host.
    pub unsafe fn reconcile(
        &mut self,
        spec: Option<InterventionSpec>,
        now: Instant,
    ) -> Result<InterventionSummary, String> {
        let mut summary = InterventionSummary::default();
        if !self.ready {
            return Ok(summary);
        }
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("intervention world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.applied = false;
            self.owner = None;
            return Ok(summary);
        };
        let handle = player.chr_ins.field_ins_handle;
        if self.owner != Some(handle) {
            player.chr_ins.remove_speffect(NO_DEAD_ROW as i32);
            self.applied = false;
            self.owner = Some(handle);
        }
        let data = &mut player.chr_ins.modules.data;
        let (hp, max_hp) = (data.hp, data.max_hp);
        if hp <= 0 || max_hp <= 1 {
            if self.applied {
                player.chr_ins.remove_speffect(NO_DEAD_ROW as i32);
                self.applied = false;
            }
            return Ok(summary);
        }
        if let Some(spec) = spec {
            self.cooldown = spec.cooldown;
        }
        let since = self
            .last_trigger
            .map(|t| now.duration_since(t).as_secs_f32());
        let grace = spec.is_some_and(|s| since.is_some_and(|t| t < s.ward_seconds));
        let cooling = since.is_some_and(|t| t < self.cooldown);
        summary.cooldown_left = if cooling {
            self.cooldown - since.unwrap_or(0.0)
        } else {
            0.0
        };

        // A blow the flag held at 1 HP: the save resolves now.
        if self.applied && hp <= 1 && !cooling {
            if let Some(spec) = spec {
                let restored = ((max_hp as f32 * spec.heal_share).round() as i32).clamp(1, max_hp);
                player.chr_ins.modules.data.hp = restored;
                unsafe { write_ward(spec) }?;
                player.chr_ins.remove_speffect(WARD_ROW as i32);
                player.apply_speffect(WARD_ROW as i32, true);
                self.last_trigger = Some(now);
                self.saves = self.saves.saturating_add(1);
                summary.triggered = true;
                summary.cooldown_left = spec.cooldown;
                // The flag stays through the ward so the follow-up hit of the
                // same combo cannot finish the character either.
                return Ok(summary);
            }
        }

        let want = spec.is_some() && (!cooling || grace);
        if want && !self.applied {
            unsafe { write_no_dead() }?;
            player.apply_speffect(NO_DEAD_ROW as i32, true);
            self.applied = true;
        } else if !want && self.applied {
            player.chr_ins.remove_speffect(NO_DEAD_ROW as i32);
            self.applied = false;
        }
        summary.armed = self.applied && !cooling;
        Ok(summary)
    }

    /// # Safety
    /// Native game task.
    pub unsafe fn clear(&mut self) {
        if let Ok(world) = unsafe { WorldChrMan::instance_mut() } {
            if let Some(player) = world.main_player.as_mut() {
                player.chr_ins.remove_speffect(NO_DEAD_ROW as i32);
            }
        }
        self.applied = false;
        self.owner = None;
    }

    pub fn cooldown_left(&self, now: Instant) -> f32 {
        self.last_trigger
            .map(|t| (self.cooldown - now.duration_since(t).as_secs_f32()).max(0.0))
            .unwrap_or(0.0)
    }
}

unsafe fn neutral_into(id: u32) -> Result<&'static mut eldenring::param::SP_EFFECT_PARAM_ST, String> {
    let repo = unsafe { SoloParamRepository::instance_mut() }
        .map_err(|e| format!("intervention repository unavailable: {e}"))?;
    let neutral = repo
        .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
        .ok_or("neutral template missing")?
        .clone();
    let row = repo
        .get_mut::<SpEffectParam>(id)
        .ok_or_else(|| format!("intervention row {id} missing"))?;
    *row = neutral;
    row.set_sp_category(0);
    row.set_save_category(-1);
    row.set_state_info(0);
    Ok(row)
}

unsafe fn write_no_dead() -> Result<(), String> {
    let row = unsafe { neutral_into(NO_DEAD_ROW) }?;
    row.set_effect_endurance(-1.0);
    row.set_no_dead(true);
    Ok(())
}

unsafe fn write_ward(spec: InterventionSpec) -> Result<(), String> {
    let row = unsafe { neutral_into(WARD_ROW) }?;
    row.set_effect_endurance(spec.ward_seconds);
    row.set_vfx_id(WARD_VFX);
    let cut = spec.ward_cut.clamp(0.05, 1.0);
    row.set_slash_damage_cut_rate(cut);
    row.set_blow_damage_cut_rate(cut);
    row.set_thrust_damage_cut_rate(cut);
    row.set_neutral_damage_cut_rate(cut);
    row.set_magic_damage_cut_rate(cut);
    row.set_fire_damage_cut_rate(cut);
    row.set_thunder_damage_cut_rate(cut);
    row.set_dark_damage_cut_rate(cut);
    Ok(())
}
