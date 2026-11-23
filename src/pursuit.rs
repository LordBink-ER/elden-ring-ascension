//! 3.2/3.3 No Quarter: the victim of your critical can be hit while it gets
//! back up, like any other enemy.
//!
//! After a riposte, a backstab or a stance-break critical the game makes the
//! victim invincible until it stands up. While No Quarter's window is open
//! (it counts from the moment your critical ends; the card's last levels
//! cover the whole time), every frame, after the animations set it and right
//! before the game processes damage, the victim's invincibility (the TAE
//! flags and the event bit, the same ones roll.rs grants the player) is
//! cleared. Then everything lands the normal way: your weapon with its
//! buffs, its status buildup and your cards' and attributes' bonuses, and
//! your card spells.
//!
//! In case the game still refuses those hits (not verified in game), the
//! mod checks every swing: a swing at the victim within reach that brings no
//! real hit within 0.45 s is delivered by the mod (the damage of your usual
//! hit on that foe, plus the card's bonus), and card spells aimed at it then
//! deliver their damage the same way when they would land. All of that goes
//! through an owned instant SpEffect (row 95_020_014, changeHpPoint) that
//! invincibility does not block. The card's bonus (more damage only in this
//! state) is in the mod's strikes, and for everything else it is a
//! damage-taken row (95_020_013) kept on the victim while the window is open.
//!
//! Native game task only; source-derived, not verified in game.
use crate::{powers::DownedStrike, world::WorldScan};
use eldenring::{
    cs::{CSTaskGroupIndex, CSTaskImp, ChrInsExt, FieldInsHandle, SoloParamRepository, SpEffectParam, WorldChrMan},
    fd4::FD4TaskData,
};
use fromsoftware_shared::{FromStatic, RecurringTaskHandle, SharedTaskImpExt};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const STRIKE_ROW: u32 = 95_020_014;
/// While the window is open the victim takes the card's bonus as damage
/// taken (its cut rates), so your native hits get it too.
pub const BONUS_ROW: u32 = 95_020_013;
const NEUTRAL_TEMPLATE_ROW: u32 = 95_010_000;
/// When a swing connects after its start (a typical active frame), and how
/// long a real hit may take before the mod delivers it.
const SWING_DELAY: Duration = Duration::from_millis(450);
const MIN_GAP: Duration = Duration::from_millis(400);
/// Weapon reach from the foe's centre, plus its size.
const REACH: f32 = 3.2;
const BOSS_SIZE: f32 = 2.0;
const FOE_SIZE: f32 = 0.6;
/// Card spell damage before the foe's defences (a rough average cut).
const SPELL_DEFENCE: f32 = 0.7;

/// The victim whose invincibility the per-frame tasks clear, until when.
static UNGUARD: Mutex<Option<(FieldInsHandle, Instant)>> = Mutex::new(None);

pub struct UnguardTasks {
    _behavior: RecurringTaskHandle<FD4TaskData>,
    _damage: RecurringTaskHandle<FD4TaskData>,
}

/// Clear the TAE / event invincibility of the No Quarter victim.
fn unguard() {
    let Ok(target) = UNGUARD.try_lock() else {
        return;
    };
    let Some((handle, until)) = *target else {
        return;
    };
    drop(target);
    if Instant::now() >= until {
        return;
    }
    let Ok(world) = (unsafe { WorldChrMan::instance_mut() }) else {
        return;
    };
    let Some(chr) = world.chr_ins_by_handle_mut(&handle) else {
        return;
    };
    if chr.modules.data.hp <= 0 || chr.chr_flags1c5.death_flag() {
        return;
    }
    let flags = &mut chr.modules.action_flag.action_modifiers_flags;
    flags.set_perfect_invincibility(false);
    flags.set_invincible_excluding_throw_attacks_defender(false);
    flags.set_pve_only_iframes(false);
    chr.modules.event.flags &= !2;
}

/// Called by bootstrap; the tasks live until the process exits. They run
/// where roll.rs writes the player's own dodge invincibility (after the
/// behaviour update, before damage shapes) and again right before the
/// damage manager.
pub fn install_unguard() -> Result<UnguardTasks, String> {
    let runtime = CSTaskImp::wait_for_instance(Duration::from_secs(30)).map_err(|e| e.to_string())?;
    let behavior = runtime.run_recurring(|_: &FD4TaskData| unguard(), CSTaskGroupIndex::HavokAi_SilhouetteGeneratorHelper_End);
    let damage = runtime.run_recurring(|_: &FD4TaskData| unguard(), CSTaskGroupIndex::DmgMan_Pre);
    Ok(UnguardTasks {
        _behavior: behavior,
        _damage: damage,
    })
}

