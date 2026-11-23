//! 0.6: item use animations (local TAE id 50000-59999, confirmed by the 0.7.0
//! log: 50110, 50112, 50210) play faster; 0.8 Celerity and cards also speed up
//! attack animations (local 30000-39999, the player's attack range; the first
//! ones sped up are logged). Every frame, right before the behavior update,
//! CSChrBehaviorModule's animation_speed is set to 1 + bonus while such an
//! animation plays; afterwards it is returned to 1.0.
//! Ownership is taken only when the speed is 1.0, so another effect that
//! changes the speed is never overwritten. Source-derived (fromsoftware-rs
//! field, FromSoftware's player TAE layout); not verified in game.
use eldenring::{
    cs::{CSTaskGroupIndex, CSTaskImp, GameMan, WorldChrMan},
    fd4::FD4TaskData,
};
use fromsoftware_shared::{FromStatic, RecurringTaskHandle, SharedTaskImpExt};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

/// Distinct item animations sped up so far (for the runtime log).
#[derive(Default)]
pub struct HasteLog {
    pub seen: Vec<i32>,
    pub logged: usize,
}

pub struct HasteTask {
    _task: RecurringTaskHandle<FD4TaskData>,
}

pub fn is_item_anim(anim: i32) -> bool {
    (50_000..=59_999).contains(&anim.rem_euclid(1_000_000))
}
/// 0.8: the player's attack animations (light, heavy, charged, running,
/// jumping attacks); skills and throws are excluded.
pub fn is_attack_anim(anim: i32) -> bool {
    (30_000..=39_999).contains(&anim.rem_euclid(1_000_000))
}

/// `bonus_permille`: item bonus, `attack_permille`: attack bonus; 0 = off,
/// at most 500 (+50%).
pub fn install(
    bonus_permille: Arc<AtomicU32>,
    attack_permille: Arc<AtomicU32>,
    log: Arc<Mutex<HasteLog>>,
) -> Result<HasteTask, String> {
    let runtime = CSTaskImp::wait_for_instance(Duration::from_secs(30)).map_err(|e| e.to_string())?;
    // (behavior module address, speed written)
    let mut owned: Option<(usize, f32)> = None;
    let task = runtime.run_recurring(
        move |_: &FD4TaskData| {
            let Ok(game) = (unsafe { GameMan::instance() }) else {
                owned = None;
                return;
            };
            let Ok(world) = (unsafe { WorldChrMan::instance_mut() }) else {
                owned = None;
                return;
            };
            let Some(player) = world.main_player.as_mut() else {
                owned = None;
                return;
            };
            let player_data = unsafe { player.player_game_data.as_ref() };
            let host = unsafe { crate::game::native_offline_host(game, player_data) };
            let chr = &mut player.chr_ins;
            let alive = chr.modules.data.hp > 0 && !chr.chr_flags1c5.death_flag();
            let time_act = &chr.modules.time_act;
            let index = time_act.read_idx as usize % time_act.anim_queue.len();
            let anim = time_act.anim_queue[index].anim_id;
            let behavior = &mut chr.modules.behavior;
            let address = &**behavior as *const _ as usize;
            if owned.is_some_and(|(a, _)| a != address) {
                owned = None;
            }
            let permille = if is_item_anim(anim) {
                bonus_permille.load(Ordering::Acquire).min(500)
            } else if is_attack_anim(anim) {
                attack_permille.load(Ordering::Acquire).min(500)
            } else {
                0
            };
            let speed = 1.0 + permille as f32 / 1000.0;
            let current = behavior.animation_speed;
            if permille > 0 && host && alive && !game.warp_requested && current.is_finite() {

                let ours = owned.is_some_and(|(_, written)| (current - written).abs() < 1e-4);
                if ours || (current - 1.0).abs() < 1e-3 {
                    behavior.animation_speed = speed;
                    owned = Some((address, speed));
                    if let Ok(mut log) = log.try_lock() {
                        if log.seen.len() < 16 && !log.seen.contains(&anim) {
                            log.seen.push(anim);
                        }
                    }
                }
            } else if let Some((_, written)) = owned.take() {
                if (behavior.animation_speed - written).abs() < 1e-4 {
                    behavior.animation_speed = 1.0;
                }
            }
        },
        CSTaskGroupIndex::ChrIns_PreBehaviorSafe,
    );
    Ok(HasteTask { _task: task })
}
