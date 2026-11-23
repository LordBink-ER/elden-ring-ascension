//! Bounded INT roll-window extension, after behavior and before damage shapes.
//! Implementation uses the pinned SDK task/TAE flags; gameplay is unverified.
use eldenring::{
    cs::{CSTaskGroupIndex, CSTaskImp, FieldInsHandle, GameMan, WorldChrMan},
    fd4::FD4TaskData,
};
use fromsoftware_shared::{FromStatic, RecurringTaskHandle, SharedTaskImpExt};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

/// A FieldInsHandle has container/index/block identity but no allocation
/// generation. Keep addresses as comparison-only scalars, never old references.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ActorStamp {
    handle: FieldInsHandle,
    chr_ins_address: usize,
    player_game_data_address: usize,
    save_slot: i32,
}

#[derive(Default)]
struct Window {
    actor: Option<ActorStamp>,
    native_last_frame: bool,
    native_window_start: Option<Instant>,
    extension_until: Option<Instant>,
    last_extension: Option<Instant>,
    owns_event_bit: bool,
    owns_pve_bit: bool,
}

pub struct RollTasks {
    _begin: RecurringTaskHandle<FD4TaskData>,
    _damage: RecurringTaskHandle<FD4TaskData>,
}

/// Called only by bootstrap after the executable and dedicated package gates.
/// The returned tasks must live until the process exits.
pub fn install(extension_ms: Arc<AtomicU32>) -> Result<RollTasks, String> {
    let runtime =
        CSTaskImp::wait_for_instance(Duration::from_secs(30)).map_err(|e| e.to_string())?;
    let state = Arc::new(Mutex::new(Window::default()));
    let begin_state = state.clone();
    let begin = runtime.run_recurring(
        move |_: &FD4TaskData| {
            let Ok(mut state) = begin_state.lock() else {
                return;
            };
            let Ok(game) = (unsafe { GameMan::instance() }) else {
                *state = Window::default();
                return;
            };
            let Ok(world) = (unsafe { WorldChrMan::instance_mut() }) else {
                *state = Window::default();
                return;
            };
            let Some(player) = world.main_player.as_mut() else {
                *state = Window::default();
                return;
            };
            let actor = ActorStamp {
                handle: player.chr_ins.field_ins_handle,
                chr_ins_address: &player.chr_ins as *const _ as usize,
                player_game_data_address: player.player_game_data.as_ptr() as usize,
                save_slot: game.save_slot,
            };
            if state.actor != Some(actor) {
                *state = Window {
                    actor: Some(actor),
                    ..Default::default()
                };
                return;
            }
            // Remove only the bits that this extension supplied in the prior frame.
            // Native behavior then establishes this frame's own TAE window normally.
            if state.owns_event_bit {
                player.chr_ins.modules.event.flags &= !2;
            }
            if state.owns_pve_bit {
                player
                    .chr_ins
                    .modules
                    .action_flag
                    .action_modifiers_flags
                    .set_pve_only_iframes(false);
            }
            state.owns_event_bit = false;
            state.owns_pve_bit = false;
        },
        CSTaskGroupIndex::FrameBegin,
    );

    let damage = runtime.run_recurring(
        move |_: &FD4TaskData| {
            let Ok(mut state) = state.lock() else {
                return;
            };
            let Ok(game) = (unsafe { GameMan::instance() }) else {
                *state = Window::default();
                return;
            };
            let Ok(world) = (unsafe { WorldChrMan::instance_mut() }) else {
                *state = Window::default();
                return;
            };
            let Some(player) = world.main_player.as_mut() else {
                *state = Window::default();
                return;
            };
            let actor = ActorStamp {
                handle: player.chr_ins.field_ins_handle,
                chr_ins_address: &player.chr_ins as *const _ as usize,
                player_game_data_address: player.player_game_data.as_ptr() as usize,
                save_slot: game.save_slot,
            };
            if state.actor != Some(actor) {
                *state = Window {
                    actor: Some(actor),
                    ..Default::default()
                };
            }
            let flags = player.chr_ins.modules.action_flag.action_modifiers_flags;
            let player_data = unsafe { player.player_game_data.as_ref() };
            let eligible = unsafe { crate::game::native_offline_host(game, player_data) }
                && !game.warp_requested
                && !game.new_game_plus_requested
                && player.chr_ins.modules.data.hp > 0
                && !player.chr_ins.chr_flags1c5.death_flag()
                && !player.chr_ins.chr_flags1c5.is_invincible()
                && flags.dodging()
                && !flags.perfect_invincibility()
                && !flags.invincible_during_throw_attacker()
                && !flags.invincible_excluding_throw_attacks_defender()
                && !flags.jump_frames_lower_body_iframes();
            let native = player.chr_ins.modules.event.flags & 2 != 0 || flags.pve_only_iframes();
            let now = Instant::now();
            let milliseconds = extension_ms.load(Ordering::Acquire).min(100);
            if !eligible || milliseconds == 0 {
                state.native_last_frame = native;
                state.native_window_start = None;
                state.extension_until = None;
                return;
            }
            if native && !state.native_last_frame {
                state.native_window_start = Some(now);
            }
            let bounded_native_window = state.native_window_start.is_some_and(|start| {
                (0.08..=0.65).contains(&now.duration_since(start).as_secs_f32())
            });
            if state.native_last_frame
                && !native
                && bounded_native_window
                && state
                    .last_extension
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(750))
            {
                state.extension_until = Some(now + Duration::from_millis(milliseconds as u64));
                state.last_extension = Some(now);
            }
            state.native_last_frame = native;
            if !native {
                state.native_window_start = None;
            }
            if native {
                state.extension_until = None;
            }
            if !native && state.extension_until.is_some_and(|end| now < end) {
                state.owns_event_bit = player.chr_ins.modules.event.flags & 2 == 0;
                state.owns_pve_bit = !player
                    .chr_ins
                    .modules
                    .action_flag
                    .action_modifiers_flags
                    .pve_only_iframes();
                player.chr_ins.modules.event.flags |= 2;
                player
                    .chr_ins
                    .modules
                    .action_flag
                    .action_modifiers_flags
                    .set_pve_only_iframes(true);
            }
        },
        // This phase precedes DmgMan_Pre/ShapeCast, instead of racing its
        // native damage-preparation tasks inside the same group.
        CSTaskGroupIndex::HavokAi_SilhouetteGeneratorHelper_End,
    );
    Ok(RollTasks {
        _begin: begin,
        _damage: damage,
    })
}