#[derive(Default)]
pub struct Pursuit {
    installed: bool,
    last_anim: i32,
    /// A swing at the victim: (foe, when it started).
    pending: Option<(FieldInsHandle, Instant)>,
    last_strike: Option<Instant>,
    /// Your last real hit on the victim (not ours).
    last_real_hit: Option<(FieldInsHandle, Instant)>,
    /// Did real hits land in the window? (latest last)
    native: VecDeque<bool>,
    /// Spell damage to deliver: (foe, damage, when).
    queued: Vec<(FieldInsHandle, f32, Instant)>,
    /// This tick's open window: (victim, real hits land).
    window: Option<(FieldInsHandle, bool)>,
    bonus_on: Option<FieldInsHandle>,
    pub strikes: u32,
    noted: u32,
    native_noted: Option<bool>,
}

impl Pursuit {
    pub fn forget_world(&mut self) {
        self.installed = false;
        self.pending = None;
        self.queued.clear();
        self.window = None;
        self.bonus_on = None;
        if let Ok(mut target) = UNGUARD.lock() {
            *target = None;
        }
    }

    /// The open window this tick: the victim, and whether your real hits
    /// land on it (then spells are simply cast at it).
    pub fn window(&self) -> Option<(FieldInsHandle, bool)> {
        self.window
    }

    /// Card spell damage for the victim, delivered when the spell lands
    /// (only while real hits do not land).
    pub fn queue(&mut self, handle: FieldInsHandle, damage: f32, at: Instant) {
        if self.queued.len() < 32 && damage.is_finite() && damage > 0.0 {
            self.queued.push((handle, damage * SPELL_DEFENCE, at));
        }
    }

    fn native_works(&self) -> bool {
        let yes = self.native.iter().filter(|v| **v).count();
        yes > 0 && yes * 2 >= self.native.len()
    }

    unsafe fn install(&mut self) -> Result<(), String> {
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let neutral = params
            .get::<SpEffectParam>(NEUTRAL_TEMPLATE_ROW)
            .ok_or("neutral template missing")?
            .clone();
        for id in [STRIKE_ROW, BONUS_ROW] {
            let row = params
                .get_mut::<SpEffectParam>(id)
                .ok_or("No Quarter row missing; regenerate the package")?;
            *row = neutral.clone();
            row.set_sp_category(0);
            row.set_save_category(-1);
            row.set_state_info(0);
            row.set_effect_target_enemy(true);
            row.set_effect_target_ai(true);
            row.set_effect_target_player(false);
            if id == STRIKE_ROW {
                row.set_effect_endurance(0.0);
                row.set_motion_interval(0.0);
                row.set_change_hp_point(0);
            } else {
                // Renewed when it runs out while the window is open.
                row.set_effect_endurance(1.0);
            }
        }
        self.installed = true;
        Ok(())
    }

