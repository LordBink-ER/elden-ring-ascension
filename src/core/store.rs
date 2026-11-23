//! Immutable generation journal. A process-held OS file lock serializes writers.
//! File contents are flushed before rename; the containing directory is also
//! synced on Unix. Existing generation files are never replaced on Windows.
use super::{CoreError, Progression, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const SCHEMA: u32 = 1;
const KEEP_GENERATIONS: usize = 16;
const MAX_FILE_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: u32,
    generation: u64,
    state: Progression,
    sha256: String,
}

pub struct Store {
    directory: PathBuf,
    _lock: File,
    generation: u64,
    state: Progression,
    poisoned: bool,
    #[cfg(test)]
    fail_after_rename: bool,
}

fn hash(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}
fn state_hash(generation: u64, state: &Progression) -> Result<String> {
    Ok(hash(&serde_json::to_vec(&(SCHEMA, generation, state))?))
}
fn directory(root: &Path, character_id: &str) -> PathBuf {
    root.join(hash(character_id.as_bytes()))
}
fn generations(path: &Path) -> Result<Vec<u64>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "json") {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.len() != 20 || !stem.bytes().all(|b| b.is_ascii_digit()) {
                return Err(CoreError::Corrupt("unexpected journal filename".into()));
            }
            let generation: u64 = stem
                .parse()
                .map_err(|_| CoreError::Corrupt("invalid journal generation".into()))?;
            if generation == 0 {
                return Err(CoreError::Corrupt("journal generation zero".into()));
            }
            result.push(generation);
        }
    }
    result.sort_unstable();
    Ok(result)
}
fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join("writer.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock
            || e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            CoreError::Locked
        } else {
            CoreError::Io(e)
        }
    })?;
    Ok(file)
}

