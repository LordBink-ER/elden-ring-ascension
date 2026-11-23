//! Gameplay input capture for Ascension's own card modal.
//!
//! Pinned fromsoftware-rs 57023153 documents ChrIns.debug_flags bits 4 and 5
//! as disabling secondary actions and movement inputs. These input-only bits
//! leave physics, damage, animation playback and raw UI XInput untouched.
//! CSChrActionRequestModule is updated in ChrIns_PreBehavior; its pending
//! input caches are discarded in the following PreBehaviorSafe group,
//! before HavokBehavior can consume them. Old button presses are never replayed.
//! Source-derived implementation; no in-game verification was performed.
use eldenring::{
    cs::{
        ActionTimers, CSFeManHudState, CSFeManImp, CSLuaEventManImp, CSMenuManImp,
        CSTaskGroupIndex, CSTaskImp, ChrActions, FieldInsHandle, GameDataMan, GameMan, WorldChrMan,
    },
    fd4::{FD4PadManager, FD4TaskData},
};
use fromsoftware_shared::{FromStatic, RecurringTaskHandle, SharedTaskImpExt};
use std::sync::{
    Arc, Mutex, MutexGuard, TryLockError,
    atomic::{AtomicBool, Ordering},
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

#[derive(Clone, Copy, PartialEq, Eq)]
struct ActorStamp {
    handle: FieldInsHandle,
    chr_ins_address: usize,
    player_game_data_address: usize,
    save_slot: i32,
}
#[derive(Clone, Copy)]
struct OwnedSuppression {
    actor: ActorStamp,
    owns_secondary_actions: bool,
    owns_movement: bool,
}
#[derive(Default)]
struct FrameCapture {
    owned: Option<OwnedSuppression>,
}
#[derive(Clone)]
struct Signals {
    enabled: Arc<AtomicBool>,
    modal: Arc<AtomicBool>,
    safe: Arc<AtomicBool>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Begin,
    BeforeBehavior,
    End,
}

/// Keep the module and these callbacks resident for the entire process.
/// Pinned upstream tasks cannot be unregistered safely. Drop disables new
/// captures; retained callbacks can still restore an outstanding capture.
/// Never hot-unload the DLL after registering these tasks.
pub struct InputTasks {
    enabled: Arc<AtomicBool>,
    _begin: RecurringTaskHandle<FD4TaskData>,
    _before_behavior: RecurringTaskHandle<FD4TaskData>,
    _end: RecurringTaskHandle<FD4TaskData>,
}
impl Drop for InputTasks {
    fn drop(&mut self) {
        self.enabled.store(false, Ordering::Release);
    }
}

/// Register after bootstrap's executable/package gates and native task-system
/// initialization. No waiting, filesystem I/O, hooks or device enumeration.
/// Task registration follows the pinned upstream thread-safe CSTask example.
pub fn install(modal: Arc<AtomicBool>, safe: Arc<AtomicBool>) -> Result<InputTasks, String> {
    let task = unsafe { CSTaskImp::instance() }.map_err(|error| error.to_string())?;
    let state = Arc::new(Mutex::new(FrameCapture::default()));
    let enabled = Arc::new(AtomicBool::new(true));
    let signals = Signals {
        enabled: enabled.clone(),
        modal,
        safe,
    };
    let begin = register_phase(
        task,
        state.clone(),
        signals.clone(),
        Phase::Begin,
        CSTaskGroupIndex::FrameBegin,
    );
    let before_behavior = register_phase(
        task,
        state.clone(),
        signals.clone(),
        Phase::BeforeBehavior,
        CSTaskGroupIndex::ChrIns_PreBehaviorSafe,
    );
    let end = register_phase(task, state, signals, Phase::End, CSTaskGroupIndex::FrameEnd);
    Ok(InputTasks {
        enabled,
        _begin: begin,
        _before_behavior: before_behavior,
        _end: end,
    })
}

fn register_phase(
    task: &CSTaskImp,
    state: Arc<Mutex<FrameCapture>>,
    signals: Signals,
    phase: Phase,
    group: CSTaskGroupIndex,
) -> RecurringTaskHandle<FD4TaskData> {
    task.run_recurring(
        move |_: &FD4TaskData| {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let Some(mut frame) = try_state(&state) else {
                    return;
                };
                // Recover a skipped FrameEnd before acquiring this frame's
                // bits. No game reference survives any task callback.
                if phase == Phase::Begin || phase == Phase::End {
                    let restored = unsafe { restore_current(&mut frame) };
                    if phase == Phase::End || !restored {
                        return;
                    }
                }
                if !signals.enabled.load(Ordering::Acquire)
                    || !signals.modal.load(Ordering::Acquire)
                    || !signals.safe.load(Ordering::Acquire)
                {
                    unsafe { restore_current(&mut frame) };
                    return;
                }
                if !unsafe { fresh_input_safe() }
                    || !unsafe { suppress_current(&mut frame, phase == Phase::BeforeBehavior) }
                {
                    // Yield immediately on focus/menu/load/death/ownership
                    // changes. Only the runtime may establish safe=true again.
                    signals.modal.store(false, Ordering::Release);
                    signals.safe.store(false, Ordering::Release);
                    unsafe { restore_current(&mut frame) };
                }
            }));
            if outcome.is_err() {
                signals.enabled.store(false, Ordering::Release);
                signals.modal.store(false, Ordering::Release);
                signals.safe.store(false, Ordering::Release);
            }
        },
        group,
    )
}

