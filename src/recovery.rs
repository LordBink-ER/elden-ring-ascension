//! One explicitly reviewed incident, not automatic rewards from diagnostic logs.
//! Before any marker mutation, the identity worker attests an exact checksummed
//! slot body and binds the evidence to its durable PREPARED UUID. Only that UUID
//! may recover the victory after baseline. This module never writes an SL2.
use crate::{
    core::{BossTier, Store},
    identity::{BindingIntent, BindingScope, BoundIdentity},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
const INCIDENT: &str = "observed-grafted-scion-20261029";
const SLOT: u8 = 4; // Native/save index, zero based.
const CYCLE: u32 = 0;
const BOSS: &str = "boss_10010800";
const FLAG: u32 = 10_010_800;
const SLOT_SHA256: &str = "333e242e568b477488eed1720b79cd3b25d4fa3ee44e274a71882052f7d94d6b";
const LOG_BYTES: u64 = 8_457_479;
const LOG_SHA256: &str = "d6d810ebfb8eef33c1777f186446038da6a1583f8b5d59d2b7faf61ab3215d7a";
const PREVIOUS_LINE: usize = 2_424;
const VICTORY_LINE: usize = 2_425;
const PREVIOUS_SHA256: &str = "aceb709f416c3b8e1b6565c01ddf101a1b5fbcae163b861ff57265cafd268528";
const VICTORY_SHA256: &str = "a0473e338bf77062580dedaf2f99e6ccf474339597f8b98dd0aabda3b9a8bb9f";
const CONTINUATION: &str = "observed-grafted-scion-20261029-continuation-021";
const CONTINUATION_SLOT_SHA256: &str =
    "d434700cf21adaac8ef591e2b4d82954f057f832dbb15b6e7647a82a97cc79f5";
const CONTINUATION_BOUND_SHA256: &str =
    "2a909c34e07c72a5a5c78d2b159b03b5e684dba3a3ca17209853148b9477900c";
const CONTINUATION_INTENT_SHA256: &str =
    "fddf638148a16dc552f6de160f041cd513b2ebcd77e8f5951c21d961b50ef094";
const CONTINUATION_LOG_BYTES: u64 = 8_847_485;
const CONTINUATION_LOG_SHA256: &str =
    "3aa6b6d4a8e02513b2c5077b6f2ed4c4c5ae8bdb897ed1b01bd172b79e25301a";
const MAX_RECORD_BYTES: u64 = 64 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    log_prefix_bytes: u64,
    log_prefix_sha256: String,
    previous_line: usize,
    previous_line_sha256: String,
    victory_line: usize,
    victory_line_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    incident_id: String,
    scope: BindingScope,
    slot: u8,
    cycle: u32,
    boss_id: String,
    boss_flag: u32,
    tier: BossTier,
    slot_body_sha256: String,
    /// Audit provenance only. Other slots may change without invalidating the
    /// independently checked target slot body, so this is not a match key.
    source_save_sha256: String,
    evidence: Evidence,
}
impl Receipt {
    fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.schema != 1
            || self.incident_id != INCIDENT
            || self.slot != SLOT
            || self.cycle != CYCLE
            || self.boss_id != BOSS
            || self.boss_flag != FLAG
            || self.tier != BossTier::Minor
            || self.slot_body_sha256 != SLOT_SHA256
            || !is_digest(&self.source_save_sha256)
            || self.evidence.log_prefix_bytes != LOG_BYTES
            || self.evidence.log_prefix_sha256 != LOG_SHA256
            || self.evidence.previous_line != PREVIOUS_LINE
            || self.evidence.previous_line_sha256 != PREVIOUS_SHA256
            || self.evidence.victory_line != VICTORY_LINE
            || self.evidence.victory_line_sha256 != VICTORY_SHA256
        {
            return Err("recovery receipt does not match the reviewed incident".into());
        }
        Ok(())
    }

    fn verify_log(&self, root: &Path) -> Result<()> {
        // The log is append-only during normal use. Hash only the reviewed
        // prefix so later telemetry cannot invalidate an already captured event.
        let mut bytes = Vec::with_capacity(LOG_BYTES as usize);
        File::open(root.join("logs/runtime.jsonl"))
            .map_err(|e| format!("recovery evidence log is unavailable: {e}"))?
            .take(LOG_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 != LOG_BYTES || digest(&bytes) != LOG_SHA256 {
            return Err(
                "recovery evidence log prefix changed; automatic guessing forbidden".into(),
            );
        }
        let lines: Vec<_> = bytes.split_inclusive(|b| *b == b'\n').collect();
        let previous = lines
            .get(PREVIOUS_LINE - 1)
            .ok_or("recovery previous snapshot is missing")?;
        let victory = lines
            .get(VICTORY_LINE - 1)
            .ok_or("recovery victory snapshot is missing")?;
        if digest(previous) != PREVIOUS_SHA256 || digest(victory) != VICTORY_SHA256 {
            return Err("recovery snapshot evidence does not match its recorded hash".into());
        }
        let previous: Value = serde_json::from_slice(previous).map_err(|e| e.to_string())?;
        let victory: Value = serde_json::from_slice(victory).map_err(|e| e.to_string())?;
        let before = &previous["snapshot"];
        let after = &victory["snapshot"];
        // The old offline/can_observe diagnostic gate was itself faulty. Its
        // booleans cannot disprove this specifically reviewed raw flag edge.
        // They are never relaxed for general log replay or normal observation.
        if previous["event"] != "snapshot"
            || victory["event"] != "snapshot"
            || previous["time_ms"].as_u64() != Some(1_793_238_734_342)
            || victory["time_ms"].as_u64() != Some(1_793_238_734_442)
            || before["loaded"] != true
            || after["loaded"] != true
            || before["identity"] != after["identity"]
            || before["identity"]["save_slot"].as_u64() != Some(SLOT as u64)
            || before["ng_plus"].as_u64() != Some(CYCLE as u64)
            || after["ng_plus"].as_u64() != Some(CYCLE as u64)
            || before["player_address"].as_u64().is_none_or(|p| p == 0)
            || before["player_address"] != after["player_address"]
            || before["play_time_ms"].as_u64() != Some(136_455)
            || after["play_time_ms"].as_u64() != Some(136_551)
            || before["boss_defeated"]["10010800"] != false
            || after["boss_defeated"]["10010800"] != true
        {
            return Err("recovery evidence is not the reviewed continuous victory edge".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundReceipt {
    schema: u32,
    receipt: Receipt,
    uuid: [u8; 16],
    character_key: String,
    prepared_source_save_sha256: String,
    marker_manifest_sha256: String,
}

/// One additional, manually reviewed exact fingerprint after the 0.2.1
/// pre-grant validation failure. This extends evidence, never rewrites its UUID.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Continuation {
    schema: u32,
    incident_id: String,
    bound_payload_sha256: String,
    prepared_payload_sha256: String,
    original_slot_body_sha256: String,
    continued_slot_body_sha256: String,
    source_save_sha256: String,
    log_prefix_bytes: u64,
    log_prefix_sha256: String,
}

fn matches_slot(
    root: &Path,
    bound: &BoundReceipt,
    intent: &BindingIntent,
    source_slot_body_sha256: &str,
) -> Result<bool> {
    if source_slot_body_sha256 == bound.receipt.slot_body_sha256 {
        return Ok(true);
    }
    if source_slot_body_sha256 != CONTINUATION_SLOT_SHA256 {
        return Ok(false);
    }
    let path = root
        .join("state/recovery")
        .join(format!("{CONTINUATION}.json"));
    let Some(proof) = read_record::<Continuation>(&path)? else {
        return Ok(false);
    };
    if proof.schema != 1
        || proof.incident_id != CONTINUATION
        || proof.bound_payload_sha256 != CONTINUATION_BOUND_SHA256
        || proof.prepared_payload_sha256 != CONTINUATION_INTENT_SHA256
        || proof.original_slot_body_sha256 != SLOT_SHA256
        || proof.continued_slot_body_sha256 != CONTINUATION_SLOT_SHA256
        || !is_digest(&proof.source_save_sha256)
        || proof.log_prefix_bytes != CONTINUATION_LOG_BYTES
        || proof.log_prefix_sha256 != CONTINUATION_LOG_SHA256
        || digest(&serde_json::to_vec(bound).map_err(|e| e.to_string())?)
            != proof.bound_payload_sha256
        || digest(&serde_json::to_vec(intent).map_err(|e| e.to_string())?)
            != proof.prepared_payload_sha256
    {
        return Err(
            "reviewed recovery continuation conflicts with its original UUID/intent".into(),
        );
    }
    let mut bytes = Vec::with_capacity(CONTINUATION_LOG_BYTES as usize);
    File::open(root.join("logs/runtime.jsonl"))
        .map_err(|e| e.to_string())?
        .take(CONTINUATION_LOG_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != CONTINUATION_LOG_BYTES || digest(&bytes) != CONTINUATION_LOG_SHA256 {
        return Err("reviewed recovery continuation log prefix changed".into());
    }
    Ok(true)
}
impl BoundReceipt {
    fn validate(&self) -> Result<()> {
        self.receipt.validate()?;
        if self.schema != 1
            || self.uuid[6] >> 4 != 4
            || self.uuid[8] >> 6 != 2
            || self.character_key != self.receipt.scope.character_key(&self.uuid)
            || !is_digest(&self.prepared_source_save_sha256)
            || !is_digest(&self.marker_manifest_sha256)
        {
            return Err("bound recovery receipt failed identity validation".into());
        }
        Ok(())
    }
}

fn receipt_path(root: &Path) -> PathBuf {
    root.join("local/recovery").join(format!("{INCIDENT}.json"))
}
fn bound_path(root: &Path) -> PathBuf {
    root.join("state/recovery").join(format!("{INCIDENT}.json"))
}

/// Identity-worker hook: call AFTER PREPARED was durably written and BEFORE
/// returning PreparedBinding or allowing any native inventory mutation.
///
/// `source_slot_body_sha256` must come from the SAME validated save bytes used
/// to establish occupied slot, matching Steam account and empty marker token.
/// The caller holds its existing account/profile identity writer lock.
/// A mismatch is an ordinary no-op: never rebind using a name, slot or stats.
pub fn bind_prepared(
    root: &Path,
    intent: &BindingIntent,
    source_slot_body_sha256: &str,
) -> Result<bool> {
    if intent.schema != 1 || intent.phase != "PREPARED" {
        return Err("recovery requires a durable fresh PREPARED binding".into());
    }
    if let Some(bound) = read_record::<BoundReceipt>(&bound_path(root))? {
        bound.validate()?;
        // Immutable once assigned. A deleted/recreated character in this slot
        // receives another UUID and can never inherit this receipt.
        return Ok(bound.receipt.scope == intent.scope
            && bound.uuid == intent.uuid
            && bound.receipt.slot == intent.slot
            && bound.marker_manifest_sha256 == intent.manifest_sha256
            && matches_slot(root, &bound, intent, source_slot_body_sha256)?);
    }
    let Some(receipt) = read_record::<Receipt>(&receipt_path(root))? else {
        return Ok(false);
    };
    receipt.validate()?;
    if receipt.scope != intent.scope
        || receipt.slot != intent.slot
        || receipt.slot_body_sha256 != source_slot_body_sha256
    {
        return Ok(false);
    }
    receipt.verify_log(root)?;
    let bound = BoundReceipt {
        schema: 1,
        character_key: receipt.scope.character_key(&intent.uuid),
        receipt,
        uuid: intent.uuid,
        prepared_source_save_sha256: intent.source_save_sha256.clone(),
        marker_manifest_sha256: intent.manifest_sha256.clone(),
    };
    bound.validate()?;
    write_new_record(&bound_path(root), &bound)?;
    Ok(true)
}

/// Narrow crash recovery BEFORE rotating a previous PREPARED intent. A crash
/// may have committed the immutable receipt before returning PreparedBinding.
/// Reuse that UUID only if the still-unmarked checksummed slot is byte-for-byte
/// identical to the original or its one reviewed continuation, and the durable
/// intent/codec agree. This is not a general continuation or fuzzy match policy.
/// This hook does not authorize any recovery of an ACTIVE or unknown UUID.
pub fn resume_prepared(
    root: &Path,
    prior: &BindingIntent,
    source_slot_body_sha256: &str,
) -> Result<bool> {
    let Some(bound) = read_record::<BoundReceipt>(&bound_path(root))? else {
        return Ok(false);
    };
    bound.validate()?;
    Ok(prior.schema == 1
        && prior.phase == "PREPARED"
        && prior.scope == bound.receipt.scope
        && prior.slot == bound.receipt.slot
        && prior.uuid == bound.uuid
        && prior.manifest_sha256 == bound.marker_manifest_sha256
        && matches_slot(root, &bound, prior, source_slot_body_sha256)?)
}

/// Progression-worker hook: call after normal reconcile_cycle, before preparing
/// the next offer. Retry on later observed snapshots if baseline is not ready.
/// `binding` must already have passed IdentityService's persisted-token proof.
/// Pending/selected boss+cycle is the durable consumption record; no separate
/// consumed flag can race the progression commit or manufacture duplicates.
pub fn apply_bound(
    root: &Path,
    binding: &BoundIdentity,
    store: &mut Store,
    cycle: u32,
    boss_defeated: &BTreeMap<u32, bool>,
) -> Result<bool> {
    let Some(bound) = read_record::<BoundReceipt>(&bound_path(root))? else {
        return Ok(false);
    };
    bound.validate()?;
    if bound.receipt.scope != binding.scope
        || bound.uuid != binding.uuid
        || bound.receipt.slot != binding.slot
        || bound.character_key != binding.character_key
    {
        return Ok(false);
    }
    if store.state().character_id() != binding.character_key {
        return Err("recovery refused a progression store for another identity".into());
    }
    if store
        .state()
        .has_reward(bound.receipt.cycle, &bound.receipt.boss_id)
    {
        return Ok(false);
    }
    if cycle != bound.receipt.cycle
        || store.state().cycle() != Some(cycle)
        || boss_defeated.get(&bound.receipt.boss_flag) != Some(&true)
    {
        return Ok(false);
    }
    store
        .transaction(|state| {
            state.recover_observed_victory(cycle, &bound.receipt.boss_id, bound.receipt.tier)
        })
        .map_err(|e| format!("durable observed-victory recovery failed: {e}"))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: u32,
    payload_json: String,
    sha256: String,
}
fn read_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_RECORD_BYTES {
        return Err("recovery record exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("recovery record exceeds size limit".into());
    }
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if envelope.schema != 1 || digest(envelope.payload_json.as_bytes()) != envelope.sha256 {
        return Err("recovery record integrity failure; automatic reset forbidden".into());
    }
    serde_json::from_str(&envelope.payload_json)
        .map(Some)
        .map_err(|e| e.to_string())
}
fn write_new_record(path: &Path, payload: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or("recovery record has no directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    {
        // Flush directory entries as well when state/recovery was just created.
        for directory in parent.ancestors().take(3) {
            File::open(directory)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
    }
    let payload_json = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let envelope = Envelope {
        schema: 1,
        sha256: digest(payload_json.as_bytes()),
        payload_json,
    };
    let bytes = serde_json::to_vec_pretty(&envelope).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".{INCIDENT}-{}-{}-{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    durable_move_new(&temporary, path)?;
    Ok(())
}

#[cfg(windows)]
fn durable_move_new(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
    // MOVEFILE_WRITE_THROUGH, deliberately without REPLACE_EXISTING.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) } == 0 {
        return Err(format!(
            "durable immutable recovery binding failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn durable_move_new(source: &Path, destination: &Path) -> Result<()> {
    // link is atomic and refuses replacement. A crash may leave the temporary
    // link, which is harmless; an incomplete final JSON is never published.
    fs::hard_link(source, destination).map_err(|e| e.to_string())?;
    let parent = destination.parent().ok_or("missing recovery directory")?;
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::remove_file(source).map_err(|e| e.to_string())?;
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn is_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