#[cfg(not(windows))]
fn durable_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn durable_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // No MOVEFILE_REPLACE_EXISTING: immutable generations must never overwrite.
    // https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // The buffers are nul-terminated and remain live for the synchronous call.
    let success = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Store {
    /// Explicit creation only. Existing journals, including corrupt journals,
    /// cannot be overwritten with a new character baseline.
    pub fn create(root: &Path, initial: Progression) -> Result<Self> {
        initial.validate()?;
        let directory = directory(root, initial.character_id());
        fs::create_dir_all(&directory)?;
        let lock = lock(&directory)?;
        if !generations(&directory)?.is_empty() {
            return Err(CoreError::AlreadyExists);
        }
        let mut store = Self {
            directory,
            _lock: lock,
            generation: 0,
            state: initial,
            poisoned: false,
            #[cfg(test)]
            fail_after_rename: false,
        };
        store.write_generation(&store.state.clone())?;
        Ok(store)
    }

    /// The newest committed generation must pass both SHA256 and semantic
    /// validation. Never silently falls back, rolls back powers, or resets.
    /// Retained older generations are for explicit, reviewed recovery.
    pub fn open(root: &Path, character_id: &str) -> Result<Self> {
        let directory = directory(root, character_id);
        if !directory.is_dir() {
            return Err(CoreError::NotFound);
        }
        let lock = lock(&directory)?;
        let generation = generations(&directory)?
            .last()
            .copied()
            .ok_or(CoreError::NotFound)?;
        let file = directory.join(format!("{generation:020}.json"));
        if fs::metadata(&file)?.len() > MAX_FILE_SIZE {
            return Err(CoreError::Corrupt("journal file exceeds size limit".into()));
        }
        let envelope: Envelope = serde_json::from_slice(&fs::read(file)?)
            .map_err(|e| CoreError::Corrupt(format!("malformed journal: {e}")))?;
        if envelope.schema != SCHEMA
            || envelope.generation != generation
            || envelope.state.character_id() != character_id
            || envelope.sha256 != state_hash(generation, &envelope.state)?
        {
            return Err(CoreError::Corrupt(
                "journal schema, identity, generation or checksum mismatch".into(),
            ));
        }
        envelope.state.validate()?;
        Ok(Self {
            directory,
            _lock: lock,
            generation,
            state: envelope.state,
            poisoned: false,
            #[cfg(test)]
            fail_after_rename: false,
        })
    }

    pub fn state(&self) -> &Progression {
        &self.state
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Mutates a clone, validates it, then durably commits it before adopting
    /// it in memory or returning the result. Disk errors cannot grant a power.
    /// A no-op transaction does not create a generation.
    pub fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut Progression) -> Result<T>,
    ) -> Result<T> {
        if self.poisoned {
            return Err(CoreError::Corrupt("previous commit outcome is uncertain; close and reopen the journal before continuing".into()));
        }
        let mut candidate = self.state.clone();
        let output = operation(&mut candidate)?;
        candidate.validate()?;
        if candidate != self.state {
            self.write_generation(&candidate)?;
            self.state = candidate;
        }
        Ok(output)
    }

    fn write_generation(&mut self, state: &Progression) -> Result<()> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| CoreError::Invalid("journal generation overflow".into()))?;
        let envelope = Envelope {
            schema: SCHEMA,
            generation,
            state: state.clone(),
            sha256: state_hash(generation, state)?,
        };
        let bytes = serde_json::to_vec_pretty(&envelope)?;
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(CoreError::Invalid("journal size limit exceeded".into()));
        }
        // A crashed partial .tmp cannot shadow the final generation. Retain it
        // for diagnosis and choose a unique temp filename for the retry.
        let mut suffix = 0u32;
        let (temp_path, mut temp) = loop {
            let p = self.directory.join(format!(
                "{generation:020}.{}.{}.tmp",
                std::process::id(),
                suffix
            ));
            match OpenOptions::new().write(true).create_new(true).open(&p) {
                Ok(f) => break (p, f),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    suffix = suffix.checked_add(1).ok_or_else(|| {
                        CoreError::Invalid("too many temporary journal files".into())
                    })?;
                }
                Err(e) => return Err(e.into()),
            }
        };
        temp.write_all(&bytes)?;
        temp.sync_all()?;
        drop(temp);
        let final_path = self.directory.join(format!("{generation:020}.json"));
        if final_path.exists() {
            return Err(CoreError::Corrupt("next generation already exists".into()));
        }
        if let Err(e) = durable_rename(&temp_path, &final_path) {
            // Some filesystem errors can report failure after a move occurred.
            // Reopen to determine the durable outcome instead of guessing.
            if final_path.exists() {
                self.poisoned = true;
            }
            return Err(e.into());
        }
        #[cfg(test)]
        if self.fail_after_rename {
            self.fail_after_rename = false;
            self.poisoned = true;
            return Err(std::io::Error::other("injected post-rename sync failure").into());
        }
        #[cfg(unix)]
        if let Err(e) = File::open(&self.directory).and_then(|f| f.sync_all()) {
            self.poisoned = true;
            return Err(e.into());
        }
        self.generation = generation;
        // Retain a bounded recovery journal. Cleanup failure cannot invalidate
        // an already committed selection; retry on a later transaction.
        if let Ok(gens) = generations(&self.directory) {
            let remove = gens.len().saturating_sub(KEEP_GENERATIONS);
            for g in gens.into_iter().take(remove) {
                let _ = fs::remove_file(self.directory.join(format!("{g:020}.json")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::BossTier;

    #[test]
    fn uncertain_post_rename_commit_requires_reopen() {
        let root = std::env::temp_dir().join(format!(
            "ascension-uncertain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut initial = Progression::new("fixture-post-rename", 5).unwrap();
        initial.baseline(0, &[]).unwrap();
        let mut store = Store::create(&root, initial.clone()).unwrap();
        store.fail_after_rename = true;
        assert!(
            store
                .transaction(|s| s.observe_boss(0, "victory", BossTier::Minor))
                .is_err()
        );
        assert_eq!(store.state(), &initial);
        assert!(matches!(
            store.transaction(|_| Ok(())),
            Err(CoreError::Corrupt(_))
        ));
        drop(store);
        let store = Store::open(&root, initial.character_id()).unwrap();
        assert!(store.state().has_seen(0, "victory"));
        assert_eq!(store.state().pending_count(), 1);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