    /// One tick. Returns the foe a mod strike landed on (for the reflex
    /// tracker), and log notes.
    ///
    /// # Safety
    /// Native game task, offline, generated params loaded.
    pub unsafe fn tick(
        &mut self,
        scan: &WorldScan,
        spec: Option<DownedStrike>,
        now: Instant,
        notes: &mut Vec<String>,
    ) -> Result<Option<FieldInsHandle>, String> {
        let anim = scan.player_state.anim;
        let swung = anim != self.last_anim
            && crate::haste::is_attack_anim(anim)
            && !scan.player_state.in_critical();
        self.last_anim = anim;
        let spec = spec.filter(|_| scan.ready);
        // The victim, if it is down and inside the card's window.
        let in_window = |handle: FieldInsHandle| {
            spec.is_some_and(|spec| {
                scan.reflex.downed.is_some_and(|d| {
                    d.handle == handle && d.since_throw.is_some_and(|t| spec.window >= 10.0 || t <= spec.window)
                })
            })
        };
        let victim = scan.reflex.downed.map(|d| d.handle).filter(|h| in_window(*h));
        // Real hits of yours on it (ours come within 0.5 s of a strike).
        if let Some(handle) = victim {
            let ours = self.last_strike.is_some_and(|at| now.duration_since(at) <= Duration::from_millis(500));
            if scan.hits > 0 && scan.last_hit_target == Some(handle) && !ours {
                self.last_real_hit = Some((handle, now));
            }
        }
        self.window = victim.map(|h| (h, self.native_works()));
        if let Ok(mut target) = UNGUARD.lock() {
            *target = victim.map(|h| (h, now + Duration::from_millis(250)));
        }
        let Some(spec) = spec else {
            self.pending = None;
            self.queued.clear();
            return Ok(None);
        };
        if !self.installed {
            unsafe { self.install() }?;
        }
        let params = unsafe { SoloParamRepository::instance_mut() }.map_err(|e| e.to_string())?;
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        // The card's bonus as damage taken on the victim while the window is
        // open (native hits and spells get it too).
        if let Some(handle) = victim.filter(|_| spec.bonus > 0.0) {
            if self.bonus_on != Some(handle) {
                if let Some(row) = params.get_mut::<SpEffectParam>(BONUS_ROW) {
                    let cut = 1.0 + spec.bonus.clamp(0.0, 1.0);
                    row.set_slash_damage_cut_rate(cut);
                    row.set_blow_damage_cut_rate(cut);
                    row.set_thrust_damage_cut_rate(cut);
                    row.set_neutral_damage_cut_rate(cut);
                    row.set_magic_damage_cut_rate(cut);
                    row.set_fire_damage_cut_rate(cut);
                    row.set_thunder_damage_cut_rate(cut);
                    row.set_dark_damage_cut_rate(cut);
                }
                self.bonus_on = Some(handle);
            }
            // Only when it is not on the foe already (never stacked).
            if let Some(actor) = world.chr_ins_by_handle_mut(&handle) {
                if !actor.special_effect.entries().any(|e| e.param_id == BONUS_ROW as i32) {
                    actor.apply_speffect(BONUS_ROW as i32, true);
                }
            }
        } else {
            self.bonus_on = None;
        }
        let in_reach = |handle: FieldInsHandle, slack: f32| {
            scan.foes.iter().find(|f| f.handle == handle).filter(|f| {
                let size = if f.boss { BOSS_SIZE } else { FOE_SIZE };
                let d = [f.position[0] - scan.player_position[0], f.position[2] - scan.player_position[2]];
                let flat = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
                let facing = (scan.player_forward[0] * d[0] + scan.player_forward[2] * d[1]) / flat;
                f.distance <= REACH + size + slack && facing >= 0.35 - slack * 0.2
            })
        };
        if swung && self.pending.is_none() {
            if let Some(handle) = victim {
                if in_reach(handle, 0.0).is_some() {
                    self.pending = Some((handle, now));
                }
            }
        }
        let mut damage_now: Vec<(FieldInsHandle, f32, &'static str)> = Vec::new();
        if let Some((handle, swing)) = self.pending {
            if now.duration_since(swing) >= SWING_DELAY {
                self.pending = None;
                let landed = self.last_real_hit.is_some_and(|(h, at)| h == handle && at >= swing);
                self.native.push_back(landed);
                if self.native.len() > 4 {
                    self.native.pop_front();
                }
                let works = self.native_works();
                if self.native_noted != Some(works) && self.native.len() >= 2 {
                    self.native_noted = Some(works);
                    notes.push(if works {
                        "No Quarter: your real hits land on foes getting up (invincibility cleared)".to_owned()
                    } else {
                        "No Quarter: the game still refuses hits on foes getting up; the mod delivers them".to_owned()
                    });
                }
                if !landed && self.last_strike.is_none_or(|at| now.duration_since(at) >= MIN_GAP) {
                    // Still in the window and close enough (the swing started).
                    if let Some(foe) = in_reach(handle, 1.0).filter(|_| in_window(handle)) {
                        let usual = if foe.typical_hit > 0 { foe.typical_hit } else { scan.typical_hit };
                        if usual > 0 && foe.hp > 0 {
                            let damage = (usual as f32 * (1.0 + spec.bonus.clamp(0.0, 1.0))).min(foe.max_hp as f32 * 0.2);
                            damage_now.push((handle, damage, "blow"));
                        }
                    }
                }
            }
        }
        // Card spells that reached the victim while real hits do not land.
        let mut spells = 0.0f32;
        let mut spell_target = None;
        self.queued.retain(|(handle, damage, at)| {
            if now < *at {
                return true;
            }
            if in_window(*handle) {
                spells += damage * (1.0 + spec.bonus.clamp(0.0, 1.0));
                spell_target = Some(*handle);
            }
            false
        });
        if let Some(handle) = spell_target {
            if let Some(foe) = scan.foes.iter().find(|f| f.handle == handle) {
                damage_now.push((handle, spells.min(foe.max_hp as f32 * 0.2), "spells"));
            }
        }
        // One strike per tick (the row is shared): the sum for the victim.
        let Some(&(handle, _, _)) = damage_now.first() else {
            return Ok(None);
        };
        let total: f32 = damage_now.iter().filter(|(h, _, _)| *h == handle).map(|(_, d, _)| *d).sum();
        let what: Vec<&str> = damage_now.iter().map(|(_, _, w)| *w).collect();
        let damage = total.round().max(1.0) as i32;
        let row = params
            .get_mut::<SpEffectParam>(STRIKE_ROW)
            .ok_or("No Quarter row missing; regenerate the package")?;
        // Positive changeHpPoint takes HP (negative heals, as on player rows).
        row.set_change_hp_point(damage);
        let Some(actor) = world.chr_ins_by_handle_mut(&handle) else {
            return Ok(None);
        };
        actor.apply_speffect(STRIKE_ROW as i32, true);
        self.last_strike = Some(now);
        self.strikes = self.strikes.saturating_add(1);
        if self.noted < 6 {
            self.noted += 1;
            notes.push(format!(
                "No Quarter: delivered {damage} damage ({}) to a foe getting up (+{:.0}%)",
                what.join(" + "),
                spec.bonus * 100.0
            ));
        }
        Ok(Some(handle))
    }
}