fn try_state(state: &Mutex<FrameCapture>) -> Option<MutexGuard<'_, FrameCapture>> {
    match state.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(error)) => Some(error.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// Resolve the current actor afresh, and only acquire input bits currently
/// false. Existing native restrictions and every unrelated flag are preserved.
unsafe fn suppress_current(frame: &mut FrameCapture, discard_requests: bool) -> bool {
    let Ok(game) = (unsafe { GameMan::instance() }) else {
        return false;
    };
    let Ok(world) = (unsafe { WorldChrMan::instance_mut() }) else {
        return false;
    };
    let Some(player) = world.main_player.as_mut() else {
        return false;
    };
    let actor = ActorStamp {
        handle: player.chr_ins.field_ins_handle,
        chr_ins_address: &player.chr_ins as *const _ as usize,
        player_game_data_address: player.player_game_data.as_ptr() as usize,
        save_slot: game.save_slot,
    };
    let requests = &player.chr_ins.modules.action_request;
    if requests.owner.as_ptr() as usize != actor.chr_ins_address
        || requests.action_request_queue.input_entries.len()
            > requests.action_request_queue.input_entries.capacity()
        || requests.action_request_queue.cancel_entries.len()
            > requests.action_request_queue.cancel_entries.capacity()
        || requests.action_request_queue.input_entries.capacity() > 4096
        || requests.action_request_queue.cancel_entries.capacity() > 4096
    {
        return false;
    }
    if frame.owned.is_some_and(|owned| owned.actor != actor) {
        // FieldInsHandle has no allocation generation. An address/slot change
        // invalidates the old claim; never clear bits on a replacement actor.
        frame.owned = None;
    }
    let owned = frame.owned.get_or_insert(OwnedSuppression {
        actor,
        owns_secondary_actions: false,
        owns_movement: false,
    });
    // Record each claim before its mutation, permitting panic cleanup.
    if !player.chr_ins.debug_flags.disabled_secondary_actions() {
        owned.owns_secondary_actions = true;
        player
            .chr_ins
            .debug_flags
            .set_disabled_secondary_actions(true);
    }
    if !player.chr_ins.debug_flags.disabled_movement() {
        owned.owns_movement = true;
        player.chr_ins.debug_flags.set_disabled_movement(true);
    }
    if discard_requests {
        let requests = &mut player.chr_ins.modules.action_request;
        // Actual button/request caches only: do not alter animation permission
        // masks, TAE cancel flags, queue TAE IDs, vector lengths or allocation.
        requests.action_requests = ChrActions(0);
        requests.previous_action_requests = ChrActions(0);
        requests.new_action_presses = ChrActions(0);
        requests.released_actions = ChrActions(0);
        requests.cancel_ready_actions = ChrActions(0);
        requests.queued_action_inputs = ChrActions(0);
        requests.readback_new_presses = ChrActions(0);
        requests.readback_cancel_ready = ChrActions(0);
        requests.readback_queued_inputs = ChrActions(0);
        requests.movement_request_flags.set_raw_input(false);
        requests.movement_request_flags.set_cancel_eligible(false);
        requests.movement_request_flags.set_dash(false);
        requests.movement_request_duration = 0.0;
        requests.action_timers = ActionTimers {
            r1: 0.0,
            r2: 0.0,
            l1: 0.0,
            l2: 0.0,
            action: 0.0,
            roll: 0.0,
            jump: 0.0,
            use_item: 0.0,
            switch_spell: 0.0,
            change_weapon_r: 0.0,
            change_weapon_l: 0.0,
            change_item: 0.0,
            r3: 0.0,
            l3: 0.0,
            touch_r: 0.0,
            touch_l: 0.0,
        };
        for entry in requests.action_request_queue.input_entries.iter_mut() {
            entry.actions = ChrActions(0);
        }
        for entry in requests.action_request_queue.cancel_entries.iter_mut() {
            entry.actions = ChrActions(0);
        }
    }
    true
}

/// Restore only our two acquired bits on the freshly resolved same actor.
/// A missing actor defers cleanup; a replacement invalidates the scalar claim.
/// Never restore old request caches: replaying them would leak modal clicks.
unsafe fn restore_current(frame: &mut FrameCapture) -> bool {
    let Some(owned) = frame.owned else {
        return true;
    };
    let Ok(game) = (unsafe { GameMan::instance() }) else {
        return false;
    };
    if !(0..10).contains(&game.save_slot) {
        return false;
    }
    let Ok(world) = (unsafe { WorldChrMan::instance_mut() }) else {
        return false;
    };
    let Some(player) = world.main_player.as_mut() else {
        return false;
    };
    let current = ActorStamp {
        handle: player.chr_ins.field_ins_handle,
        chr_ins_address: &player.chr_ins as *const _ as usize,
        player_game_data_address: player.player_game_data.as_ptr() as usize,
        save_slot: game.save_slot,
    };
    if current == owned.actor {
        if owned.owns_secondary_actions {
            player
                .chr_ins
                .debug_flags
                .set_disabled_secondary_actions(false);
        }
        if owned.owns_movement {
            player.chr_ins.debug_flags.set_disabled_movement(false);
        }
    }
    frame.owned = None;
    true
}

/// Fresh native gates, not the previous render-thread snapshot. Requiring the
/// normal HUD, free announcement controller and no Lua load/reentry excludes
/// known native menu/loading paths without guessing undocumented UI indices.
/// No complete typed CSMovie/CSRemo state exists in the pinned SDK; this remains
/// a conservative source-derived gate, not a guarantee proven in gameplay.
unsafe fn fresh_input_safe() -> bool {
    let Ok(pads) = (unsafe { FD4PadManager::instance() }) else {
        return false;
    };
    if pads.window_handle == 0 || unsafe { GetForegroundWindow() }.0 as isize != pads.window_handle
    {
        return false;
    }
    let Ok(game) = (unsafe { GameMan::instance() }) else {
        return false;
    };
    if game.warp_requested || game.new_game_plus_requested || !(0..10).contains(&game.save_slot) {
        return false;
    }
    let Ok(data) = (unsafe { GameDataMan::instance() }) else {
        return false;
    };
    if data.just_died || data.boss_fight_active {
        return false;
    }
    let Ok(world) = (unsafe { WorldChrMan::instance() }) else {
        return false;
    };
    let Some(player) = world.main_player.as_ref() else {
        return false;
    };
    let player_data = unsafe { player.player_game_data.as_ref() };
    if !unsafe { crate::game::native_offline_host(game, player_data) }
        || player.chr_ins.modules.data.hp <= 0
        || player.chr_ins.chr_flags1c5.death_flag()
        || player.chr_ins.chr_flags1c5.is_invincible()
    {
        return false;
    }
    let Ok(fe) = (unsafe { CSFeManImp::instance() }) else {
        return false;
    };
    if fe.hud_state != CSFeManHudState::Default {
        return false;
    }
    let Ok(menu) = (unsafe { CSMenuManImp::instance() }) else {
        return false;
    };
    if menu.disable_save_menu != 0 {
        return false;
    }
    let announcement = unsafe { menu.system_announce_view_model.view.as_ref() };
    if !announcement.is_active || announcement.is_visible {
        return false;
    }
    let Ok(lua) = (unsafe { CSLuaEventManImp::instance() }) else {
        return false;
    };
    !lua.lua_event_proxy.is_load_wait
        && lua
            .lua_event_script_imitation
            .as_ref()
            .is_some_and(|imitation| !imitation.is_wait_reentry_to_map)
}
