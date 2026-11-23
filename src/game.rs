//! Read-only, version-gated Elden Ring adapter.
//!
//! All game references are acquired and dropped on the native task callback.
//! The render thread receives owned snapshots only. No effect or save writes.
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use eldenring::{
    cs::{
        CSEventFlagMan, CSFeManHudState, CSFeManImp, CSLuaEventManImp, CSMenuManImp,
        CSTaskGroupIndex, CSTaskImp, EventWorldType, GameDataMan, GameMan, MultiplayRole,
        ChrIns, PlayerGameData, SoloParamRepository, SpEffectParam, WorldChrMan,
    },
    fd4::FD4TaskData,
};
use fromsoftware_shared::{FromStatic, RecurringTaskHandle, SharedTaskImpExt};
use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
    },
    core::BOOL,
};

pub const SUPPORTED_PRODUCT_VERSION: &str = "2.7.2.0";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawCharacterIdentity {
    pub save_slot: i32,
    pub character_id: u32,
    pub packed_time_stamp: u32,
    pub name: String,
    /// Set after the separate mod-owned UUID is proven persisted in the save.
    /// This never certifies character_id or packed_time_stamp as a birth key.
    pub verified: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attributes {
    pub vigor: u32,
    pub mind: u32,
    pub endurance: u32,
    pub strength: u32,
    pub dexterity: u32,
    pub intelligence: u32,
    pub faith: u32,
    pub arcane: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resources {
    pub hp: i32,
    pub max_hp: i32,
    pub fp: i32,
    pub max_fp: i32,
    pub stamina: i32,
    pub max_stamina: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub loaded: bool,
    /// Source-derived normal-HUD/menu/loading gate. Runtime additionally waits
    /// for a stable frame window; this does not certify complete cutscene detection.
    pub safe: bool,
    /// Ready for read-only flag observation, independently from UI safety.
    pub can_observe: bool,
    pub offline: bool,
    /// Native bytes preserve evidence without conflating online mode with
    /// general server availability or assuming every enum value is mapped.
    #[serde(default)]
    pub is_in_online_mode_raw: Option<u8>,
    #[serde(default)]
    pub server_connection_enabled_raw: Option<u8>,
    #[serde(default)]
    pub event_world_type_raw: Option<u8>,
    #[serde(default)]
    pub is_my_world_raw: Option<u8>,
    #[serde(default)]
    pub start_offline_raw: Option<u8>,
    #[serde(default)]
    pub game_multiplay_role_raw: Option<u8>,
    #[serde(default)]
    pub player_multiplay_role_raw: Option<u8>,
    #[serde(default)]
    pub hud_state_raw: Option<u8>,
    #[serde(default)]
    pub system_announce_active_raw: Option<u8>,
    #[serde(default)]
    pub system_announce_visible_raw: Option<u8>,
    pub identity: Option<RawCharacterIdentity>,
    pub ng_plus: u32,
    pub play_time_ms: u32,
    /// Rune level (character level).
    #[serde(default)]
    pub level: u32,
    /// 0.8: runes held (Ascension stat purchases).
    #[serde(default)]
    pub runes: u32,
    pub attributes: Attributes,
    pub resources: Resources,
    pub boss_fight_active: bool,
    /// Normal in-world HUD state (no menu/map/loading screen overlay).
    #[serde(default)]
    pub normal_hud: bool,
    pub dodging: bool,
    pub guarding: bool,
    pub airborne: bool,
    pub equip_load_ratio: Option<f32>,
    pub player_address: u64,
    pub boss_defeated: BTreeMap<u32, bool>,
    /// Visible UI indices are diagnostic evidence, not guessed menu identities.
    pub visible_native_ui: Vec<usize>,
    pub unsafe_reasons: Vec<String>,
    /// Non-blocking disagreements retained separately from safety failures.
    #[serde(default)]
    pub diagnostic_notes: Vec<String>,
    /// 3.1: the attributes you levelled (the Level Up page's numbers); the
    /// `attributes` above are these plus every active boost (talismans,
    /// physick tears, Great Runes, armour, cards).
    #[serde(default)]
    pub base_attributes: Attributes,
    /// 3.1: damage multipliers of every active effect (physical, magic,
    /// fire, lightning, holy): buffs, tears, talismans and cards. Card
    /// spells are your attacks, so they get these too (their attack rows
    /// accept 100% of effect buffs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_rates: Option<[f32; 5]>,
    /// 3.0: an Ascended Boss fell in the last few minutes: (token, how many
    /// Ascended affixes it had). The worker gives it to the next new victory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ascended_victory: Option<(u64, u8)>,
}

impl GameSnapshot {
    /// No pause/equipment/map/system menu among the visible native UI states
    /// (indices from labelled in-game tests of the CS2 conversion: 28 pause,
    /// 29 equipment, 31/32 pause pages, 37 system, 43 popup, 61 map).
    fn visible_native_ui_menu_free(&self) -> bool {
        !self
            .visible_native_ui
            .iter()
            .any(|i| matches!(i, 28 | 29 | 31 | 32 | 37 | 43 | 61))
    }
}

/// Current online mode, not general server availability or a startup preference.
/// Raw comparison also rejects values outside the SDK's canonical bool mapping.
pub(crate) fn actual_offline(game: &GameMan) -> bool {
    unsafe {
        std::ptr::addr_of!(game.is_in_online_mode)
            .cast::<u8>()
            .read()
            == 0
    }
}

/// Complete offline local-host proof for the freshly resolved main player's data.
///
/// Sources in pinned fromsoftware-rs 57023153: game_man.rs defines Local=0 and
/// current online mode; multiplay_type.rs defines Host=0; game_data_man.rs and
/// player_game_data.rs expose the main-data pointer and main-player ownership.
/// `server_connection_enabled` is general availability (rename 8c80b2d), while
/// `is_my_world` remains diagnostic because its offline initialization is unclear.
/// No unknown online/world/role/main-player discriminant can pass this policy.
///
/// # Safety
/// Native task only. `data` must come from the current WorldChrMan.main_player.
/// This acquires only a shared GameDataMan; it never reborrows WorldChrMan, so
/// callers may already hold their one mutable WorldChrMan/player reference.
pub(crate) unsafe fn native_offline_host(game: &GameMan, data: &PlayerGameData) -> bool {
    if !actual_offline(game)
        || unsafe {
            std::ptr::addr_of!(game.event_world_type)
                .cast::<u8>()
                .read()
        } != EventWorldType::Local as u8
        || unsafe { std::ptr::addr_of!(game.multiplay_role).cast::<u8>().read() }
            != MultiplayRole::Host as u8
        || unsafe { std::ptr::addr_of!(data.multiplay_role).cast::<u8>().read() }
            != MultiplayRole::Host as u8
        || unsafe { std::ptr::addr_of!(data.is_main_player).cast::<u8>().read() } != 1
    {
        return false;
    }
    let Ok(game_data) = (unsafe { GameDataMan::instance() }) else {
        return false;
    };
    std::ptr::eq(data, game_data.main_player_game_data.as_ptr())
}

/// A live task cannot be hot-unloaded safely with the pinned upstream task API.
/// Dropping this handle stops our work using a separate atomic gate, but does
/// not unregister the upstream callback. Keep the DLL loaded until process exit.
pub struct TaskRegistration {
    active: Arc<AtomicBool>,
    _task: RecurringTaskHandle<FD4TaskData>,
}

impl Drop for TaskRegistration {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

unsafe extern "system" fn enum_visible_process_window(hwnd: HWND, context: LPARAM) -> BOOL {
    // SAFETY: EnumWindows invokes synchronously; context points to stack state.
    let state = unsafe { &mut *(context.0 as *mut WindowProbe) };
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    if process == state.process && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        state.visible = true;
        return BOOL(0);
    }
    BOOL(1)
}
struct WindowProbe {
    process: u32,
    visible: bool,
}
fn has_visible_game_window() -> bool {
    let mut state = WindowProbe {
        process: unsafe { GetCurrentProcessId() },
        visible: false,
    };
    // EnumWindows returns an error for intentional early cancellation as well.
    let _ = unsafe {
        EnumWindows(
            Some(enum_visible_process_window),
            LPARAM((&mut state as *mut WindowProbe) as isize),
        )
    };
    state.visible
}

/// Register from the DLL's bootstrap worker after its mandatory SHA256
/// compatibility gate, never from DllMain or the main game thread. This waits
/// for initialization. Caller retains the handle for the entire DLL lifetime.
/// This is crate-private so bootstrap remains the single entry point.
pub(crate) fn register(
    mut boss_flags: Vec<u32>,
    mut callback: impl FnMut(GameSnapshot) + Send + 'static,
) -> Result<TaskRegistration, String> {
    boss_flags.sort_unstable();
    boss_flags.dedup();
    if boss_flags.iter().any(|flag| *flag == 0) {
        return Err("boss definitions contain reserved event flag zero".into());
    }
    if boss_flags.len() > 4096 {
        return Err("boss observation limit exceeded".into());
    }

    // Reflection singleton discovery must not run before the game's CRT and
    // window initialization, since upstream caches the first reflection scan.
    let started = Instant::now();
    while !has_visible_game_window() {
        if started.elapsed() > Duration::from_secs(120) {
            return Err("game window did not become visible within 120 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Avoid permanently caching incomplete reflection metadata at startup.
    // This discovery is fresh; null instance values are normal on the title screen.
    let program = fromsoftware_shared::Program::current();
    eldenring::util::system::wait_for_system_init(&program, Duration::from_secs(30))
        .map_err(|e| e.to_string())?;
    let required = [
        CSTaskImp::name(),
        WorldChrMan::name(),
        CSEventFlagMan::name(),
        CSMenuManImp::name(),
        CSFeManImp::name(),
        CSLuaEventManImp::name(),
        SoloParamRepository::name(),
        eldenring::fd4::FD4PadManager::name(),
    ];
    loop {
        let derived = from_singleton::find::derived_singletons(program);
        let partial = from_singleton::find::fd4_singletons(program);
        if !partial.all_null() {
            let discovered = unsafe { partial.finish() };
            if required.iter().all(|name| {
                derived.contains_key(name.as_ref()) || discovered.contains_key(name.as_ref())
            }) {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let task_runtime = CSTaskImp::wait_for_instance(Duration::from_secs(30))
        .map_err(|error| format!("native task runtime unavailable: {error}"))?;
    let active = Arc::new(AtomicBool::new(true));
    let task_active = active.clone();
    let mut last_sample = Instant::now() - Duration::from_secs(1);
    let task = task_runtime.run_recurring(
        move |_: &FD4TaskData| {
            if !task_active.load(Ordering::Acquire)
                || last_sample.elapsed() < Duration::from_millis(100)
            {
                return;
            }
            last_sample = Instant::now();
            // Read-only sampling and callback execute synchronously in FrameBegin.
            // catch_unwind needs panic=unwind; memory access faults are not caught.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let state = unsafe { snapshot(&boss_flags) };
                callback(state);
            }));
            if result.is_err() {
                task_active.store(false, Ordering::Release);
            }
        },
        CSTaskGroupIndex::FrameBegin,
    );
    Ok(TaskRegistration {
        active,
        _task: task,
    })
}

/// Capture owned diagnostic data, without mutating any game fields.
///
/// # Safety
/// Only call from the main native game task thread, after
/// the bootstrap SHA256 compatibility gate and native initialization succeeded.
/// No reference into the game escapes this function.
pub unsafe fn snapshot(boss_flags: &[u32]) -> GameSnapshot {
    let mut result = GameSnapshot::default();
    let world = match unsafe { WorldChrMan::instance() } {
        Ok(world) => world,
        Err(error) => {
            result
                .unsafe_reasons
                .push(format!("world unavailable: {error}"));
            return result;
        }
    };
    let Some(player) = world.main_player.as_ref() else {
        result.unsafe_reasons.push("local player not loaded".into());
        return result;
    };
    let Ok(game_data) = (unsafe { GameDataMan::instance() }) else {
        result.unsafe_reasons.push("game data unavailable".into());
        return result;
    };
    let Ok(game) = (unsafe { GameMan::instance() }) else {
        result
            .unsafe_reasons
            .push("game manager unavailable".into());
        return result;
    };
    let player_data = unsafe { player.player_game_data.as_ref() };
    // Address projections do not load a Rust bool/enum discriminant. This also
    // makes a stale SDK layout visible as an unexpected byte instead of hiding
    // it behind a combined, derived offline/local-world result.
    let online_raw = unsafe {
        std::ptr::addr_of!(game.is_in_online_mode)
            .cast::<u8>()
            .read()
    };
    let server_raw = unsafe {
        std::ptr::addr_of!(game.server_connection_enabled)
            .cast::<u8>()
            .read()
    };
    let event_world_raw = unsafe {
        std::ptr::addr_of!(game.event_world_type)
            .cast::<u8>()
            .read()
    };
    let own_world_raw = unsafe {
        std::ptr::addr_of!(player_data.is_my_world)
            .cast::<u8>()
            .read()
    };
    result.is_in_online_mode_raw = Some(online_raw);
    result.server_connection_enabled_raw = Some(server_raw);
    result.event_world_type_raw = Some(event_world_raw);
    result.is_my_world_raw = Some(own_world_raw);
    result.game_multiplay_role_raw =
        Some(unsafe { std::ptr::addr_of!(game.multiplay_role).cast::<u8>().read() });
    result.player_multiplay_role_raw = Some(unsafe {
        std::ptr::addr_of!(player_data.multiplay_role)
            .cast::<u8>()
            .read()
    });
    let main_player_raw = unsafe {
        std::ptr::addr_of!(player_data.is_main_player)
            .cast::<u8>()
            .read()
    };
    let settings = game_data.game_settings.as_ptr();
    if !settings.is_null() {
        result.start_offline_raw = Some(unsafe {
            std::ptr::addr_of!((*settings).start_offline)
                .cast::<u8>()
                .read()
        });
    }
    if main_player_raw != 1 || player_data as *const _ != game_data.main_player_game_data.as_ptr() {
        result
            .unsafe_reasons
            .push("main player data association unavailable".into());
        return result;
    }
    result.loaded = true;
    result.player_address = player.player_game_data.as_ptr() as u64;
    // server_connection_enabled describes availability in general, not an
    // active online session. ME3's offline property does not disable it.
    result.offline = online_raw == 0;
    result.ng_plus = game_data.ng_lvl;
    result.play_time_ms = game_data.play_time;
    result.boss_fight_active = game_data.boss_fight_active;
    let name_end = player_data
        .character_name
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(player_data.character_name.len());
    result.identity = Some(RawCharacterIdentity {
        save_slot: game.save_slot,
        character_id: player_data.character_id,
        packed_time_stamp: player_data.packed_time_stamp,
        name: String::from_utf16_lossy(&player_data.character_name[..name_end]),
        verified: false,
    });
    result.level = player_data.level;
    result.runes = player_data.rune_count;

    // The game's own boosted values; 3.1 also adds up every active effect's
    // attribute bonus itself and keeps the higher of the two, so a talisman,
    // a knot tear or a Great Rune always reaches the Attribute Overhaul.
    let base = [
        player_data.vigor,
        player_data.mind,
        player_data.endurance,
        player_data.strength,
        player_data.dexterity,
        player_data.intelligence,
        player_data.faith,
        player_data.arcane,
    ];
    let effective = [
        player_data.effective_vigor,
        player_data.effective_mind,
        player_data.effective_endurance,
        player_data.effective_strength,
        player_data.effective_dexterity,
        player_data.effective_intelligence,
        player_data.effective_faith,
        player_data.effective_arcane,
    ];
    let (adds, rates) = unsafe { effect_boosts(&player.chr_ins) };
    let value = |i: usize| {
        let summed = (i64::from(base[i]) + i64::from(adds[i])).clamp(1, 99) as u32;
        if (1..=999).contains(&effective[i]) { effective[i].max(summed) } else { summed }
    };
    result.base_attributes = Attributes {
        vigor: base[0],
        mind: base[1],
        endurance: base[2],
        strength: base[3],
        dexterity: base[4],
        intelligence: base[5],
        faith: base[6],
        arcane: base[7],
    };
    result.attributes = Attributes {
        vigor: value(0),
        mind: value(1),
        endurance: value(2),
        strength: value(3),
        dexterity: value(4),
        intelligence: value(5),
        faith: value(6),
        arcane: value(7),
    };
    result.damage_rates = Some(rates);
    let data = &player.chr_ins.modules.data;
    result.resources = Resources {
        hp: data.hp,
        max_hp: data.max_hp,
        fp: data.fp,
        max_fp: data.max_fp,
        stamina: data.stamina,
        max_stamina: data.max_stamina,
    };
    let action = player.chr_ins.modules.action_flag.action_modifiers_flags;
    result.dodging = action.dodging();
    result.guarding = action.guard_type_set();
    result.airborne = !player.chr_ins.modules.physics.standing_on_solid_ground;
    result.equip_load_ratio = unsafe { equipped_load(player_data) };
    let sane_attributes = [
        result.attributes.vigor,
        result.attributes.mind,
        result.attributes.endurance,
        result.attributes.strength,
        result.attributes.dexterity,
        result.attributes.intelligence,
        result.attributes.faith,
        result.attributes.arcane,
    ]
    .iter()
    .all(|value| (1..=999).contains(value));
    if !sane_attributes {
        result
            .unsafe_reasons
            .push("effective attributes outside supported diagnostic range".into());
    }
    if !result.offline {
        result.unsafe_reasons.push(if online_raw == 1 {
            "game reports online mode".into()
        } else {
            format!("online-mode byte is outside the SDK bool mapping: {online_raw}")
        });
    }
    if event_world_raw != EventWorldType::Local as u8 {
        result.unsafe_reasons.push(format!(
            "event world is not SDK Local: raw={event_world_raw}"
        ));
    }
    if result.game_multiplay_role_raw != Some(MultiplayRole::Host as u8) {
        result.unsafe_reasons.push(format!(
            "game multiplayer role is not SDK Host: raw={:?}",
            result.game_multiplay_role_raw
        ));
    }
    if result.player_multiplay_role_raw != Some(MultiplayRole::Host as u8) {
        result.unsafe_reasons.push(format!(
            "player multiplayer role is not SDK Host: raw={:?}",
            result.player_multiplay_role_raw
        ));
    }
    let local_offline_host = unsafe { native_offline_host(game, player_data) };
    if local_offline_host && own_world_raw != 1 {
        result.diagnostic_notes.push(format!(
            "is_my_world raw={own_world_raw} disagrees with the complete offline local-host/main-data proof; ownership byte is diagnostic only"
        ));
    }
    if !(0..10).contains(&game.save_slot) {
        result.unsafe_reasons.push("invalid save slot".into());
    }
    if data.hp <= 0 || game_data.just_died || player.chr_ins.chr_flags1c5.death_flag() {
        result
            .unsafe_reasons
            .push("player dead or death transition active".into());
    }
    if game.warp_requested || game.new_game_plus_requested {
        result
            .unsafe_reasons
            .push("warp or NG+ transition requested".into());
    }
    if player.chr_ins.chr_flags1c5.is_invincible() {
        result
            .unsafe_reasons
            .push("native loading/event invulnerability active".into());
    }
    if let Ok(menu) = unsafe { CSMenuManImp::instance() } {
        result.visible_native_ui = menu
            .ui_states
            .iter()
            .enumerate()
            .filter_map(|(i, state)| state.visible().then_some(i))
            .collect();
    } else {
        result
            .unsafe_reasons
            .push("native UI singleton unavailable".into());
    }

    if let Ok(flags) = unsafe { CSEventFlagMan::instance() } {
        let storage = &flags.virtual_memory_flag;
        if storage.event_flag_divisor == 1000
            && storage.event_flag_holder_size == 125
            && storage.event_flag_holder_count > 0
            && !storage.flag_blocks.is_null()
        {
            for &flag in boss_flags {
                result.boss_defeated.insert(flag, storage.get_flag(flag));
            }
        } else {
            result
                .unsafe_reasons
                .push("event flag storage not ready".into());
        }
    } else {
        result
            .unsafe_reasons
            .push("event flag singleton unavailable".into());
    }
    result.can_observe = result.unsafe_reasons.is_empty() && local_offline_host;
    // Source-defined normal HUD and announcement states: no guessed UI indices.
    let normal_hud = unsafe { CSFeManImp::instance() }.is_ok_and(|fe| {
        let raw = unsafe { std::ptr::addr_of!(fe.hud_state).cast::<u8>().read() };
        result.hud_state_raw = Some(raw);
        raw == CSFeManHudState::Default as u8
    });
    let free_menu = unsafe { CSMenuManImp::instance() }.is_ok_and(|menu| {
        let view = unsafe { menu.system_announce_view_model.view.as_ref() };
        let active = unsafe { std::ptr::addr_of!(view.is_active).cast::<u8>().read() };
        let visible = unsafe { std::ptr::addr_of!(view.is_visible).cast::<u8>().read() };
        result.system_announce_active_raw = Some(active);
        result.system_announce_visible_raw = Some(visible);
        active == 1 && visible == 0 && menu.disable_save_menu == 0
    });
    let free_load = unsafe { CSLuaEventManImp::instance() }.is_ok_and(|lua| {
        !lua.lua_event_proxy.is_load_wait
            && lua
                .lua_event_script_imitation
                .as_ref()
                .is_some_and(|s| !s.is_wait_reentry_to_map)
    });
    result.normal_hud = normal_hud && free_load && result.visible_native_ui_menu_free();
    result.safe =
        result.can_observe && normal_hud && free_menu && free_load && !result.boss_fight_active;
    if !result.safe {
        result
            .unsafe_reasons
            .push("reward waits for normal HUD, free menu and stable local world".into());
    }
    result
}

unsafe fn equipped_load(data: &eldenring::cs::PlayerGameData) -> Option<f32> {
    use eldenring::param::EquipParamStruct;
    if !data.max_equip_load.is_finite() || data.max_equip_load <= 0.0 {
        return None;
    }
    let params = unsafe { SoloParamRepository::instance() }.ok()?;
    let e = &data.equipment.equipment_entries;
    let mut ids = vec![
        e.weapon_primary_left,
        e.weapon_primary_right,
        e.weapon_secondary_left,
        e.weapon_secondary_right,
        e.weapon_tertiary_left,
        e.weapon_tertiary_right,
        e.protector_head,
        e.protector_chest,
        e.protector_hands,
        e.protector_legs,
    ];
    ids.extend(e.accessories.iter().filter_map(|id| id.as_valid()));
    let mut total = 0.0;
    for id in ids {
        if id.as_optional().as_valid().is_none() {
            continue;
        }
        let weight = match params.get_equip_param(id)? {
            EquipParamStruct::EQUIP_PARAM_WEAPON_ST(row) => row.weight(),
            EquipParamStruct::EQUIP_PARAM_PROTECTOR_ST(row) => row.weight(),
            EquipParamStruct::EQUIP_PARAM_ACCESSORY_ST(row) => row.weight(),
            _ => return None,
        };
        if !weight.is_finite() || weight < 0.0 {
            return None;
        }
        total += weight;
    }
    Some(total / data.max_equip_load)
}

/// 3.1: what the player's active effects add: attribute points (Vigor ..
/// Arcane) and damage multipliers (physical, magic, fire, lightning, holy;
/// damage rate x attack power rate). Covers talismans, physick tears,
/// Great Runes, armour, incantation buffs and Ascension's own rows.
///
/// # Safety
/// Main game task; `player` is the live main player.
unsafe fn effect_boosts(player: &ChrIns) -> ([i32; 8], [f32; 5]) {
    let mut adds = [0i32; 8];
    let mut rates = [1.0f32; 5];
    let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else {
        return (adds, rates);
    };
    for entry in player.special_effect.entries() {
        let Ok(id) = u32::try_from(entry.param_id) else {
            continue;
        };
        let Some(row) = repo.get::<SpEffectParam>(id) else {
            continue;
        };
        let add = [
            row.add_life_force_status(),
            row.add_willpower_status(),
            row.add_endure_status(),
            row.add_strength_status(),
            row.add_dexterity_status(),
            row.add_magic_status(),
            row.add_faith_status(),
            row.add_luck_status(),
        ];
        for (total, value) in adds.iter_mut().zip(add) {
            *total += i32::from(value);
        }
        let rate = [
            row.physics_attack_rate() * row.physics_attack_power_rate(),
            row.magic_attack_rate() * row.magic_attack_power_rate(),
            row.fire_attack_rate() * row.fire_attack_power_rate(),
            row.thunder_attack_rate() * row.thunder_attack_power_rate(),
            row.dark_attack_rate() * row.dark_attack_power_rate(),
        ];
        for (total, value) in rates.iter_mut().zip(rate) {
            if value.is_finite() && value > 0.0 {
                *total *= value;
            }
        }
    }
    for rate in &mut rates {
        *rate = if rate.is_finite() { rate.clamp(0.1, 10.0) } else { 1.0 };
    }
    (adds, rates)
}

