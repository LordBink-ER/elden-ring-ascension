//! Game-task orchestration. Durable progression belongs to a separate worker;
//! UI only submits epoch-scoped commands and reads committed owned snapshots.
use crate::bosses::{BossScaler, BossScalingConfig};
use crate::{
    config::{BossDefinition, Config},
    core::{self, Catalog, Offer, Progression, Store},
    effects::NativeEffects,
    game::GameSnapshot,
    identity::{
        self, BindingScope, BoundIdentity, IdentityReply, IdentityService, MarkerManifest,
        NativeBinder, NativeBindingStep,
    },
    powers::{AttributeValues, CombatEvent, CombatSnapshot, PowerLibrary, RecentEvent},
    procs::{ProcContext, ProcStatus},
    session::Session,
    world::WorldTracker,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct ProgressView {
    pub epoch: u64,
    pub ready: bool,
    pub character_key: String,
    pub levels: BTreeMap<String, u8>,
    pub offer: Option<Offer>,
    pub pending: usize,
    pub total_choices: usize,
    pub dry_offers: u32,
    pub embers: u32,
    pub banished: Vec<String>,
    /// 0.5 Fate: the locked card, rewards skipped, runes still owed.
    pub locked: Option<String>,
    pub skipped: usize,
    pub rune_debt: u32,
    /// Rune payments acknowledged by the storage worker (session counter).
    pub rune_acks: u64,
    /// 0.6 Fate cards: Lock cost, skip rune multiplier and Embers per skip.
    pub lock_cost: u32,
    pub skip_rate: f64,
    pub skip_embers: u32,
    pub status: String,
    /// 3.0 Chronicle: every card taken, oldest first (boss, card, level).
    pub chronicle: Vec<core::ChoiceRecord>,

    pub error: Option<String>,
    pub capabilities: BTreeSet<String>,
}
pub enum Command {
    Detach {
        epoch: u64,
    },
    Attach {
        epoch: u64,
        binding: BoundIdentity,
        snapshot: GameSnapshot,
        capabilities: BTreeSet<String>,
    },
    Observe {
        epoch: u64,
        snapshot: GameSnapshot,
    },
    Capabilities {
        epoch: u64,
        capabilities: BTreeSet<String>,
    },
    Choose {
        epoch: u64,
        offer: u64,
        power: String,
    },
    Reroll {
        epoch: u64,
        offer: u64,
    },
    Banish {
        epoch: u64,
        offer: u64,
        slot: usize,
    },
    /// Embers earned on the game task (Ascended Elite kills).
    Embers {
        epoch: u64,
        amount: u32,
    },
    /// 0.5: lock the card in `slot` (Embers).
    Lock {
        epoch: u64,
        offer: u64,
        slot: usize,
    },
    /// 0.5: skip this reward for runes and one Ember.
    Skip {
        epoch: u64,
        offer: u64,
    },
    /// 0.5: the game task paid `amount` owed runes into the character.
    RunesPaid {
        epoch: u64,
        amount: u32,
    },
}

/// CSMenuManImp ui state of the Level Up page (CS2 conversion's labelled test;
/// detected in the user's 1.0 session log).
const LEVEL_UP_UI: usize = 48;

/// Live combat state for the HUD tracker; written by the game task.
#[derive(Clone, Default)]
pub struct HudView {
    pub procs: Vec<ProcStatus>,
    pub intervention_armed: bool,
    pub intervention_cooldown: f32,
    pub intervention_saves: u32,
    pub streak: u16,
    pub recent_kills: u16,
    pub in_combat: bool,
    pub spawned_total: u64,
    pub spawn_failures: u64,
    pub last_spawn_error: Option<i32>,
    pub level: u32,
    pub ng_plus: u32,
    pub attributes: AttributeValues,
    pub luck: f64,
    /// The game shows its normal HUD (no menu, map, loading or cutscene).
    pub hud_ok: bool,
    /// Card status buildup: (status, buildup, listed in the weapon menus).
    pub status_buildup: Vec<(String, i32, bool)>,
    /// Living Ascended Elites nearby: (affixes, distance m).
    pub elites_nearby: Vec<(String, f32)>,
    pub elites_met: u32,
    pub elites_killed: u32,
    /// Embers just earned from an elite (amount, when) for a toast.
    pub ember_gain: Option<(u32, Instant)>,
    /// 0.5: affixes of this NG+ cycle's bosses, and whether a boss is fought.
    pub boss_affixes: Vec<crate::fate::BossAffix>,
    /// 3.0: the NG+ cycle's affixes (every boss), for the You tab.
    pub cycle_affixes: Vec<crate::fate::BossAffix>,
    /// 3.1: points your active effects add to each attribute (Vigor..Arcane)
    /// and your damage multipliers (physical, magic, fire, lightning, holy).
    pub attribute_boost: [u32; 8],
    pub damage_rates: [f32; 5],
    pub boss_fight: bool,
    /// 0.5: HP paid to Blood Price this session.
    pub blood_paid: u32,
    /// 0.5: runes just paid for a skipped reward (amount, when) for a toast.
    pub rune_gain: Option<(u32, Instant)>,
    /// 0.5: skip rewards enabled in [fate].
    pub skip_enabled: bool,
    /// 2.3: the latest combat label (moments.rs) and session reflex totals.
    pub flash: Option<crate::moments::Flash>,
    pub reflexes: crate::reflexes::ReflexTotals,
    /// 0.6: damage kind an Adaptive boss resists now (index into ELEMENTS).
    pub adaptive_element: usize,
    /// 0.6: flask/item animation speed bonus in effect (0..0.5).
    pub item_speed: f32,
    /// The Tide, runes held, Path points, speed and lifesteal in effect.
    pub tide: u8,
    pub runes: u32,
    pub paths: crate::paths::PathPoints,
    pub attack_speed: f32,
    pub lifesteal: f32,
    pub healed: u64,
    pub spread: u32,
    pub ascension_power: u32,
    /// 0.9: allies' damage / HP / defense bonus and how many have it.
    pub ally_boost: [f32; 3],
    pub allies_buffed: u32,
    /// 1.0: a one-time hint for a character without cards.
    pub hint: Option<(String, Instant)>,
    /// 2.3: the banner on screen (an Elite felled, a Path bonus, death
    /// defied, the Tide rising).
    pub banner: Option<crate::moments::Flash>,
}


pub struct SharedRuntime {
    pub view: Mutex<ProgressView>,
    pub hud: Mutex<HudView>,
    pub modal: Arc<AtomicBool>,
    pub safe: Arc<AtomicBool>,
    pub epoch: AtomicU64,
    pub roll_ms: Arc<AtomicU32>,
    /// 0.6: flask/item animation speed bonus for the per-frame task (permille).
    pub item_speed: Arc<AtomicU32>,
    pub item_speed_ready: Arc<AtomicBool>,
    pub haste_log: Arc<Mutex<crate::haste::HasteLog>>,
    pub library: Arc<PowerLibrary>,
    /// 2.0: boss id -> name (signature cards).
    pub boss_names: BTreeMap<String, String>,
    pub synergies: Arc<crate::synergies::SynergyBook>,
    pub proc_defs: Arc<std::collections::BTreeMap<String, crate::procs::ProcDef>>,
    /// 0.7: card art, animation and sound settings ([ui]).
    pub ui: crate::config::UiSettings,
    /// Attribute Evolution strength ([balance]), for the Build tab.
    pub attribute_strength: f32,
    /// 0.8: attack animation speed bonus for the per-frame task (permille).
    pub attack_speed: Arc<AtomicU32>,
    /// 2.1: the game's Level Up page is open (CSMenuManImp ui state 48, seen
    /// in the 1.0 log); the overlay shows what the attributes give beside it.
    pub level_up_open: AtomicBool,
    /// 3.4: no character is loaded (title screen): the overlay shows the
    /// mod's name and version there.
    pub title_screen: AtomicBool,
    /// 2.2: the attributes the Level Up page shows before you confirm, when
    /// they are above your confirmed ones (level_preview.rs).
    pub level_preview: Mutex<Option<AttributeValues>>,
    /// 1.0: project root (Settings tab saves config/ascension.toml).
    pub root: PathBuf,
    /// [world] settings (runes and the Tide) for the interface.
    pub world: crate::config::WorldSettings,
    tx: SyncSender<Command>,
}
impl SharedRuntime {
    pub fn choose(&self, epoch: u64, offer: u64, power: String) -> Result<(), String> {
        if epoch != self.epoch.load(Ordering::Acquire) {
            return Err("character changed; reopen the reward".into());
        }
        self.tx
            .try_send(Command::Choose {
                epoch,
                offer,
                power,
            })
            .map_err(|e| format!("storage busy; selection not committed: {e}"))
    }
    pub fn banish(&self, epoch: u64, offer: u64, slot: usize) -> Result<(), String> {
        if epoch != self.epoch.load(Ordering::Acquire) {
            return Err("character changed; reopen the reward".into());
        }
        self.tx
            .try_send(Command::Banish { epoch, offer, slot })
            .map_err(|e| format!("storage busy; banish not committed: {e}"))
    }
    pub fn reroll(&self, epoch: u64, offer: u64) -> Result<(), String> {
        if epoch != self.epoch.load(Ordering::Acquire) {
            return Err("character changed; reopen the reward".into());
        }
        self.tx
            .try_send(Command::Reroll { epoch, offer })
            .map_err(|e| format!("storage busy; reroll not committed: {e}"))
    }
    pub fn lock(&self, epoch: u64, offer: u64, slot: usize) -> Result<(), String> {
        if epoch != self.epoch.load(Ordering::Acquire) {
            return Err("character changed; reopen the reward".into());
        }
        self.tx
            .try_send(Command::Lock { epoch, offer, slot })
            .map_err(|e| format!("storage busy; lock not committed: {e}"))
    }
    pub fn skip(&self, epoch: u64, offer: u64) -> Result<(), String> {
        if epoch != self.epoch.load(Ordering::Acquire) {
            return Err("character changed; reopen the reward".into());
        }
        self.tx
            .try_send(Command::Skip { epoch, offer })
            .map_err(|e| format!("storage busy; skip not committed: {e}"))
    }
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct LaunchContext {
    schema: u32,
    steam_account: String,
    profile_namespace: String,
    save_path: PathBuf,
}

/// 3.4.1: without the Linux launcher (a plain ME3 launch, e.g. on Windows)
/// the context comes from the game's own save folder: %APPDATA%\EldenRing\
/// <SteamID64>\. The account folder that holds this mod's save wins, else
/// the most recently used one. Written to local/ once that save exists.
fn derive_launch_context(root: &std::path::Path, namespace: &str) -> Result<LaunchContext, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("APPDATA is not set")?;
    let base = PathBuf::from(appdata).join("EldenRing");
    let mut accounts: Vec<(bool, std::time::SystemTime, String, PathBuf)> = std::fs::read_dir(&base)
        .map_err(|e| format!("no Elden Ring save folder at {}: {e}", base.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() != 17 || !name.bytes().all(|b| b.is_ascii_digit()) || !entry.path().is_dir() {
                return None;
            }
            let folder = entry.path();
            let save = folder.join(namespace);
            let has_save = save.is_file();
            let used = std::fs::metadata(if has_save { &save } else { &folder })
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((has_save, used, name, save))
        })
        .collect();
    accounts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    let (has_save, _, steam_account, save_path) = accounts
        .into_iter()
        .next()
        .ok_or_else(|| format!("no Steam account folder in {}", base.display()))?;
    let context = LaunchContext {
        schema: 1,
        steam_account,
        profile_namespace: namespace.to_owned(),
        save_path,
    };
    // Kept only once this mod's save exists, so a first guess between two
    // Steam accounts is made again next time instead of sticking.
    if has_save {
        if let Ok(json) = serde_json::to_vec_pretty(&context) {
            let _ = std::fs::create_dir_all(root.join("local"));
            let _ = std::fs::write(root.join("local/launch-context.json"), json);
        }
    }
    Ok(context)
}

pub struct Runtime {
    pub shared: Arc<SharedRuntime>,
    session: Session,
    marker: MarkerManifest,
    identity: IdentityService,
    binder: Option<NativeBinder>,
    bound: Option<BoundIdentity>,
    request: Option<u64>,
    next_verify: Instant,
    effects: NativeEffects,
    actor: Option<(i32, u64)>,
    epoch: u64,
    last_hp: Option<i32>,
    hp_loss: Instant,
    dodge_end: Option<Instant>,
    was_dodging: bool,
    combat_start: Option<Instant>,
    safe_since: Option<Instant>,
    last_telemetry: Instant,
    last_status: String,
    attribute_strength: f32,
    attach_sent: bool,
    committed_levels: BTreeMap<String, u8>,
    advertised_capabilities: BTreeSet<String>,
    effect_warnings: Vec<String>,
    boss_scaler: BossScaler,
    world: WorldTracker,
    elites: crate::elites::Elites,
    elite_warning: Option<String>,
    proc_failures_logged: u32,
    last_proc_log: Instant,
    /// Confirmed hits/kills/best streak since the last minute's log line.
    combat_log: (u32, u32, u16, Instant),
    last_elite_kill: Option<Instant>,
    scan_announced: bool,
    last_pause_log: Instant,
    status_buildup: Vec<(String, i32, bool)>,
    fate: crate::config::FateSettings,
    /// Owed runes sent for payment: (epoch, worker acknowledgements when
    /// sent) until the worker acknowledges that payment.
    rune_in_flight: Option<(u64, u64)>,
    blood_paid: u32,
    blood_logged: u32,
    rune_gain: Option<(u32, Instant)>,
    affixes_logged: Option<(u32, Vec<crate::fate::BossAffix>)>,
    /// 0.6 reflexes (perfect dodge, block, guard counter, critical, ...).
    reflexes: crate::reflexes::ReflexTracker,
    reflex_log: (crate::reflexes::ReflexTotals, Instant),
    flask_logged: u32,
    adaptive_logged: Option<usize>,
    item_speed_logged: Option<u32>,
    /// The world (Tide rows, Plague Bearer), the Path points of the committed
    /// cards, and the powers in effect (cards plus the hidden Path bonuses).
    tide: crate::tide::Tide,
    tide_level: u8,
    /// 0.9 Command: allies' owned row.
    allies: crate::allies::Allies,
    ally_boost: [f32; 3],
    path_points: crate::paths::PathPoints,
    /// Path tiers seen last tick (None until the first committed view) and
    /// the latest unlock notice.
    path_tiers: Option<[usize; 9]>,
    /// 2.3: the combat label on screen (and the rank of its sound), the
    /// banner on screen, banners waiting, and the Tide seen last tick.
    flash: Option<(crate::moments::Flash, u64)>,
    banner: Option<crate::moments::Flash>,
    banners: std::collections::VecDeque<(crate::moments::Flash, crate::sound::Cue)>,
    tide_seen: Option<u8>,
    /// 3.0: the last Ascended Boss that fell: (token, affixes, when).
    ascended_felled: Option<(u64, u8, Instant)>,
    ascended_token: u64,
    /// 3.4: since when no character is loaded.
    unloaded_since: Option<Instant>,
    /// 3.1: attribute boosts last logged, and when damage was last logged.
    boosts_logged: Option<[u32; 8]>,
    damage_logged: Option<(Instant, [i32; 5])>,
    /// 3.2 No Quarter.
    pursuit: crate::pursuit::Pursuit,
    pursuit_warning: Option<String>,
    effective_levels: BTreeMap<String, u8>,
    paths_logged: Option<(crate::paths::PathPoints, u8)>,
    healed_total: u64,
    attack_logged: usize,
    tide_warning: Option<String>,
    hint_shown: bool,
    level_up_logged: bool,
    level_preview: crate::level_preview::LevelPreview,
}
impl Runtime {
    pub fn new(
        root: PathBuf,
        config: Config,
        bosses: Vec<BossDefinition>,
        session: Session,
    ) -> Result<Self, String> {
        let attribute_strength = config.balance.attribute_strength;
        let fate = config.fate.clone();
        let elites = crate::elites::Elites::new(config.elites.clone());
        let mut effects = NativeEffects::default();
        effects.load_procs(&root);
        effects.set_elites_enabled(config.elites.enabled);
        effects.set_status_priority(
            &config
                .status
                .priority
                .iter()
                .map(|channel| channel.field())
                .collect::<Vec<_>>(),
        )?;
        // Boss tier by health-bar name id; a missing table = every boss "greater".
        let names: BTreeMap<i32, u8> = std::fs::read(root.join("config/boss-tiers-by-name.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|v| v.get("tiers").cloned())
            .and_then(|v| v.as_object().cloned())
            .map(|tiers| {
                tiers
                    .iter()
                    .filter_map(|(id, tier)| {
                        let index = match tier.as_str()? {
                            "minor" => 0,
                            "greater" => 1,
                            "major" => 2,
                            "remembrance" => 3,
                            _ => return None,
                        };
                        Some((id.parse().ok()?, index))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let boss_scaler = BossScaler::new(BossScalingConfig {
            effect_id: 95_030_000,
            tiers: config.balance.tiers(),
            names,
            poise: config.world.boss_poise,
        })?;

        let context: LaunchContext = match std::fs::read(root.join("local/launch-context.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            // 3.4.1: a plain ME3 launch (Windows): found from the save folder.
            Err(_) => derive_launch_context(&root, &config.profile_namespace)
                .map_err(|e| format!("launch context unavailable ({e}); start the game once with this profile so the save folder exists"))?,
        };
        if context.schema != 1 || context.profile_namespace != config.profile_namespace {
            return Err("launch context does not match this save namespace".into());
        }
        let marker: MarkerManifest = serde_json::from_slice(
            &std::fs::read(root.join("config/identity-marker.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        marker.validate()?;
        let library = Arc::new(
            PowerLibrary::from_json(
                &std::fs::read_to_string(root.join("config/powers.json"))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        );
        // Synergies are an extra layer: a broken file disables them, never
        // the progression itself.
        let synergies = Arc::new(
            std::fs::read_to_string(root.join("config/synergies.json"))
                .map_err(|e| e.to_string())
                .and_then(|text| {
                    crate::synergies::SynergyBook::from_json(&text, &library)
                        .map_err(|e| e.to_string())
                })
                .unwrap_or_else(|error| {
                    session.status(format!("Synergies disabled: {error}"));
                    crate::synergies::SynergyBook {
                        schema: 1,
                        synergies: Vec::new(),
                    }
                }),
        );
        effects.set_synergies(synergies.clone());
        let identity = IdentityService::start(
            root.join("state/bindings"),
            BindingScope {
                steam_account: context.steam_account,
                profile_namespace: context.profile_namespace,
            },
            marker.clone(),
            context.save_path,
            root.clone(),
        )?;
        let (tx, rx) = mpsc::sync_channel(32);
        let shared = Arc::new(SharedRuntime {
            view: Mutex::new(ProgressView::default()),
            hud: Mutex::new(HudView::default()),
            modal: Arc::new(AtomicBool::new(false)),
            safe: Arc::new(AtomicBool::new(false)),
            epoch: AtomicU64::new(0),
            roll_ms: Arc::new(AtomicU32::new(0)),
            item_speed: Arc::new(AtomicU32::new(0)),
            item_speed_ready: Arc::new(AtomicBool::new(false)),
            haste_log: Arc::new(Mutex::new(crate::haste::HasteLog::default())),
            boss_names: bosses.iter().map(|b| (b.id.clone(), b.name.clone())).collect(),
            library,
            synergies,
            proc_defs: Arc::new(effects.procs().defs().clone()),
            ui: config.ui.clone(),
            attribute_strength: config.balance.attribute_strength,
            attack_speed: Arc::new(AtomicU32::new(0)),
            level_up_open: AtomicBool::new(false),
            title_screen: AtomicBool::new(true),
            level_preview: Mutex::new(None),
            root: root.clone(),
            world: config.world,
            tx,
        });
        let config_world = config.world;
        let worker_shared = shared.clone();
        let worker_session = session.clone();
        let cursed: BTreeSet<String> = shared
            .library
            .powers
            .iter()
            .filter(|p| p.curse.is_some())
            .map(|p| p.id.clone())
            .collect();
        std::thread::Builder::new()
            .name("ascension-progression-storage".into())
            .spawn(move || {
                let mut store: Option<Store> = None;
                let mut catalog: Option<Catalog> = None;
                let mut current_epoch = 0;
                let mut snapshot: Option<(GameSnapshot, Instant)> = None;
                let mut bound_identity: Option<BoundIdentity> = None;
                let mut recovery_checked_cycle: Option<u32> = None;
                // 3.0: the last fallen Ascended Boss already given to a victory.
                let mut ascended_used: Option<u64> = None;
                let mut rune_acks: u64 = 0;
                while let Ok(command) = rx.recv() {
                    let result: core::Result<()> = (|| {
                        match command {
                            Command::Detach { epoch } => {
                                current_epoch = epoch;
                                store = None;
                                catalog = None;
                                snapshot = None;
                                bound_identity = None;
                                recovery_checked_cycle = None;
                                return Ok(());
                            }
                            Command::Attach {
                                epoch,
                                binding,
                                snapshot: initial,
                                capabilities,
                            } => {
                                if epoch != worker_shared.epoch.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                current_epoch = epoch;
                                store = None;
                                catalog = None;
                                snapshot = None;
                                bound_identity = None;
                                recovery_checked_cycle = None;
                                catalog =
                                    Some(worker_shared.library.catalog(capabilities.clone())?);
                                let mut opened = match Store::open(
                                    &root.join("state/progression"),
                                    &binding.character_key,
                                ) {
                                    Ok(store) => store,
                                    Err(core::CoreError::NotFound) => Store::create(
                                        &root.join("state/progression"),
                                        Progression::new(&binding.character_key, os_seed()?)?,
                                    )?,
                                    Err(error) => return Err(error),
                                };
                                // 1.0: a copy of this character's progression
                                // every time it loads (newest 10 kept).
                                match backup_progression(&root, opened.directory()) {
                                    Ok(Some(path)) => worker_session
                                        .status(format!("Ascension progress backed up to {path}")),
                                    Ok(None) => {}
                                    Err(error) => worker_session
                                        .status(format!("Ascension progress backup skipped: {error}")),
                                }
                                reconcile_cycle(&mut opened, &initial, &bosses, None)?;
                                if initial.can_observe
                                    && crate::recovery::apply_bound(
                                        &root,
                                        &binding,
                                        &mut opened,
                                        initial.ng_plus,
                                        &initial.boss_defeated,
                                    )
                                    .map_err(core::CoreError::Invalid)?
                                {
                                    worker_session
                                        .status("Recovered an observed Ascension boss reward");
                                }
                                if initial.can_observe {
                                    recovery_checked_cycle = Some(initial.ng_plus);
                                }
                                store = Some(opened);
                                bound_identity = Some(binding);
                                snapshot = Some((initial, Instant::now()));
                                if let Ok(mut view) = worker_shared.view.lock() {
                                    view.capabilities = capabilities;
                                    view.error = None;
                                }
                            }
                            Command::Observe {
                                epoch,
                                snapshot: observed,
                            } => {
                                if epoch != current_epoch
                                    || epoch != worker_shared.epoch.load(Ordering::Acquire)
                                {
                                    return Ok(());
                                }
                                if let Some(store) = store.as_mut() {
                                    if observed.can_observe {
                                        let fresh_ascended = observed
                                            .ascended_victory
                                            .filter(|(token, _)| Some(*token) != ascended_used);
                                        if let Some(token) =
                                            reconcile_cycle(store, &observed, &bosses, fresh_ascended)?
                                        {
                                            ascended_used = Some(token);
                                        }
                                        if let Some(binding) =
                                            bound_identity.as_ref().filter(|_| {
                                                recovery_checked_cycle != Some(observed.ng_plus)
                                            })
                                        {
                                            if crate::recovery::apply_bound(
                                                &root,
                                                binding,
                                                store,
                                                observed.ng_plus,
                                                &observed.boss_defeated,
                                            )
                                            .map_err(core::CoreError::Invalid)?
                                            {
                                                worker_session.status(
                                                    "Recovered an observed Ascension boss reward",
                                                );
                                            }
                                            recovery_checked_cycle = Some(observed.ng_plus);
                                        }
                                    }
                                }
                                snapshot = Some((observed, Instant::now()));
                            }
                            Command::Capabilities {
                                epoch,
                                capabilities,
                            } => {
                                if epoch != current_epoch
                                    || epoch != worker_shared.epoch.load(Ordering::Acquire)
                                {
                                    return Ok(());
                                }
                                catalog =
                                    Some(worker_shared.library.catalog(capabilities.clone())?);
                                if let Ok(mut view) = worker_shared.view.lock() {
                                    view.capabilities = capabilities;
                                }
                            }
                            Command::Choose {
                                epoch,
                                offer,
                                power,
                            } => {
                                require_choice_frame(
                                    epoch,
                                    current_epoch,
                                    &worker_shared,
                                    &snapshot,
                                )?;
                                let store = store.as_mut().ok_or(core::CoreError::NoOffer)?;
                                store.transaction(|state| {
                                    state.choose(
                                        offer,
                                        &power,
                                        catalog.as_ref().ok_or(core::CoreError::NoOffer)?,
                                    )
                                })?;
                            }
                            Command::Banish { epoch, offer, slot } => {
                                require_choice_frame(
                                    epoch,
                                    current_epoch,
                                    &worker_shared,
                                    &snapshot,
                                )?;
                                let store = store.as_mut().ok_or(core::CoreError::NoOffer)?;
                                let draw = luck_draw(&config, &snapshot, &cursed, &worker_shared.library, store.state());
                                store.transaction(|state| {
                                    state.banish(
                                        offer,
                                        slot,
                                        catalog.as_ref().ok_or(core::CoreError::NoOffer)?,
                                        &draw,
                                    )?;
                                    Ok(())
                                })?;
                            }
                            Command::Lock { epoch, offer, slot } => {
                                require_choice_frame(
                                    epoch,
                                    current_epoch,
                                    &worker_shared,
                                    &snapshot,
                                )?;
                                let store = store.as_mut().ok_or(core::CoreError::NoOffer)?;
                                let cost = core::lock_cost(
                                    catalog.as_ref().ok_or(core::CoreError::NoOffer)?,
                                    store.state().levels(),
                                );
                                store.transaction(|state| {
                                    state.lock_with(offer, slot, cost)?;
                                    Ok(())
                                })?;
                            }
                            Command::Skip { epoch, offer } => {
                                require_choice_frame(
                                    epoch,
                                    current_epoch,
                                    &worker_shared,
                                    &snapshot,
                                )?;
                                if !config.fate.skip_rewards {
                                    return Err(core::CoreError::Invalid(
                                        "skipping rewards is disabled in [fate]".into(),
                                    ));
                                }
                                let store = store.as_mut().ok_or(core::CoreError::NoOffer)?;
                                let (level, ng_plus) = snapshot
                                    .as_ref()
                                    .map(|(s, _)| (s.level.max(1), s.ng_plus))
                                    .unwrap_or((1, 0));
                                let tier = store
                                    .state()
                                    .current_offer()
                                    .map(|o| o.victory.tier)
                                    .ok_or(core::CoreError::NoOffer)?;
                                let (rate, extra) = skip_bonus(
                                    catalog.as_ref().ok_or(core::CoreError::NoOffer)?,
                                    &worker_shared.library,
                                    store.state().levels(),
                                );
                                let runes = (f64::from(core::skip_runes(level, tier, ng_plus)) * rate)
                                    .round()
                                    .min(f64::from(u32::MAX)) as u32;
                                let embers = core::SKIP_EMBERS + extra;
                                let mut granted = 0;
                                store.transaction(|state| {
                                    granted = state.skip_with(offer, runes, embers)?;
                                    Ok(())
                                })?;
                                worker_session.status(format!(
                                    "Ascension reward skipped: {granted} runes owed, +{embers} Ember"
                                ));
                            }
                            Command::RunesPaid { epoch, amount } => {
                                rune_acks += 1;
                                if epoch != current_epoch {
                                    return Ok(());
                                }
                                if let Some(store) = store.as_mut() {
                                    store.transaction(|state| state.runes_paid(amount))?;
                                }
                            }
                            Command::Embers { epoch, amount } => {
                                if epoch != current_epoch
                                    || epoch != worker_shared.epoch.load(Ordering::Acquire)
                                {
                                    return Ok(());
                                }
                                if let Some(store) = store.as_mut() {
                                    let amount = amount.clamp(1, 100);
                                    store.transaction(|state| state.add_embers(amount))?;
                                }
                            }
                            Command::Reroll { epoch, offer } => {
                                require_choice_frame(
                                    epoch,
                                    current_epoch,
                                    &worker_shared,
                                    &snapshot,
                                )?;
                                let store = store.as_mut().ok_or(core::CoreError::NoOffer)?;
                                let draw = luck_draw(&config, &snapshot, &cursed, &worker_shared.library, store.state());
                                store.transaction(|state| {
                                    state.reroll(
                                        offer,
                                        catalog.as_ref().ok_or(core::CoreError::NoOffer)?,
                                        &draw,
                                    )?;
                                    Ok(())
                                })?;
                            }
                        }
                        if let (Some(store), Some(catalog)) = (store.as_mut(), catalog.as_ref()) {
                            if store.state().current_offer().is_none()
                                && store.state().pending_count() > 0
                                && catalog.iter().any(|power| {
                                    catalog.is_enabled(power)
                                        && store.state().level(&power.id)
                                            < config.draw.max_power_level
                                })
                            {
                                let draw = luck_draw(&config, &snapshot, &cursed, &worker_shared.library, store.state());
                                match store.transaction(|state| {
                                    state.prepare_offer(catalog, &draw)?;
                                    Ok(())
                                }) {
                                    Ok(()) | Err(core::CoreError::PoolExhausted { .. }) => {}
                                    Err(error) => return Err(error),
                                }
                            }
                            if current_epoch == worker_shared.epoch.load(Ordering::Acquire) {
                                if let Ok(mut view) = worker_shared.view.lock() {
                                    view.epoch = current_epoch;
                                    view.ready = true;
                                    view.character_key = store.state().character_id().into();
                                    view.levels = store.state().levels().clone();
                                    view.offer = store.state().current_offer().cloned();
                                    view.pending = store.state().pending_count();
                                    view.total_choices = store.state().choices().len();
                                    if view.chronicle.len() != view.total_choices {
                                        view.chronicle = store.state().choices().to_vec();
                                    }
                                    view.dry_offers = store.state().dry_offers();
                                    view.embers = store.state().embers();
                                    view.banished =
                                        store.state().banished().iter().cloned().collect();
                                    view.locked =
                                        store.state().locked().map(|l| l.power_id.clone());
                                    view.skipped = store.state().skipped_count();
                                    view.rune_debt = store.state().rune_debt();
                                    view.rune_acks = rune_acks;
                                    view.lock_cost =
                                        core::lock_cost(catalog, store.state().levels());
                                    let (rate, extra) = skip_bonus(
                                        catalog,
                                        &worker_shared.library,
                                        store.state().levels(),
                                    );
                                    view.skip_rate = rate;
                                    view.skip_embers = core::SKIP_EMBERS + extra;
                                    view.error = None;
                                }
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        worker_session.error(format!("progression stopped: {error}"));
                        if let Ok(mut view) = worker_shared.view.lock() {
                            view.error = Some(error.to_string());
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            session,
            marker,
            identity,
            binder: None,
            bound: None,
            request: None,
            next_verify: Instant::now(),
            effects,
            actor: None,
            epoch: 0,
            last_hp: None,
            hp_loss: Instant::now(),
            dodge_end: None,
            was_dodging: false,
            combat_start: None,
            safe_since: None,
            last_telemetry: Instant::now() - Duration::from_secs(1),
            last_status: String::new(),
            attribute_strength,
            attach_sent: false,
            committed_levels: BTreeMap::new(),
            advertised_capabilities: BTreeSet::new(),
            effect_warnings: Vec::new(),
            boss_scaler,
            world: WorldTracker::default(),
            elites,
            elite_warning: None,
            proc_failures_logged: 0,
            last_proc_log: Instant::now(),
            combat_log: (0, 0, 0, Instant::now()),
            last_elite_kill: None,
            scan_announced: false,
            last_pause_log: Instant::now() - Duration::from_secs(60),
            status_buildup: Vec::new(),
            fate,
            rune_in_flight: None,
            blood_paid: 0,
            blood_logged: 0,
            rune_gain: None,
            affixes_logged: None,
            reflexes: crate::reflexes::ReflexTracker::default(),
            reflex_log: (Default::default(), Instant::now()),
            flask_logged: 0,
            adaptive_logged: None,
            item_speed_logged: None,
            tide: crate::tide::Tide::new(config_world),
            tide_level: 0,
            allies: crate::allies::Allies::default(),
            ally_boost: [0.0; 3],
            path_points: [0; 9],
            path_tiers: None,
            flash: None,
            banner: None,
            banners: Default::default(),
            tide_seen: None,
            ascended_felled: None,
            ascended_token: 0,
            boosts_logged: None,
            unloaded_since: None,
            damage_logged: None,
            pursuit: Default::default(),
            pursuit_warning: None,
            effective_levels: BTreeMap::new(),
            paths_logged: None,
            healed_total: 0,
            attack_logged: 0,
            tide_warning: None,
            hint_shown: false,
            level_up_logged: false,
            level_preview: Default::default(),
        })
    }
    fn status(&mut self, status: impl Into<String>) {
        let status = status.into();
        if self.last_status != status {
            self.session.status(&status);
            self.last_status = status.clone();
            if let Ok(mut view) = self.shared.view.lock() {
                view.status = status;
            }
        }
    }
    fn detach(&mut self) {
        self.boss_scaler.forget_world();
        self.world.reset();
        self.effects.forget_world();
        self.elites.forget_world();
        self.tide.forget_world();
        self.allies.forget_world();
        self.pursuit.forget_world();
        self.path_tiers = None;
        self.tide_seen = None;
        self.banners.clear();
        self.shared.attack_speed.store(0, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.shared.epoch.store(self.epoch, Ordering::Release);
        self.shared.safe.store(false, Ordering::Release);
        self.shared.modal.store(false, Ordering::Release);
        self.shared.roll_ms.store(0, Ordering::Release);
        self.shared.item_speed.store(0, Ordering::Release);
        self.reflexes.reset();
        self.bound = None;
        self.binder = None;
        self.request = None;
        self.actor = None;
        self.last_hp = None;
        self.safe_since = None;
        self.was_dodging = false;
        self.dodge_end = None;
        self.combat_start = None;
        self.attach_sent = false;
        self.committed_levels.clear();
        self.advertised_capabilities.clear();
        self.effect_warnings.clear();
        if let Ok(mut view) = self.shared.view.lock() {
            *view = ProgressView {
                epoch: self.epoch,
                ..Default::default()
            };
        }
        let _ = self
            .shared
            .tx
            .try_send(Command::Detach { epoch: self.epoch });
    }
    /// Main game task only. All native helpers reacquire local references.
    pub unsafe fn tick(&mut self, mut snapshot: GameSnapshot) {
        let now = Instant::now();
        // 0.5: this character's NG+ boss affixes for the current cycle.
        if let Some(bound) = &self.bound {
            if snapshot.can_observe {
                let affixes = crate::fate::boss_affixes(
                    snapshot.ng_plus,
                    crate::fate::seed(&bound.character_key),
                    &self.fate,
                );
                if self
                    .affixes_logged
                    .as_ref()
                    .is_none_or(|(cycle, logged)| *cycle != snapshot.ng_plus || *logged != affixes)
                {
                    if !affixes.is_empty() {
                        let names: Vec<String> = affixes
                            .iter()
                            .map(|a| format!("{} ({})", a.name(), a.summary()))
                            .collect();
                        self.session.status(format!(
                            "NG+{} boss affixes: {}",
                            snapshot.ng_plus,
                            names.join(", ")
                        ));
                    }
                    self.affixes_logged = Some((snapshot.ng_plus, affixes.clone()));
                }
                self.boss_scaler.set_affixes(affixes);
                // 3.0 Ascended Bosses (from Tide I).
                self.boss_scaler.set_ascension(self.fate.ascended_bosses.then(|| {
                    (crate::fate::seed(&bound.character_key), snapshot.ng_plus, self.tide_level)
                }));
            }
        }
        match unsafe {
            self.boss_scaler
                .tick(snapshot.loaded && snapshot.offline && snapshot.can_observe)
        } {
            Ok(events) => {
                for event in events {
                    self.session
                        .status(format!("Ascended World: {}", event.message));
                    match event.kind.as_str() {
                        // 3.0: an Ascended Boss appears: a banner with its affixes.
                        "ascended" if event.ascended > 0 => {
                            let own = event.ascended;
                            self.banners.push_front((
                                crate::moments::banner(
                                    "ASCENDED BOSS",
                                    event.affixes.join("  ·  ").to_uppercase(),
                                    if own >= 2 {
                                        "Win: rarer cards, one more to choose from, and Embers."
                                    } else {
                                        "Win: rarer cards and Embers."
                                    },
                                    [0.95, 0.42, 0.30, 1.0],
                                    now,
                                ),
                                crate::sound::Cue::TideRises,
                            ));
                            self.banner = None;
                        }
                        "ascended_felled" => {
                            self.ascended_token = self.ascended_token.wrapping_add(1);
                            let count = event.ascended.clamp(1, 3);
                            self.ascended_felled = Some((self.ascended_token, count, now));
                        }
                        _ => {}
                    }
                }
            }
            Err(error) => {
                if snapshot.can_observe {
                    self.status(format!("Boss scaling unavailable: {error}"));
                }
            }
        }
        // 3.0: the fallen Ascended Boss rides along until its victory is seen.
        self.ascended_felled = self
            .ascended_felled
            .filter(|(_, _, at)| now.duration_since(*at) < Duration::from_secs(180));
        snapshot.ascended_victory = self.ascended_felled.map(|(token, count, _)| (token, count));
        // 3.4: the title screen (no character for 2 s) gets the mod's banner.
        if snapshot.loaded {
            self.unloaded_since = None;
        } else if self.unloaded_since.is_none() {
            self.unloaded_since = Some(now);
        }
        self.shared.title_screen.store(
            self.unloaded_since.is_some_and(|at| now.duration_since(at) >= Duration::from_secs(2)),
            Ordering::Release,
        );
        if !snapshot.loaded || !snapshot.offline {
            if self.actor.is_some() {
                let _ = unsafe { self.effects.clear() };
                self.detach();
            }
            self.shared.safe.store(false, Ordering::Release);
            self.shared.modal.store(false, Ordering::Release);
            if self.last_telemetry.elapsed() >= Duration::from_secs(1) {
                self.session.snapshot(snapshot);
                self.last_telemetry = now;
            }
            return;
        }
        let actor = snapshot
            .identity
            .as_ref()
            .map(|id| (id.save_slot, snapshot.player_address));
        if actor != self.actor {
            let _ = unsafe { self.effects.clear() };
            self.detach();
            self.actor = actor;
            self.hp_loss = now;
            self.status("Preparing this character's persistent Ascension identity");
        }
        let hurt_share = match self.last_hp {
            Some(hp) if snapshot.resources.hp < hp && snapshot.resources.max_hp > 0 => {
                (hp - snapshot.resources.hp) as f32 / snapshot.resources.max_hp as f32
            }
            _ => 0.0,
        };
        let dodge_started = !self.was_dodging && snapshot.dodging;
        if self.last_hp.is_some_and(|hp| snapshot.resources.hp < hp) {
            self.hp_loss = now;
            self.safe_since = None;
        }
        let previous_hp = self.last_hp;
        self.last_hp = Some(snapshot.resources.hp);
        if self.was_dodging && !snapshot.dodging {
            self.dodge_end = Some(now);
        }
        self.was_dodging = snapshot.dodging;
        if snapshot.boss_fight_active {
            self.combat_start.get_or_insert(now);
        } else {
            self.combat_start = None;
        }
        if snapshot.safe {
            self.safe_since.get_or_insert(now);
        } else {
            self.safe_since = None;
        }
        let safe = self
            .safe_since
            .is_some_and(|start| start.elapsed() >= Duration::from_secs(3))
            && self.hp_loss.elapsed() >= Duration::from_secs(3);
        self.shared.safe.store(safe, Ordering::Release);
        if !safe {
            self.shared.modal.store(false, Ordering::Release);
        }
        let observation = match unsafe { identity::observe(&self.marker, self.epoch) } {
            Ok(value) => value,
            Err(error) => {
                // The current inventory can no longer prove which character
                // owns these powers. Fail closed before any further native work
                // or queued UI choice can use the previous binding.
                self.shared.safe.store(false, Ordering::Release);
                self.shared.modal.store(false, Ordering::Release);
                self.shared.roll_ms.store(0, Ordering::Release);
                let cleanup = unsafe { self.effects.clear() };
                self.detach();
                self.status(format!(
                    "Character identity unavailable; progression detached: {error}"
                ));
                if let Err(cleanup_error) = cleanup {
                    self.session.error(format!(
                        "Identity failure effect cleanup failed: {cleanup_error}"
                    ));
                }
                return;
            }
        };
        while let Some(reply) = self.identity.poll() {
            match reply {
                IdentityReply::Prepared { request, binding }
                    if Some(request) == self.request && binding.anchor() == &observation.anchor =>
                {
                    self.request = None;
                    match NativeBinder::new(self.marker.clone(), binding) {
                        Ok(binder) => self.binder = Some(binder),
                        Err(error) => self.status(error),
                    }
                }
                IdentityReply::Bound {
                    request,
                    anchor,
                    binding,
                } if Some(request) == self.request && anchor == observation.anchor => {
                    self.request = None;
                    self.binder = None;
                    self.bound = Some(binding.clone());
                    self.attach_sent = self
                        .shared
                        .tx
                        .try_send(Command::Attach {
                            epoch: self.epoch,
                            binding,
                            snapshot: snapshot.clone(),
                            capabilities: self.effects.capabilities(),
                        })
                        .is_ok();
                    self.status("Ascension identity persisted; progression attached");
                }
                IdentityReply::Pending { request, reason }
                | IdentityReply::Error { request, reason }
                    if Some(request) == self.request =>
                {
                    self.request = None;
                    self.next_verify = now + Duration::from_secs(1);
                    self.status(reason);
                }
                _ => {}
            }
        }
        if let Some(bound) = &self.bound {
            if observation.token.decode().ok() != Some(bound.uuid) {
                let _ = unsafe { self.effects.clear() };
                self.status("Character identity marker changed; progression detached");
                self.detach();
                return;
            }
            if !self.attach_sent {
                self.attach_sent = self
                    .shared
                    .tx
                    .try_send(Command::Attach {
                        epoch: self.epoch,
                        binding: bound.clone(),
                        snapshot: snapshot.clone(),
                        capabilities: self.effects.capabilities(),
                    })
                    .is_ok();
            }
            if let Some(raw) = snapshot.identity.as_mut() {
                raw.verified = true;
            }
            let _ = self.shared.tx.try_send(Command::Observe {
                epoch: self.epoch,
                snapshot: snapshot.clone(),
            });
        } else if let Some(binder) = self.binder.as_mut() {
            match unsafe { binder.step(self.epoch, safe) } {
                Ok(NativeBindingStep::Progress(status)) => self.status(status),
                Ok(NativeBindingStep::AwaitingPersistence(obs))
                    if self.request.is_none() && now >= self.next_verify =>
                {
                    match self.identity.verify_persisted(obs) {
                        Ok(request) => self.request = Some(request),
                        Err(error) => self.status(error),
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.binder = None;
                    self.status(error);
                }
            }
        } else if self.request.is_none() && now >= self.next_verify && snapshot.can_observe {
            let request = if observation.token.commit_quantity == 1 {
                self.identity.verify_persisted(observation)
            } else {
                self.identity.prepare(observation)
            };
            match request {
                Ok(request) => self.request = Some(request),
                Err(error) => {
                    self.status(error);
                    self.next_verify = now + Duration::from_secs(1);
                }
            }
        }
        // 2.1: the Level Up page at a Site of Grace (read-only panel).
        let level_up = self.bound.is_some() && snapshot.visible_native_ui.contains(&LEVEL_UP_UI);
        if level_up && !self.level_up_logged {
            self.level_up_logged = true;
            self.session.status("Level Up page open: attribute bonuses shown beside it");
        }
        self.shared.level_up_open.store(level_up, Ordering::Release);
        // 2.2: the points on the page before you confirm them.
        let preview = if level_up {
            // 3.1: the page levels your base attributes; boosts (talismans,
            // tears) ride on top of the points you add.
            let as_array = |a: &crate::game::Attributes| {
                [a.vigor, a.mind, a.endurance, a.strength, a.dexterity, a.intelligence, a.faith, a.arcane]
            };
            let boosted = as_array(&snapshot.attributes);
            let base = as_array(&snapshot.base_attributes);
            let committed = if base.iter().all(|v| (1..=99).contains(v)) { base } else { boosted };
            let boost: [u32; 8] = std::array::from_fn(|i| boosted[i].saturating_sub(committed[i]));
            let mut notes = Vec::new();
            let values = unsafe { self.level_preview.tick(committed, snapshot.level, now, &mut notes) };
            for note in notes {
                self.session.status(note);
            }
            values.map(|v| AttributeValues {
                vigor: v[0] + boost[0],
                mind: v[1] + boost[1],
                endurance: v[2] + boost[2],
                strength: v[3] + boost[3],
                dexterity: v[4] + boost[4],
                intelligence: v[5] + boost[5],
                faith: v[6] + boost[6],
                arcane: v[7] + boost[7],
            })
        } else {
            self.level_preview.reset();
            None
        };
        if let Ok(mut shared) = self.shared.level_preview.lock() {
            *shared = preview;
        }
        let mut rune_debt = None;
        let mut view_ready = false;
        if let Ok(view) = self.shared.view.try_lock() {
            if view.epoch == self.epoch && view.ready {
                self.committed_levels = view.levels.clone();
                rune_debt = Some((view.rune_debt, view.rune_acks));
                view_ready = true;
            }
        }
        // 2.0: the powers in effect: owned cards plus the hidden Path bonuses
        // their levels unlock; and the Tide (card levels only).
        self.path_points = crate::paths::points(&self.committed_levels, &self.shared.library);
        // A Path bonus just unlocked: a short notice at the top of the screen.
        let tiers: [usize; 9] = std::array::from_fn(|i| crate::paths::tier(self.path_points[i]));
        if view_ready && self.bound.is_some() {
            if let Some(old) = self.path_tiers {
                for path in crate::paths::PATHS {
                    let i = path.index();
                    if tiers[i] > old[i] {
                        let bonus = &path.bonuses()[tiers[i] - 1];
                        let mut color = path.color();
                        color[3] = 1.0;
                        self.banners.push_back((
                            crate::moments::banner(
                                format!("{} PATH  ·  {}", path.name().to_uppercase(), bonus.points),
                                bonus.name.to_uppercase(),
                                bonus.text,
                                color,
                                now,
                            ),
                            crate::sound::Cue::PathBonus,
                        ));
                        self.session.status(format!(
                            "Path bonus unlocked: {} {} ({})",
                            path.name(),
                            bonus.points,
                            bonus.name
                        ));
                    }
                }
            }
            self.path_tiers = Some(tiers);
        }
        self.effective_levels = self.committed_levels.clone();
        for id in crate::paths::hidden_powers(&self.path_points) {
            self.effective_levels.insert(id.to_owned(), 1);
        }
        let card_levels = core::total_levels(&self.committed_levels);
        self.tide_level = crate::paths::tide(card_levels, self.shared.world.tide_step);
        // 2.3: the Tide rose with your new card levels.
        if view_ready && self.bound.is_some() {
            if self.tide_seen.is_some_and(|seen| self.tide_level > seen) {
                self.banners.push_back((
                    crate::moments::banner(
                        "THE LANDS BETWEEN STIR",
                        "THE TIDE RISES",
                        format!(
                            "Tide {}: enemies grow stronger, runes richer.",
                            crate::moments::roman(self.tide_level)
                        ),
                        [0.48, 0.80, 0.92, 1.0],
                        now,
                    ),
                    crate::sound::Cue::TideRises,
                ));
            }
            self.tide_seen = Some(self.tide_level);
        }
        if self.paths_logged != Some((self.path_points, self.tide_level)) && self.bound.is_some() && view_ready {
            let parts: Vec<String> = crate::paths::PATHS
                .iter()
                .filter(|p| self.path_points[p.index()] > 0)
                .map(|p| format!("{} {}", p.name(), self.path_points[p.index()]))
                .collect();
            let bonuses: Vec<&str> = crate::paths::PATHS
                .iter()
                .flat_map(|p| {
                    let points = self.path_points[p.index()];
                    p.bonuses().into_iter().filter(move |b| points >= b.points).map(|b| b.name)
                })
                .collect();
            self.session.status(format!(
                "Paths: {}; bonuses: {}; Tide {} ({card_levels} card levels)",
                if parts.is_empty() { "none".to_owned() } else { parts.join(", ") },
                if bonuses.is_empty() { "none".to_owned() } else { bonuses.join(", ") },
                self.tide_level
            ));
            self.paths_logged = Some((self.path_points, self.tide_level));
        }

        // 0.5: pay runes owed for skipped rewards, one payment in flight.
        if let Some((debt, acks)) = rune_debt {
            if self
                .rune_in_flight
                .is_some_and(|(epoch, sent)| epoch != self.epoch || acks > sent)
            {
                self.rune_in_flight = None;
            }
            if debt > 0
                && self.rune_in_flight.is_none()
                && self.bound.is_some()
                && snapshot.can_observe
                && snapshot.resources.hp > 0
                && self
                    .shared
                    .tx
                    .try_send(Command::RunesPaid {
                        epoch: self.epoch,
                        amount: debt,
                    })
                    .is_ok()
            {
                self.rune_in_flight = Some((self.epoch, acks));
                match unsafe { crate::fate::native::grant_runes(debt) } {
                    Ok(paid) => {
                        self.rune_gain = Some((paid, now));
                        self.session.status(format!(
                            "Paid {paid} runes for a skipped Ascension reward"
                        ));
                    }
                    Err(error) => self
                        .session
                        .error(format!("Skipped-reward runes not paid: {error}")),
                }
            }
        }
        let mut scan = if snapshot.can_observe && self.bound.is_some() {
            unsafe { self.world.tick(now) }
        } else {
            if self.bound.is_some() && self.last_pause_log.elapsed() >= Duration::from_secs(60) {
                self.last_pause_log = now;
                self.session.status(format!(
                    "World scan paused: {}",
                    snapshot.unsafe_reasons.first().map(String::as_str).unwrap_or("local host check failed")
                ));
            }
            self.world.reset();
            Default::default()
        };
        self.effects.set_world_ready(scan.ready);
        self.effects
            .set_item_speed_ready(self.shared.item_speed_ready.load(Ordering::Acquire));
        // 0.6 reflexes from this scan (perfect dodge, block, critical, ...).
        self.reflexes.observe(&mut scan, now);
        for note in self.reflexes.notes.drain(..) {
            self.session.status(note);
        }
        // 3.2 No Quarter: blows on a critical's victim while it gets up.
        {
            let spec = self
                .shared
                .library
                .downed_strike(&self.effective_levels, &self.effects.capabilities());
            let mut notes = Vec::new();
            match unsafe { self.pursuit.tick(&scan, spec, now, &mut notes) } {
                Ok(Some(handle)) => self.reflexes.note_strike(handle, now),
                Ok(None) => {}
                Err(error) => {
                    if self.pursuit_warning.as_deref() != Some(error.as_str()) {
                        self.session.status(format!("No Quarter unavailable: {error}"));
                        self.pursuit_warning = Some(error);
                    }
                }
            }
            for note in notes {
                self.session.status(note);
            }
        }
        // 2.3: the strongest moment of this tick: its label, its sound
        // ([ui] reflex_sounds) and the cards of yours that react to it. A
        // weaker one does not cover a stronger label still fresh on screen.
        if let Some((flash, cue)) =
            crate::moments::combat(&scan.reflex, &self.shared.library, &self.effective_levels, now)
        {
            crate::sound::play(cue);
            let rank = cue.priority();
            let covered = self
                .flash
                .as_ref()
                .is_some_and(|(shown, shown_rank)| shown.at.elapsed().as_secs_f32() < 0.7 && *shown_rank > rank);
            if !covered {
                self.flash = Some((flash, rank));
            }
        }
        // Banners one after another.
        if self.banner.as_ref().is_none_or(|b| !b.alive()) {
            self.banner = None;
            if let Some((mut banner, cue)) = self.banners.pop_front() {
                banner.at = now;
                crate::sound::play(cue);
                self.banner = Some(banner);
            }
        }
        for note in scan.detail_notes.drain(..) {
            self.session.status(format!("World: {note}"));
        }
        // A flask drink restores a large share of HP at once: log the
        // animation that did it (evidence for the item-speed range).
        if scan.ready
            && self.flask_logged < 4
            && scan.player_state.max_hp > 0
            && previous_hp.is_some_and(|hp| {
                (scan.player_state.hp - hp) * 8 >= scan.player_state.max_hp
            })
        {
            self.flask_logged += 1;
            self.session.status(format!(
                "Heal observed: +{} HP during animation {} (item range: {})",
                scan.player_state.hp - previous_hp.unwrap_or(0),
                scan.player_state.anim,
                crate::haste::is_item_anim(scan.player_state.anim)
            ));
        }
        // 0.6 Adaptive boss affix: the weapon's main damage kind.
        if scan.ready {
            if let Some(element) = unsafe { crate::fate::native::weapon_element() } {
                self.boss_scaler.set_adaptive_element(element);
            }
        }
        if self.boss_scaler.affixes().contains(&crate::fate::BossAffix::Adaptive)
            && self.adaptive_logged != Some(self.boss_scaler.adaptive_element())
        {
            let element = self.boss_scaler.adaptive_element();
            self.adaptive_logged = Some(element);
            self.session.status(format!(
                "Adaptive bosses now resist {} damage (your weapon's main type)",
                crate::fate::ELEMENTS[element]
            ));
        }
        // 0.5 curse Blood Price: FP spent also costs HP (never below 1 HP).
        if scan.ready && scan.fp_spent > 0 {
            if let Some(rate) = self
                .shared
                .library
                .blood_price(&self.effective_levels, &self.effects.capabilities())
            {
                let cost = (scan.fp_spent as f32 * rate).round() as i32;
                match unsafe { crate::fate::native::drain_hp(cost) } {
                    Ok(taken) if taken > 0 => {
                        self.blood_paid = self.blood_paid.saturating_add(taken as u32);
                        if self.blood_logged < 5 {
                            self.blood_logged += 1;
                            self.session.status(format!(
                                "Blood Price: {} FP spent -> {taken} HP",
                                scan.fp_spent
                            ));
                        }
                    }
                    Ok(_) => {}
                    Err(error) => self.session.error(format!("Blood Price: {error}")),
                }
            }
        }
        let (mut lure, mut ember_bonus) = self.shared.library.elite_lure(&self.effective_levels);
        // 2.0 Fortune path 5, Ember Sense.
        if self.path_points[crate::paths::Path::Fortune.index()] >= crate::paths::TIERS[0] {
            lure += 0.3;
            ember_bonus += 1;
        }
        let elite_summary = if scan.ready {
            match unsafe {
                self.elites
                    .tick(&scan, snapshot.ng_plus, lure, ember_bonus, now)
            } {
                Ok(summary) => {
                    self.elite_warning = None;
                    summary
                }
                Err(error) => {
                    if self.elite_warning.as_deref() != Some(error.as_str()) {
                        self.session.status(format!("Ascended Elites unavailable: {error}"));
                        self.elite_warning = Some(error);
                    }
                    Default::default()
                }
            }
        } else {
            Default::default()
        };
        if elite_summary.spawned > 0 {
            self.session.status(format!(
                "Ascended Elites: {} met so far, {} slain",
                self.elites.total_spawned, self.elites.total_killed
            ));
        }
        if elite_summary.killed > 0 {
            self.last_elite_kill = Some(now);
            // 2.3: a banner, like the game's own for a great enemy.
            self.banners.push_back((
                crate::moments::banner(
                    "ASCENDED ELITE",
                    "ELITE FELLED",
                    if elite_summary.embers > 0 {
                        format!("+{} Ascension Embers", elite_summary.embers)
                    } else {
                        String::new()
                    },
                    crate::moments::GOLD,
                    now,
                ),
                crate::sound::Cue::EliteFelled,
            ));
        }
        if elite_summary.embers > 0 {
            let _ = self.shared.tx.try_send(Command::Embers {
                epoch: self.epoch,
                amount: elite_summary.embers,
            });
            self.session.status(format!(
                "Ascended Elite slain: +{} Ascension Embers",
                elite_summary.embers
            ));
        }
        let mut events = std::collections::BTreeMap::new();
        if let Some(age) = scan.seconds_since_hit {
            events.insert(
                CombatEvent::HitLanded,
                RecentEvent {
                    age_seconds: age,
                    stacks: scan.streak.max(1),
                },
            );
        }
        if let Some(age) = scan.seconds_since_kill {
            events.insert(
                CombatEvent::Kill,
                RecentEvent {
                    age_seconds: age,
                    stacks: scan.recent_kills.max(1),
                },
            );
        }
        if let Some(at) = self.last_elite_kill {
            events.insert(
                CombatEvent::EliteKill,
                RecentEvent {
                    age_seconds: now.duration_since(at).as_secs_f32(),
                    stacks: 1,
                },
            );
        }
        // 0.6 reflexes: perfect dodge, block, guard counter, critical,
        // stance break, status procs (generic and per kind).
        if scan.ready {
            for (event, recent) in self.reflexes.recent(now) {
                events.insert(event, recent);
            }
        }
        let a = &snapshot.attributes;
        let ratio = |value: i32, max: i32| {
            if max > 0 {
                (value as f32 / max as f32).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let combat = CombatSnapshot {
            in_game: snapshot.resources.hp > 0,
            hp_ratio: ratio(snapshot.resources.hp, snapshot.resources.max_hp),
            fp_ratio: ratio(snapshot.resources.fp, snapshot.resources.max_fp),
            stamina_ratio: ratio(snapshot.resources.stamina, snapshot.resources.max_stamina),
            equip_load_ratio: snapshot.equip_load_ratio,
            seconds_since_hp_loss: Some(self.hp_loss.elapsed().as_secs_f32()),
            boss_fight_active: snapshot.boss_fight_active,
            dodging: snapshot.dodging,
            guarding: snapshot.guarding,
            airborne: snapshot.airborne,
            heavy_load: snapshot.equip_load_ratio.is_some_and(|ratio| ratio >= 0.7),
            seconds_since_dodge_end: self.dodge_end.map(|end| end.elapsed().as_secs_f32()),
            seconds_in_combat: self
                .combat_start
                .map(|start| start.elapsed().as_secs_f32())
                .unwrap_or(0.0),
            consecutive_hits: scan.streak,
            events,
            attributes: AttributeValues {
                vigor: a.vigor,
                mind: a.mind,
                endurance: a.endurance,
                strength: a.strength,
                dexterity: a.dexterity,
                intelligence: a.intelligence,
                faith: a.faith,
                arcane: a.arcane,
            },
            paths: self.path_points,
            tide: self.tide_level,
            allies_near: scan
                .foes
                .iter()
                .filter(|f| f.protected && f.hp > 0 && Some(f.handle) != scan.player && (1.5..=15.0).contains(&f.distance))
                .count()
                .min(255) as u8,
        };
        // 2.0: what the Paths, the vanilla attributes and the world do.
        let stat_effects = crate::paths::evaluate(
            &combat,
            crate::paths::World {
                rune_multiplier: self.shared.world.rune_multiplier,
                tide_runes: self.shared.world.tide_runes,
            },
            self.attribute_strength,
        );
        self.effects.set_boons(stat_effects.clone());
        let mut intervention_armed = false;
        let mut attack_speed = 0.0_f32;
        let mut lifesteal = 0.0_f32;
        match unsafe {
            self.effects.reconcile(
                &self.shared.library,
                &self.effective_levels,
                &combat,
                self.attribute_strength,
            )
        } {
            Ok(summary) => {
                if summary.warnings != self.effect_warnings {
                    for warning in &summary.warnings {
                        self.session.status(format!("Ascension effect: {warning}"));
                    }
                    self.effect_warnings = summary.warnings;
                }
                self.shared.roll_ms.store(
                    (summary.roll_extension_seconds * 1000.0).clamp(0.0, 100.0) as u32,
                    Ordering::Release,
                );
                intervention_armed = summary.intervention.armed;
                attack_speed = summary.attack_speed;
                lifesteal = summary.lifesteal;
                self.ally_boost = summary.allies;

                if summary.status_buildup != self.status_buildup {
                    if !summary.status_buildup.is_empty() {
                        let lines: Vec<String> = summary
                            .status_buildup
                            .iter()
                            .map(|(status, value, listed)| {
                                format!("{status} +{value}{}", if *listed { " (weapon passive)" } else { " (hidden carrier)" })
                            })
                            .collect();
                        self.session
                            .status(format!("Status buildup from cards: {}", lines.join(", ")));
                    }
                    self.status_buildup = summary.status_buildup.clone();
                }
                if summary.intervention.triggered {
                    self.session
                        .status("Divine intervention: a lethal blow was survived");
                    // 2.3: shown at once, before any waiting banner.
                    self.banners.push_front((
                        crate::moments::banner(
                            "",
                            "DEATH DEFIED",
                            "A deadly blow left you standing.",
                            [1.0, 0.93, 0.70, 1.0],
                            now,
                        ),
                        crate::sound::Cue::DeathDefied,
                    ));
                    self.banner = None;
                }
            }
            Err(error) => {
                self.shared.roll_ms.store(0, Ordering::Release);
                self.shared.safe.store(false, Ordering::Release);
                self.shared.modal.store(false, Ordering::Release);
                self.status(format!("Effects unavailable: {error}"));
            }
        }
        // 0.6: flask/item use speed from Dexterity and Swift Draught cards.
        // 0.8: flask/item speed from Celerity and cards; attack speed too.
        let item_speed = if scan.ready && combat.in_game {
            (stat_effects.item_speed
                + self
                    .shared
                    .library
                    .item_speed(&self.effective_levels, &self.effects.capabilities()))
            .clamp(0.0, 0.4)
        } else {
            0.0
        };
        let permille = (item_speed * 1000.0).round() as u32;
        self.shared.item_speed.store(permille, Ordering::Release);
        let attack_permille = if scan.ready && combat.in_game {
            (attack_speed.clamp(0.0, 0.35) * 1000.0).round() as u32
        } else {
            0
        };
        self.shared.attack_speed.store(attack_permille, Ordering::Release);
        // 0.8 Thirst and lifesteal cards: heal a share of the damage you deal
        // (card spell damage too from Thirst 20).
        if scan.ready && lifesteal > 0.0 && combat.in_game {
            let mut dealt = scan.damage_dealt;
            if stat_effects.lifesteal_spells {
                dealt = dealt.saturating_add(scan.spell_damage_dealt);
            }
            let heal = (dealt as f32 * lifesteal).round() as i32;
            if heal > 0 {
                match unsafe { crate::fate::native::heal_hp(heal) } {
                    Ok(healed) => self.healed_total += healed.max(0) as u64,
                    Err(error) => self.session.error(format!("Lifesteal: {error}")),
                }
            }
        }
        // 0.9 Command: allies' damage, HP, defense, regeneration and poise.
        if scan.ready && combat.in_game {
            let boost = crate::allies::AllyBoost {
                damage: self.ally_boost[0],
                hp: self.ally_boost[1],
                defense: self.ally_boost[2],
                regen: stat_effects.ally_regen,
                poise: if stat_effects.ally_poise > 0.0 { stat_effects.ally_poise } else { 1.0 },
            };
            if let Err(error) = unsafe { self.allies.tick(&scan, boost, now) } {
                if self.tide_warning.as_deref() != Some(error.as_str()) {
                    self.session.status(format!("Ally boost unavailable: {error}"));
                    self.tide_warning = Some(error);
                }
            }
        }
        // 0.8 The world: Tide rows on regular enemies, Malice marks, Lingering
        // Curse and Plague Bearer.
        if scan.ready {
            match unsafe { self.tide.tick(&scan, self.tide_level, &stat_effects, now) } {
                Ok(notes) => {
                    for note in notes {
                        self.session.status(note);
                    }
                }
                Err(error) => {
                    if self.tide_warning.as_deref() != Some(error.as_str()) {
                        self.session.status(format!("The Tide unavailable: {error}"));
                        self.tide_warning = Some(error);
                    }
                }
            }
        }
        if permille > 0
            && self
                .item_speed_logged
                .is_none_or(|logged| logged.abs_diff(permille) >= 10)
        {
            self.item_speed_logged = Some(permille);
            self.session.status(format!(
                "Item use speed: +{:.1}% (flasks, consumables; installed: {})",
                item_speed * 100.0,
                self.shared.item_speed_ready.load(Ordering::Acquire)
            ));
        }
        if let Ok(mut log) = self.shared.haste_log.try_lock() {
            while log.logged < log.seen.len() {
                let anim = log.seen[log.logged];
                log.logged += 1;
                if crate::haste::is_attack_anim(anim) {
                    self.attack_logged += 1;
                    self.session.status(format!(
                        "Attack animation {anim} sped up by {:.1}%",
                        attack_speed * 100.0
                    ));
                } else {
                    self.session
                        .status(format!("Item animation {anim} sped up by {:.1}%", item_speed * 100.0));
                }

            }
        }
        let modal = self.shared.modal.load(Ordering::Acquire);
        let proc_context = ProcContext {
            character_level: snapshot.level,
            attributes: combat.attributes,
            ng_plus: snapshot.ng_plus,
            boss_fight: snapshot.boss_fight_active,
            dodge_started,
            hurt_share,
            damage_rates: snapshot.damage_rates.unwrap_or([1.0; 5]),
        };
        let (proc_summary, proc_status) = if !modal && scan.ready {
            unsafe {
                self.effects.run_procs(
                    &self.shared.library,
                    &self.effective_levels,
                    &combat,
                    &scan,
                    &mut self.world,
                    &proc_context,
                    now,
                    self.pursuit.window(),
                )
            }
        } else {
            Default::default()
        };
        // 3.3 No Quarter: spell damage for a victim the game still guards.
        for (handle, damage, at) in &proc_summary.deliveries {
            self.pursuit.queue(*handle, *damage, *at);
        }
        // Evidence for hit/kill detection (0.4.2: the dragon fight showed none).
        if scan.ready && !self.scan_announced {
            self.scan_announced = true;
            self.session.status(format!(
                "World scan active: {} characters read from {}, {} alive within 60 m",
                scan.characters_read,
                if scan.from_actor_list { "the game's actor list" } else { "the character sets" },
                scan.foes.len()
            ));
        }
        for line in &scan.diagnostics {
            self.session.status(format!("World: {line}"));
        }
        self.combat_log.0 += u32::from(scan.hits);
        self.combat_log.1 += scan.kills.len() as u32;
        self.combat_log.2 = self.combat_log.2.max(scan.streak);
        if self.combat_log.3.elapsed() >= Duration::from_secs(60) {
            if scan.ready {
                self.session.status(format!(
                    "World: {} characters ({}), {} alive within 60 m; {} confirmed hits, {} kills in the last minute, best streak {}",
                    scan.characters_read,
                    if scan.from_actor_list { "actor list" } else { "character sets" },
                    scan.foes.len(),
                    self.combat_log.0,
                    self.combat_log.1,
                    self.combat_log.2
                ));
            }
            self.combat_log = (0, 0, 0, now);
        }
        if self.reflex_log.1.elapsed() >= Duration::from_secs(60) {
            let (before, _) = self.reflex_log;
            let t = self.reflexes.totals;
            let delta = [
                (t.perfect_dodges - before.perfect_dodges, "perfect dodges"),
                (t.blocks - before.blocks, "blocks"),
                (t.guard_counters - before.guard_counters, "guard counters"),
                (t.criticals - before.criticals, "criticals"),
                (t.stance_breaks - before.stance_breaks, "stance breaks"),
                (t.status_procs - before.status_procs, "status procs"),
                (t.parries - before.parries, "parries"),
                (t.backstabs - before.backstabs, "backstabs"),
                (t.executions - before.executions, "executions"),
                (t.rampages - before.rampages, "rampages"),

            ];
            if delta.iter().any(|(n, _)| *n > 0) {
                let parts: Vec<String> = delta.iter().map(|(n, what)| format!("{n} {what}")).collect();
                self.session
                    .status(format!("Reflexes in the last minute: {}", parts.join(", ")));
            }
            self.reflex_log = (t, now);
        }
        if proc_summary.spawned > 0 && self.last_proc_log.elapsed() >= Duration::from_secs(30) {
            self.session.status(format!(
                "Procs: {} bullets spawned so far ({} failed, last code {:?})",
                self.effects.procs().total_spawned,
                self.effects.procs().total_failed,
                self.effects.procs().last_error_code
            ));
            self.last_proc_log = now;
        }
        if !proc_summary.failures.is_empty() && self.proc_failures_logged < 20 {
            self.proc_failures_logged += 1;
            self.session
                .status(format!("Proc failure: {}", proc_summary.failures.join("; ")));
        }
        // 3.1: say once in the log what your boosts do (talismans, tears,
        // Great Runes, buffs): the overhaul and the card spells use them.
        {
            let b = &snapshot.base_attributes;
            let base = [b.vigor, b.mind, b.endurance, b.strength, b.dexterity, b.intelligence, b.faith, b.arcane];
            let a = &combat.attributes;
            let now_values = [a.vigor, a.mind, a.endurance, a.strength, a.dexterity, a.intelligence, a.faith, a.arcane];
            let boost: [u32; 8] = std::array::from_fn(|i| if base[i] > 0 { now_values[i].saturating_sub(base[i]) } else { 0 });
            let rates = snapshot.damage_rates.unwrap_or([1.0; 5]);
            let percent: [i32; 5] = std::array::from_fn(|i| ((rates[i] - 1.0) * 100.0).round() as i32);
            // 3.2: attributes when they change; damage (it moves with every
            // buff) at most every 2 minutes and only after a real change.
            const NAMES: [&str; 8] = ["Vigor", "Mind", "Endurance", "Strength", "Dexterity", "Intelligence", "Faith", "Arcane"];
            const ELEMENTS: [&str; 5] = ["physical", "magic", "fire", "lightning", "holy"];
            if self.bound.is_some() && self.boosts_logged != Some(boost) {
                self.boosts_logged = Some(boost);
                let stats: Vec<String> = (0..8).filter(|i| boost[*i] > 0).map(|i| format!("{} +{}", NAMES[i], boost[i])).collect();
                self.session.status(format!(
                    "Attribute boosts now: {} (the Attribute Overhaul uses them)",
                    if stats.is_empty() { "none".to_owned() } else { stats.join(", ") }
                ));
            }
            let damage_due = self.damage_logged.is_none_or(|(at, last)| {
                now.duration_since(at) >= Duration::from_secs(120) && last.iter().zip(percent).any(|(a, b)| (a - b).abs() >= 5)
            });
            if self.bound.is_some() && damage_due {
                self.damage_logged = Some((now, percent));
                let damage: Vec<String> = (0..5).filter(|i| percent[*i] != 0).map(|i| format!("{} {:+}%", ELEMENTS[i], percent[i])).collect();
                if !damage.is_empty() {
                    self.session.status(format!("Damage from all your effects now: {} (weapon and card spells)", damage.join(", ")));
                }
            }
        }
        if let Ok(mut hud) = self.shared.hud.try_lock() {
            hud.procs = proc_status;
            hud.intervention_armed = intervention_armed;
            hud.intervention_cooldown = self.effects.intervention_cooldown(now);
            hud.intervention_saves = self.effects.intervention_saves();
            hud.streak = scan.streak;
            hud.recent_kills = scan.recent_kills;
            hud.in_combat = snapshot.boss_fight_active
                || scan.seconds_since_combat.is_some_and(|s| s < 8.0);
            hud.spawned_total = self.effects.procs().total_spawned;
            hud.spawn_failures = self.effects.procs().total_failed;
            hud.last_spawn_error = self.effects.procs().last_error_code;
            hud.level = snapshot.level;
            hud.ng_plus = snapshot.ng_plus;
            hud.hud_ok = snapshot.can_observe && snapshot.normal_hud;
            hud.status_buildup = self.status_buildup.clone();
            hud.tide = self.tide_level;
            hud.runes = snapshot.runes;
            hud.paths = self.path_points;
            hud.banner = self.banner.clone();
            hud.flash = self.flash.as_ref().map(|(f, _)| f.clone());
            hud.attack_speed = attack_speed;
            hud.lifesteal = lifesteal;
            hud.healed = self.healed_total;
            hud.spread = self.tide.spread_total;
            hud.ally_boost = self.ally_boost;
            hud.allies_buffed = self.allies.buffed as u32;
            if !self.hint_shown && view_ready && hud.hud_ok {
                self.hint_shown = true;
                if self.committed_levels.is_empty() {
                    hud.hint = Some((
                        "Ascension: beat a boss to earn your first card.  F7 opens the Guide.".into(),
                        now,
                    ));
                }
            }
            hud.ascension_power = core::total_levels(&self.committed_levels);

            hud.elites_nearby = elite_summary.nearby.clone();
            hud.elites_met = self.elites.total_spawned;
            hud.elites_killed = self.elites.total_killed;
            if elite_summary.embers > 0 {
                hud.ember_gain = Some((elite_summary.embers, now));
            }
            hud.attributes = combat.attributes;
            {
                let b = &snapshot.base_attributes;
                let base = [b.vigor, b.mind, b.endurance, b.strength, b.dexterity, b.intelligence, b.faith, b.arcane];
                let a = &combat.attributes;
                let now_values = [a.vigor, a.mind, a.endurance, a.strength, a.dexterity, a.intelligence, a.faith, a.arcane];
                hud.attribute_boost = std::array::from_fn(|i| if base[i] > 0 { now_values[i].saturating_sub(base[i]) } else { 0 });
            }
            hud.damage_rates = snapshot.damage_rates.unwrap_or([1.0; 5]);
            hud.luck = stat_effects.luck + 0.02 * f64::from(self.tide_level);
            // 3.0: the affixes of the boss you fight (NG+ and Ascended).
            hud.boss_affixes = self.boss_scaler.fight_affixes().to_vec();
            hud.cycle_affixes = self.boss_scaler.affixes().to_vec();
            hud.boss_fight = snapshot.boss_fight_active;
            hud.blood_paid = self.blood_paid;
            hud.skip_enabled = self.fate.skip_rewards;
            if let Some(gain) = self.rune_gain.take() {
                hud.rune_gain = Some(gain);
            }

            hud.reflexes = self.reflexes.totals;
            hud.adaptive_element = self.boss_scaler.adaptive_element();
            hud.item_speed = item_speed;
        }
        let capabilities = self.effects.capabilities();
        if self.bound.is_some() && capabilities != self.advertised_capabilities {
            if self
                .shared
                .tx
                .try_send(Command::Capabilities {
                    epoch: self.epoch,
                    capabilities: capabilities.clone(),
                })
                .is_ok()
            {
                self.advertised_capabilities = capabilities;
            }
        }
        if self.last_telemetry.elapsed() >= Duration::from_secs(1) {
            self.session.snapshot(snapshot);
            self.last_telemetry = now;
        }
    }
}
/// 3.0: `ascended` is a fallen Ascended Boss not yet given to a victory
/// (token, affixes); returns its token when a new victory took it.
fn reconcile_cycle(
    store: &mut Store,
    snapshot: &GameSnapshot,
    bosses: &[BossDefinition],
    ascended: Option<(u64, u8)>,
) -> core::Result<Option<u64>> {
    if !snapshot.can_observe {
        return Ok(None);
    }
    match store.state().cycle() {
        Some(cycle) if cycle > snapshot.ng_plus => {
            return Err(core::CoreError::CycleMismatch {
                stored: cycle,
                observed: snapshot.ng_plus,
            });
        }
        None => {
            let already: Vec<String> = bosses
                .iter()
                .filter(|boss| snapshot.boss_defeated.get(&boss.flag) == Some(&true))
                .map(|boss| boss.id.clone())
                .collect();
            store.transaction(|state| state.baseline(snapshot.ng_plus, &already))?;
        }
        Some(cycle) if cycle < snapshot.ng_plus => {
            let already: Vec<String> = bosses
                .iter()
                .filter(|boss| snapshot.boss_defeated.get(&boss.flag) == Some(&true))
                .map(|boss| boss.id.clone())
                .collect();
            store.transaction(|state| state.baseline(snapshot.ng_plus, &already))?;
        }
        _ => {}
    }
    let fresh: Vec<_> = bosses
        .iter()
        .filter(|boss| {
            snapshot.boss_defeated.get(&boss.flag) == Some(&true)
                && !store.state().has_seen(snapshot.ng_plus, &boss.id)
        })
        .collect();
    if fresh.is_empty() {
        return Ok(None);
    }
    store.transaction(|state| {
        for (index, boss) in fresh.iter().enumerate() {
            // The first new victory takes the fallen Ascended Boss.
            let affixes = if index == 0 { ascended.map_or(0, |(_, n)| n) } else { 0 };
            state.observe_boss_with(snapshot.ng_plus, &boss.id, boss.tier, affixes)?;
        }
        Ok(())
    })?;
    Ok(ascended.map(|(token, _)| token))
}
/// The profile's draw settings plus Arcane's luck (latest snapshot), the
/// Tide's luck, Golden Destiny (Fortune path 10) and the [fate] switches.
fn luck_draw(
    config: &Config,
    snapshot: &Option<(GameSnapshot, Instant)>,
    cursed: &BTreeSet<String>,
    library: &PowerLibrary,
    state: &Progression,
) -> core::DrawConfig {
    let mut draw = config.draw.clone();
    draw.rank_rewards = config.fate.rank_rewards;
    draw.guarantee_evolutions = config.fate.evolutions;
    if !config.fate.curses {
        draw.excluded.extend(cursed.iter().cloned());
    }
    let arcane = snapshot.as_ref().map(|(s, _)| s.attributes.arcane).unwrap_or(0);
    let tide = crate::paths::tide(state.total_levels(), config.world.tide_step);
    draw.luck = crate::attributes::arcane_luck(arcane, config.balance.attribute_strength) + 0.02 * f64::from(tide);
    let points = crate::paths::points(state.levels(), library);
    draw.extra_cards = usize::from(points[crate::paths::Path::Fortune.index()] >= crate::paths::TIERS[1]);
    // 2.0: your Paths pull their cards: +3% weight per point, up to +45%.
    for power in library.powers.iter().filter(|p| !p.hidden) {
        if let Some(path) = power.family {
            let have = points[path.index()];
            // 3.0: Keystones wait for their Path (KEYSTONE_GATE points).
            if power.category == "keystone" && have < crate::paths::KEYSTONE_GATE {
                draw.excluded.insert(power.id.clone());
                continue;
            }
            if have > 0 {
                draw.boosts.insert(power.id.clone(), (1.0 + 0.03 * f64::from(have)).min(1.45));
            }
        }
    }
    draw
}
/// Skip pay (rune multiplier, extra Embers): Fate cards, doubled by Midas
/// (Fortune path 15).
fn skip_bonus(catalog: &Catalog, library: &PowerLibrary, levels: &BTreeMap<String, u8>) -> (f64, u32) {
    let (rate, extra) = core::skip_bonus(catalog, levels);
    let points = crate::paths::points(levels, library);
    if points[crate::paths::Path::Fortune.index()] >= crate::paths::TIERS[2] {
        (rate * 2.0, extra + 2)
    } else {
        (rate, extra)
    }
}

fn require_choice_frame(
    epoch: u64,
    current: u64,
    shared: &SharedRuntime,
    snapshot: &Option<(GameSnapshot, Instant)>,
) -> core::Result<()> {
    if epoch != current
        || epoch != shared.epoch.load(Ordering::Acquire)
        || !shared.safe.load(Ordering::Acquire)
        || !snapshot.as_ref().is_some_and(|(snapshot, time)| {
            snapshot.safe && time.elapsed() < Duration::from_millis(500)
        })
    {
        return Err(core::CoreError::StaleOffer);
    }
    Ok(())
}
fn os_seed() -> core::Result<u64> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            buffer: *mut u8,
            length: u32,
            flags: u32,
        ) -> i32;
    }
    let mut bytes = [0u8; 8];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 8, 2) } < 0 {
        return Err(core::CoreError::Invalid("OS entropy unavailable".into()));
    }
    Ok(u64::from_le_bytes(bytes))
}

/// 1.0.1: copy this character's journal folder (the store's own folder,
/// `state/progression/<hash of the key>`) to `state/backups/<same name>/<unix
/// time>` and keep the newest 10 copies. Only the .json generations are
/// copied: writer.lock is held exclusively by the open store.
fn backup_progression(root: &std::path::Path, source: &std::path::Path) -> Result<Option<String>, String> {
    if !source.is_dir() {
        return Ok(None);
    }
    let key = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("progression folder has no name")?
        .to_owned();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = root.join("state/backups").join(&key);
    let target = base.join(format!("{stamp}"));
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let mut copied = 0;
    for entry in std::fs::read_dir(&source).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "json") {
            if let Some(name) = path.file_name() {
                std::fs::copy(&path, target.join(name)).map_err(|e| e.to_string())?;
                copied += 1;
            }
        }
    }
    let mut copies: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&base)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let n = path.file_name()?.to_str()?.parse::<u64>().ok()?;
            path.is_dir().then_some((n, path))
        })
        .collect();
    copies.sort();
    while copies.len() > 10 {
        let (_, old) = copies.remove(0);
        let _ = std::fs::remove_dir_all(old);
    }
    Ok(Some(format!("state/backups/{key}/{stamp} ({copied} files)")))
}
