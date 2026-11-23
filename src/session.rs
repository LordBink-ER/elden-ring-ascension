//! Diagnostic worker. No file I/O or serialization on a game/render task.
use crate::game::GameSnapshot;
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct RuntimeView {
    pub phase: String,
    pub error: Option<String>,
    pub snapshot: Option<GameSnapshot>,
    pub samples: u64,
    pub flag_transitions: u64,
}
pub enum Event {
    Status(String),
    Error(String),
    Snapshot(GameSnapshot),
}

#[derive(Clone)]
pub struct Session {
    pub view: Arc<Mutex<RuntimeView>>,
    tx: SyncSender<Event>,
}
impl Session {
    pub fn start(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(root.join("logs")).map_err(|e| e.to_string())?;
        // Keep one previous log; a session log never grows past what one
        // evening of play writes (it reached 126 MB before 0.3).
        let current = root.join("logs/runtime.jsonl");
        if fs::metadata(&current).is_ok_and(|m| m.len() > 16 * 1024 * 1024) {
            let _ = fs::rename(&current, root.join("logs/runtime.previous.jsonl"));
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("logs/runtime.jsonl"))
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(16);
        let view = Arc::new(Mutex::new(RuntimeView {
            phase: "Starting native runtime".into(),
            ..Default::default()
        }));
        let worker_view = view.clone();
        std::thread::Builder::new().name("ascension-storage".into()).spawn(move || {
            let mut log = log;
            let mut previous: Option<GameSnapshot> = None;
            let mut logged_snapshot: Option<(GameSnapshot, u128)> = None;
            // Recently written status lines: alternating retry messages are
            // written once per minute with a repeat count instead of 10 Hz.
            let mut recent: Vec<(String, u128, u32)> = Vec::new();
            while let Ok(event) = rx.recv() {
                let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
                let record = match event {
                    Event::Status(message) => {
                        if let Ok(mut view) = worker_view.lock() { view.phase = message.clone(); }
                        recent.retain(|(_, at, _)| time.saturating_sub(*at) < 60_000);
                        if let Some(entry) = recent.iter_mut().find(|(m, _, _)| *m == message) {
                            entry.2 += 1;
                            continue;
                        }
                        let repeats: u32 = 0;
                        recent.push((message.clone(), time, repeats));
                        if recent.len() > 16 { recent.remove(0); }
                        json!({"time_ms":time,"event":"status","message":message})
                    }
                    Event::Error(message) => {
                        if let Ok(mut view) = worker_view.lock() { view.error = Some(message.clone()); }
                        json!({"time_ms":time,"event":"error","message":message})
                    }
                    Event::Snapshot(snapshot) => {
                        // This diagnostic comparison is evidence only: it does not grant rewards.
                        // No delta is compared across character, NG+ or unready flag storage.
                        let changes: Vec<_> = previous.as_ref().filter(|old| old.can_observe && snapshot.can_observe && old.identity == snapshot.identity && old.ng_plus == snapshot.ng_plus)
                            .map(|old| snapshot.boss_defeated.iter().filter(|(id, state)| **state && old.boss_defeated.get(id) == Some(&false)).map(|(id, _)| *id).collect()).unwrap_or_default();
                        if let Ok(mut view) = worker_view.lock() {
                            view.samples += 1;
                            view.flag_transitions += changes.len() as u64;
                            view.snapshot = Some(snapshot.clone());
                        }
                        // Telemetry is separate from core::Store's durable progression journal.
                        // A partial telemetry file after power loss cannot reset progression.
                        if let Err(error) = write_snapshot(&root, &snapshot) {
                            if let Ok(mut view) = worker_view.lock() { view.error = Some(format!("diagnostic snapshot write failed: {error}")); }
                        }
                        previous = Some(snapshot.clone());
                        // The log keeps a full snapshot once a minute, on any
                        // state change and on boss transitions; the latest one
                        // is always in latest-snapshot.json.
                        let changed = logged_snapshot.as_ref().is_none_or(|(old, at)| {
                            time.saturating_sub(*at) >= 60_000
                                || old.loaded != snapshot.loaded
                                || old.safe != snapshot.safe
                                || old.can_observe != snapshot.can_observe
                                || old.ng_plus != snapshot.ng_plus
                                || old.identity.as_ref().map(|i| (&i.name, i.save_slot, i.verified))
                                    != snapshot.identity.as_ref().map(|i| (&i.name, i.save_slot, i.verified))
                                || old.boss_fight_active != snapshot.boss_fight_active
                        });
                        if !changed && changes.is_empty() {
                            continue;
                        }
                        logged_snapshot = Some((snapshot.clone(), time));
                        let mut compact = serde_json::to_value(&snapshot).unwrap_or_default();
                        if let Some(map) = compact.as_object_mut() {
                            map.remove("boss_defeated");
                            map.insert("bosses_defeated".into(), json!(snapshot.boss_defeated.values().filter(|v| **v).count()));
                        }
                        json!({"time_ms":time,"event":"snapshot","boss_rising_edges":changes,"snapshot":compact})
                    }
                };
                if let Err(error) = writeln!(log,"{record}").and_then(|_|log.flush()) {
                    if let Ok(mut view) = worker_view.lock() { view.error = Some(format!("runtime log write failed: {error}")); }
                }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { view, tx })
    }
    pub fn status(&self, message: impl Into<String>) {
        let _ = self.tx.try_send(Event::Status(message.into()));
    }
    pub fn error(&self, message: impl Into<String>) {
        let _ = self.tx.try_send(Event::Error(message.into()));
    }
    /// Snapshot telemetry may drop a sample under I/O load; it is not the reward journal.
    pub fn snapshot(&self, snapshot: GameSnapshot) {
        let _ = self.tx.try_send(Event::Snapshot(snapshot));
    }
}
fn write_snapshot(root: &std::path::Path, snapshot: &GameSnapshot) -> Result<(), String> {
    let file = File::create(root.join("logs/latest-snapshot.json")).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(file, snapshot).map_err(|e| e.to_string())
}
