//! Mod-owned, NG+-independent identity. Game operations are main-thread only;
//! journal/proof work is performed by IdentityService's storage worker.
//! Implemented from source contracts; not verified in the running game.
use eldenring::{
    cs::{
        BlockId, EquipParamGoods, GameMan, ItemCategory, ItemLotParam_map, SoloParamRepository,
        TalkScript, WorldChrMan,
    },
    ez_state::EzStateValue,
};
use fromsoftware_shared::FromStatic;
use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

pub type Result<T> = std::result::Result<T, String>;
const FRAGMENTS: usize = 16;
const MAX_GOODS_ID: u32 = 0x7f_ffff;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerManifest {
    pub schema: u32,
    pub payload_goods: [u32; FRAGMENTS],
    pub commit_good: u32,
    pub init_item_lots: [u32; FRAGMENTS],
    pub commit_item_lot: u32,
    pub goods_itemlot_category: u8,
    pub quantity_offset: u32,
    pub commit_quantity: u32,
    pub goods_template_id: u32,
    pub itemlot_template_id: u32,
    pub source_regulation_sha256: String,
    pub generated_regulation_sha256: String,
    pub source_regulation_version: u32,
}
impl MarkerManifest {
    pub fn validate(&self) -> Result<()> {
        let mut goods = BTreeSet::new();
        let mut lots = BTreeSet::new();
        if self.schema != 1
            || self.goods_itemlot_category != 1
            || self.quantity_offset != 1
            || self.commit_quantity != 1
            || self.goods_template_id == 0
            || self.itemlot_template_id == 0
            || self.source_regulation_version == 0
            || self
                .payload_goods
                .iter()
                .chain([&self.commit_good])
                .any(|id| *id == 0 || *id > MAX_GOODS_ID || !goods.insert(*id))
            || self
                .init_item_lots
                .iter()
                .chain([&self.commit_item_lot])
                .any(|id| *id == 0 || *id > i32::MAX as u32 || !lots.insert(*id))
            || !is_digest(&self.source_regulation_sha256)
            || !is_digest(&self.generated_regulation_sha256)
        {
            return Err("invalid identity marker manifest or unsupported Goods handle ID".into());
        }
        Ok(())
    }
    /// Layout digest intentionally excludes regulation file hashes: updating
    /// balance rows must not sever an existing character's UUID binding.
    pub fn digest(&self) -> String {
        digest(
            &serde_json::to_vec(&(
                self.schema,
                self.payload_goods,
                self.commit_good,
                self.init_item_lots,
                self.commit_item_lot,
            ))
            .expect("fixed identity manifest serializes"),
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BindingScope {
    pub steam_account: String,
    pub profile_namespace: String,
}
impl BindingScope {
    pub fn validate(&self) -> Result<()> {
        if self
            .steam_account
            .parse::<u64>()
            .ok()
            .is_none_or(|n| n == 0)
            || self.profile_namespace.is_empty()
            || self.profile_namespace.len() > 128
            || self.profile_namespace.chars().any(char::is_control)
        {
            return Err("invalid identity account/profile scope".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        digest(&serde_json::to_vec(self).expect("fixed identity scope serializes"))
    }
    pub fn character_key(&self, uuid: &[u8; FRAGMENTS]) -> String {
        digest(&serde_json::to_vec(&(self, uuid)).expect("fixed identity key serializes"))
    }
}

/// A per-world-load guard, never a persistent birth key. Root increments epoch
/// on every player unload/switch; do not reuse a prepared reply in another epoch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAnchor {
    pub epoch: u64,
    pub slot: u8,
    pub player_data_address: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryToken {
    pub quantities: [u32; FRAGMENTS],
    pub commit_quantity: u32,
}
impl InventoryToken {
    pub fn is_empty(&self) -> bool {
        self.commit_quantity == 0 && self.quantities.iter().all(|q| *q == 0)
    }
    pub fn decode(&self) -> Result<[u8; FRAGMENTS]> {
        if self.commit_quantity != 1 || self.quantities.iter().any(|q| !(1..=256).contains(q)) {
            return Err("identity token is incomplete, duplicated, or invalid; progression cannot be rebound".into());
        }
        let token = std::array::from_fn(|i| (self.quantities[i] - 1) as u8);
        if token[6] >> 4 != 4 || token[8] >> 6 != 2 {
            return Err("unsupported identity UUID encoding".into());
        }
        Ok(token)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarkerObservation {
    pub anchor: NativeAnchor,
    pub token: InventoryToken,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BindingIntent {
    pub schema: u32,
    pub scope: BindingScope,
    pub uuid: [u8; FRAGMENTS],
    pub slot: u8,
    pub manifest_sha256: String,
    pub source_save_sha256: String,
    pub phase: String,
}
/// Can only be created from a durably written, validated PREPARED intent.
#[derive(Clone, Debug)]
pub struct PreparedBinding {
    intent: BindingIntent,
    anchor: NativeAnchor,
}
impl PreparedBinding {
    pub fn intent(&self) -> &BindingIntent {
        &self.intent
    }
    pub fn anchor(&self) -> &NativeAnchor {
        &self.anchor
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundIdentity {
    pub character_key: String,
    pub scope: BindingScope,
    pub uuid: [u8; FRAGMENTS],
    pub slot: u8,
    pub persisted_save_sha256: String,
}

pub enum IdentityReply {
    Prepared {
        request: u64,
        binding: PreparedBinding,
    },
    Bound {
        request: u64,
        anchor: NativeAnchor,
        binding: BoundIdentity,
    },
    Pending {
        request: u64,
        reason: String,
    },
    Error {
        request: u64,
        reason: String,
    },
}
enum WorkerRequest {
    Prepare {
        request: u64,
        observation: MarkerObservation,
    },
    Verify {
        request: u64,
        observation: MarkerObservation,
    },
}

/// Own this on the game-task state, but all filesystem work runs on its worker.
/// Requests are nonblocking and bounded. Match reply request+anchor to the
/// current world epoch before using it. Dropping the service stops its worker.
pub struct IdentityService {
    tx: SyncSender<WorkerRequest>,
    rx: Receiver<IdentityReply>,
    sequence: AtomicU64,
}
impl IdentityService {
    pub fn start(
        root: PathBuf,
        scope: BindingScope,
        manifest: MarkerManifest,
        save_path: PathBuf,
        recovery_root: PathBuf,
    ) -> Result<Self> {
        scope.validate()?;
        manifest.validate()?;
        let (tx, requests) = mpsc::sync_channel(16);
        let (replies, rx) = mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("ascension-identity-storage".into())
            .spawn(move || {
                let store = BindingJournal::open(root, scope, manifest, save_path, recovery_root);
                while let Ok(request) = requests.recv() {
                    let request_id = match &request {
                        WorkerRequest::Prepare { request, .. }
                        | WorkerRequest::Verify { request, .. } => *request,
                    };
                    let answer = match &store {
                        Err(error) => IdentityReply::Error {
                            request: request_id,
                            reason: error.clone(),
                        },
                        Ok(store) => match request {
                            WorkerRequest::Prepare {
                                request,
                                observation,
                            } => match store.prepare(observation) {
                                Ok(binding) => IdentityReply::Prepared { request, binding },
                                Err(reason) => IdentityReply::Error { request, reason },
                            },
                            WorkerRequest::Verify {
                                request,
                                observation,
                            } => match store.verify(&observation) {
                                Ok(binding) => IdentityReply::Bound {
                                    request,
                                    anchor: observation.anchor,
                                    binding,
                                },
                                // An autosave may still be in progress. Keep the token pending;
                                // the caller may re-request after the next normal save tick.
                                Err(reason) => IdentityReply::Pending { request, reason },
                            },
                        },
                    };
                    if replies.send(answer).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx,
            rx,
            sequence: AtomicU64::new(1),
        })
    }
    pub fn prepare(&self, observation: MarkerObservation) -> Result<u64> {
        let request = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.tx
            .try_send(WorkerRequest::Prepare {
                request,
                observation,
            })
            .map_err(|e| e.to_string())?;
        Ok(request)
    }
    pub fn verify_persisted(&self, observation: MarkerObservation) -> Result<u64> {
        let request = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.tx
            .try_send(WorkerRequest::Verify {
                request,
                observation,
            })
            .map_err(|e| e.to_string())?;
        Ok(request)
    }
    pub fn poll(&self) -> Option<IdentityReply> {
        self.rx.try_recv().ok()
    }
}

struct BindingJournal {
    directory: PathBuf,
    scope: BindingScope,
    manifest: MarkerManifest,
    save_path: PathBuf,
    recovery_root: PathBuf,
    prepared: RefCell<BTreeMap<u8, PreparedBinding>>,
    _lock: File,
}
impl BindingJournal {
    fn open(
        root: PathBuf,
        scope: BindingScope,
        manifest: MarkerManifest,
        save_path: PathBuf,
        recovery_root: PathBuf,
    ) -> Result<Self> {
        let directory = root.join("identity").join(scope.digest());
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|e| format!("another runtime owns this identity scope: {e}"))?;
        Ok(Self {
            directory,
            scope,
            manifest,
            save_path,
            recovery_root,
            prepared: RefCell::new(BTreeMap::new()),
            _lock: lock,
        })
    }
    fn slot_path(&self, slot: u8) -> PathBuf {
        self.directory.join(format!("slot-{slot}.json"))
    }
    fn prepare(&self, observation: MarkerObservation) -> Result<PreparedBinding> {
        if observation.anchor.slot >= 10 {
            return Err("identity binding save slot is out of range".into());
        }
        let proof = read_save_tokens(&self.save_path, &self.manifest, &self.scope)?;
        if !observation.token.is_empty() {
            return self.resume_partial(observation, &proof);
        }
        if !proof.tokens[observation.anchor.slot as usize].is_empty() {
            return Err("saved slot already has marker fragments; binding refused".into());
        }
        if !proof.occupied[observation.anchor.slot as usize] {
            return Err("new character has not reached a normal game save yet".into());
        }
        // Retry of a PREPARED transaction in this exact world generation may
        // reuse its still-unapplied UUID. A later epoch must create a fresh one.
        if let Some(cached) = self.prepared.borrow().get(&observation.anchor.slot) {
            if cached.anchor == observation.anchor {
                return Ok(cached.clone());
            }
        }
        let prior = read_record::<BindingIntent>(&self.slot_path(observation.anchor.slot))?;
        let source_slot_hash = proof.slot_body_sha256[observation.anchor.slot as usize]
            .as_deref()
            .ok_or("occupied save slot is missing its content fingerprint")?;
        let mut resume = false;
        if let Some(prior) = &prior {
            if prior.scope != self.scope || prior.schema != 1 {
                return Err("prior slot binding has incompatible scope/schema".into());
            }
            resume = prior.manifest_sha256 == self.manifest.digest()
                && crate::recovery::resume_prepared(&self.recovery_root, prior, source_slot_hash)?;
            if !resume {
                self.archive(prior)?;
            }
        }
        // Native AND checksummed saved inventories are entirely empty. The
        // sole resumption exception requires an immutable incident receipt and
        // byte-identical slot body. Otherwise rotate UUID without inference.
        let intent = if resume {
            prior
                .as_ref()
                .expect("resumption requires an existing intent")
                .clone()
        } else {
            BindingIntent {
                schema: 1,
                scope: self.scope.clone(),
                uuid: random_uuid()?,
                slot: observation.anchor.slot,
                manifest_sha256: self.manifest.digest(),
                source_save_sha256: proof.save_sha256,
                phase: "PREPARED".into(),
            }
        };
        write_record(&self.slot_path(intent.slot), &intent, prior.is_some())?;
        crate::recovery::bind_prepared(&self.recovery_root, &intent, source_slot_hash)?;
        let binding = PreparedBinding {
            intent,
            anchor: observation.anchor,
        };
        self.prepared
            .borrow_mut()
            .insert(binding.anchor.slot, binding.clone());
        Ok(binding)
    }
    /// Resume only an already durable transaction whose partial inventory has
    /// reached an ordinary checksummed save. Never rotate or rewrite a UUID
    /// after any of its fragments exist; the native binder validates and skips
    /// completed fragments before using its existing grant/quantity protocol.
    fn resume_partial(
        &self,
        observation: MarkerObservation,
        proof: &SaveTokens,
    ) -> Result<PreparedBinding> {
        let slot = observation.anchor.slot as usize;
        if slot >= proof.tokens.len()
            || !proof.occupied[slot]
            || observation.token.commit_quantity != 0
            || observation.token != proof.tokens[slot]
        {
            return Err(
                "partial identity must match the same occupied checksummed save slot before resumption"
                    .into(),
            );
        }
        let prior = read_record::<BindingIntent>(&self.slot_path(observation.anchor.slot))?.ok_or(
            "partial identity has no durable PREPARED intent; automatic overwrite forbidden",
        )?;
        if prior.schema != 1
            || prior.scope != self.scope
            || prior.slot != observation.anchor.slot
            || prior.manifest_sha256 != self.manifest.digest()
            || prior.phase != "PREPARED"
            || !is_digest(&prior.source_save_sha256)
            || prior.uuid[6] >> 4 != 4
            || prior.uuid[8] >> 6 != 2
            || !partial_matches_uuid(&observation.token, &prior.uuid)
        {
            return Err(
                "partial identity conflicts with the durable PREPARED scope/layout/slot/UUID"
                    .into(),
            );
        }
        // A UUID record is only written by successful committed verification.
        // Its presence alongside a partial token cannot be treated as a fresh
        // PREPARED transaction, including a stale slot journal after a crash.
        if read_record::<BindingIntent>(
            &self
                .directory
                .join(format!("uuid-{}.json", hex(&prior.uuid))),
        )?
        .is_some()
        {
            return Err("partial identity UUID already has a committed binding record".into());
        }
        for (other, token) in proof.tokens.iter().enumerate() {
            if other == slot || token.is_empty() {
                continue;
            }
            let collision = match token.commit_quantity {
                0 => {
                    if token.quantities.iter().any(|quantity| *quantity > 256) {
                        return Err(
                            "another saved slot has a corrupt partial identity token".into()
                        );
                    }
                    // Quantity 1 is the normal intermediate grant value. A
                    // sibling partial made only of such values is ambiguous
                    // and therefore also blocks automatic resumption.
                    partial_matches_uuid(token, &prior.uuid)
                }
                1 => token.decode()? == prior.uuid,
                _ => {
                    return Err("another saved slot has an invalid identity commit marker".into());
                }
            };
            if collision {
                return Err(
                    "another saved slot contains the same UUID or a compatible partial identity; resumption is ambiguous"
                        .into(),
                );
            }
        }
        let binding = PreparedBinding {
            intent: prior.clone(),
            anchor: observation.anchor,
        };
        self.prepared
            .borrow_mut()
            .insert(binding.anchor.slot, binding.clone());
        Ok(binding)
    }
    fn archive(&self, intent: &BindingIntent) -> Result<()> {
        let history = self.directory.join("history");
        fs::create_dir_all(&history).map_err(|e| e.to_string())?;
        let content = digest(&serde_json::to_vec(intent).map_err(|e| e.to_string())?);
        let path = history.join(format!(
            "slot-{}-{}-{}.json",
            intent.slot,
            hex(&intent.uuid),
            content
        ));
        match read_record::<BindingIntent>(&path)? {
            Some(existing) if existing == *intent => Ok(()),
            Some(_) => Err("archived identity journal conflicts with its content address".into()),
            None => write_record(&path, intent, false),
        }
    }
    fn verify(&self, observation: &MarkerObservation) -> Result<BoundIdentity> {
        let uuid = observation.token.decode()?;
        let proof = read_save_tokens(&self.save_path, &self.manifest, &self.scope)?;
        let slot = observation.anchor.slot as usize;
        if slot >= 10 || proof.tokens[slot].decode()? != uuid {
            return Err("token is not yet present in a checksummed game save".into());
        }
        for (other, token) in proof.tokens.iter().enumerate() {
            if other != slot && token.decode().ok() == Some(uuid) {
                return Err(
                    "duplicate Ascension UUID in multiple save slots; migration is ambiguous"
                        .into(),
                );
            }
        }
        let token_path = self.directory.join(format!("uuid-{}.json", hex(&uuid)));
        if let Some(prior) = read_record::<BindingIntent>(&token_path)? {
            if prior.scope != self.scope
                || prior.uuid != uuid
                || prior.slot != observation.anchor.slot
            {
                return Err(
                    "UUID was bound to another slot or scope; explicit migration required".into(),
                );
            }
        }
        if let Some(prior) = read_record::<BindingIntent>(&self.slot_path(observation.anchor.slot))?
        {
            if prior.scope != self.scope || prior.schema != 1 {
                return Err("slot binding scope/schema conflicts with the saved UUID".into());
            }
            if prior.uuid != uuid {
                self.archive(&prior)?;
            } else if prior.manifest_sha256 != self.manifest.digest() {
                return Err("saved UUID codec/layout requires an explicit migration".into());
            }
        }
        let active = BindingIntent {
            schema: 1,
            scope: self.scope.clone(),
            uuid,
            slot: observation.anchor.slot,
            manifest_sha256: self.manifest.digest(),
            source_save_sha256: proof.save_sha256.clone(),
            phase: "ACTIVE".into(),
        };
        // UUID record first. A crash before slot-record replacement is recoverable
        // by reading the committed token, never by guessing a missing token.
        write_record(&token_path, &active, true)?;
        write_record(&self.slot_path(active.slot), &active, true)?;
        self.prepared.borrow_mut().remove(&active.slot);
        Ok(BoundIdentity {
            character_key: self.scope.character_key(&uuid),
            scope: self.scope.clone(),
            uuid,
            slot: active.slot,
            persisted_save_sha256: proof.save_sha256,
        })
    }
}

fn partial_matches_uuid(token: &InventoryToken, uuid: &[u8; FRAGMENTS]) -> bool {
    token.commit_quantity == 0
        && token.quantities.iter().any(|quantity| *quantity != 0)
        && token.quantities.iter().zip(uuid).all(|(quantity, byte)| {
            *quantity == 0 || *quantity == 1 || *quantity == u32::from(*byte) + 1
        })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: u32,
    payload_json: String,
    sha256: String,
}
fn read_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if bytes.len() > 64 * 1024 {
        return Err("identity journal record exceeds limit".into());
    }
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if envelope.schema != 1 || digest(envelope.payload_json.as_bytes()) != envelope.sha256 {
        return Err("identity journal integrity failure; automatic reset forbidden".into());
    }
    serde_json::from_str(&envelope.payload_json)
        .map(Some)
        .map_err(|e| e.to_string())
}
fn write_record(path: &Path, payload: &impl Serialize, replace: bool) -> Result<()> {
    let payload_json = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let envelope = Envelope {
        schema: 1,
        sha256: digest(payload_json.as_bytes()),
        payload_json,
    };
    let bytes = serde_json::to_vec_pretty(&envelope).map_err(|e| e.to_string())?;
    if !replace {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let temporary = path.with_extension(format!("{}.tmp", hex(&random_uuid()?)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    use std::os::windows::ffi::OsStrExt;
    let source: Vec<u16> = temporary.as_os_str().encode_wide().chain([0]).collect();
    let destination: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } == 0 {
        return Err(format!(
            "atomic identity journal replacement failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
}
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut std::ffi::c_void,
        buffer: *mut u8,
        length: u32,
        flags: u32,
    ) -> i32;
}
fn random_uuid() -> Result<[u8; FRAGMENTS]> {
    let mut value = [0; FRAGMENTS];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            value.as_mut_ptr(),
            FRAGMENTS as u32,
            2,
        )
    };
    if status < 0 {
        return Err(format!("OS random generator failed: {status:#x}"));
    }
    value[6] = (value[6] & 0x0f) | 0x40;
    value[8] = (value[8] & 0x3f) | 0x80;
    Ok(value)
}
fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn is_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

struct SaveTokens {
    save_sha256: String,
    tokens: [InventoryToken; 10],
    occupied: [bool; 10],
    slot_body_sha256: [Option<String>; 10],
}
/// Read-only proof parser. Inventory mutations and save checksums are always
/// written by the game. This reader accepts only the audited PC save layouts.
fn read_save_tokens(
    path: &Path,
    manifest: &MarkerManifest,
    scope: &BindingScope,
) -> Result<SaveTokens> {
    const HEADER: usize = 0x300;
    const SLOT: usize = 0x280010;
    const BODY: usize = 0x280000;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < HEADER + 10 * SLOT + 0x60010
        || bytes.len() > 64 * 1024 * 1024
        || bytes.get(..4) != Some(b"BND4")
    {
        return Err("isolated save is not a supported PC BND4 container".into());
    }
    let metadata = HEADER + 10 * SLOT;
    let metadata_body = &bytes[metadata + 16..metadata + 16 + 0x60000];
    if md5::compute(metadata_body).as_ref() != &bytes[metadata..metadata + 16] {
        return Err("save metadata checksum not ready".into());
    }
    let account = u64::from_le_bytes(
        metadata_body[4..12]
            .try_into()
            .map_err(|_| "missing save account")?,
    );
    if account.to_string() != scope.steam_account {
        return Err("save Steam account does not match the binding scope".into());
    }
    let mut tokens = std::array::from_fn(|_| InventoryToken::default());
    let mut occupied = [false; 10];
    let mut slot_body_sha256 = std::array::from_fn(|_| None);
    for slot in 0..10 {
        let offset = HEADER + slot * SLOT;
        let body = &bytes[offset + 16..offset + 16 + BODY];
        let version = u32::from_le_bytes(body[..4].try_into().expect("bounded slot"));
        if version == 0 {
            continue;
        }
        if ![261, 262].contains(&version) {
            return Err(format!("unsupported save layout version {version}"));
        }
        if md5::compute(body).as_ref() != &bytes[offset..offset + 16] {
            return Err("save slot checksum not ready".into());
        }
        occupied[slot] = true;
        slot_body_sha256[slot] = Some(digest(body));
        let mut cursor = Cursor {
            bytes: body,
            at: 0x20,
        };
        for _ in 0..0x1400 {
            let handle = cursor.u32()?;
            cursor.skip(4)?;
            if handle != 0 && handle >> 28 != 0xc {
                cursor.skip(8)?;
            }
            if handle != 0 && handle >> 28 == 0x8 {
                cursor.skip(5)?;
            }
        }
        cursor.skip(0x1b0 + 13 * 16 + 0x58 + 0x1c + 0x58 + 0x58)?;
        for capacity in [0xa80usize, 0x180usize] {
            let length = cursor.u32()? as usize;
            if length > capacity {
                return Err("saved inventory length exceeds its fixed capacity".into());
            }
            for index in 0..capacity {
                let handle = cursor.u32()?;
                let quantity = cursor.u32()?;
                cursor.skip(4)?;
                if index >= length
                    || quantity == 0
                    || handle >> 28 != 0xb
                    || handle & 0x0080_0000 != 0
                {
                    continue;
                }
                let goods = handle & 0x00ff_ffff;
                if let Some(index) = manifest.payload_goods.iter().position(|id| *id == goods) {
                    if tokens[slot].quantities[index] != 0 {
                        return Err("duplicate marker fragment in saved inventory".into());
                    }
                    tokens[slot].quantities[index] = quantity;
                } else if goods == manifest.commit_good {
                    if tokens[slot].commit_quantity != 0 {
                        return Err("duplicate commit marker in saved inventory".into());
                    }
                    tokens[slot].commit_quantity = quantity;
                }
            }
        }
    }
    Ok(SaveTokens {
        save_sha256: digest(&bytes),
        tokens,
        occupied,
        slot_body_sha256,
    })
}
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Cursor<'_> {
    fn skip(&mut self, count: usize) -> Result<()> {
        self.at = self
            .at
            .checked_add(count)
            .filter(|p| *p <= self.bytes.len())
            .ok_or("truncated save prefix")?;
        Ok(())
    }
    fn u32(&mut self) -> Result<u32> {
        let start = self.at;
        self.skip(4)?;
        Ok(u32::from_le_bytes(
            self.bytes[start..self.at].try_into().expect("bounded read"),
        ))
    }
}

/// Capture the token on a native task after root's version/offline gates.
/// # Safety
/// Main game thread only; `epoch` must change on unload or character switch.
pub unsafe fn observe(manifest: &MarkerManifest, epoch: u64) -> Result<MarkerObservation> {
    manifest.validate()?;
    let game = unsafe { GameMan::instance() }.map_err(|e| e.to_string())?;
    if !(0..10).contains(&game.save_slot) || game.warp_requested || game.new_game_plus_requested {
        return Err("identity observation requires a loaded local offline slot".into());
    }
    let world = unsafe { WorldChrMan::instance() }.map_err(|e| e.to_string())?;
    let player = world
        .main_player
        .as_ref()
        .ok_or("local player unavailable")?;
    let data = unsafe { player.player_game_data.as_ref() };
    if !unsafe { crate::game::native_offline_host(game, data) } {
        return Err("identity observation is not the main local character".into());
    }
    let inventory = &data.equipment.equip_inventory_data.items_data;
    if inventory.normal_items_len > inventory.normal_items_capacity
        || inventory.key_items_len > inventory.key_items_capacity
        || inventory.normal_items_capacity > 10000
        || inventory.key_items_capacity > 10000
    {
        return Err("native inventory bounds invalid".into());
    }
    let mut token = InventoryToken::default();
    for item in inventory.items() {
        if item.item_id.category() != ItemCategory::Goods {
            continue;
        }
        let goods = item.item_id.param_id();
        let index = manifest.payload_goods.iter().position(|id| *id == goods);
        if index.is_none() && goods != manifest.commit_good {
            continue;
        }
        // Goods carry direct selectors. Indexed weapons/gems must never be
        // reinterpreted as our payload, even if a malformed mapping shares ID.
        if item.gaitem_handle.is_indexed()
            || item.gaitem_handle.selector() != goods
            || item.gaitem_handle.category_raw() != 3
        {
            return Err("marker ItemId/GaItem handle disagreement".into());
        }
        if let Some(index) = index {
            if token.quantities[index] != 0 {
                return Err("duplicate marker fragment in native inventory".into());
            }
            token.quantities[index] = item.quantity;
        } else {
            if token.commit_quantity != 0 {
                return Err("duplicate native commit marker".into());
            }
            token.commit_quantity = item.quantity;
        }
    }
    Ok(MarkerObservation {
        anchor: NativeAnchor {
            epoch,
            slot: game.save_slot as u8,
            player_data_address: player.player_game_data.as_ptr() as u64,
        },
        token,
    })
}

/// Validate the live rows before any identity mutation. Package generation is
/// responsible for collision checks; these checks catch incompatible runtime
/// overrides. Invoke once per load/param reload on the game task.
/// # Safety
/// Main game thread after native initialization and version gate only.
pub unsafe fn validate_native_rows(manifest: &MarkerManifest) -> Result<()> {
    manifest.validate()?;
    let params = unsafe { SoloParamRepository::instance() }.map_err(|e| e.to_string())?;
    for (index, id) in manifest
        .payload_goods
        .iter()
        .chain([&manifest.commit_good])
        .enumerate()
    {
        let row = params
            .get::<EquipParamGoods>(*id)
            .ok_or_else(|| format!("missing identity Goods row {id}"))?;
        let required = if index < FRAGMENTS { 256 } else { 1 };
        if row.max_num() < required
            || (index < FRAGMENTS && row.is_only_one())
            || row.is_remove_item_for_game_clear()
            || row.is_discard()
            || row.is_drop()
            || row.is_deposit()
            || row.is_equip()
            || row.is_consume()
            || row.is_auto_equip()
        {
            return Err(format!(
                "identity Goods row {id} has incompatible persistence/interaction settings"
            ));
        }
    }
    for (lot, good) in manifest
        .init_item_lots
        .iter()
        .zip(manifest.payload_goods.iter())
        .chain([(&manifest.commit_item_lot, &manifest.commit_good)])
    {
        let row = params
            .get::<ItemLotParam_map>(*lot)
            .ok_or_else(|| format!("missing identity ItemLot row {lot}"))?;
        let alternative_weights = [
            row.lot_item_base_point02(),
            row.lot_item_base_point03(),
            row.lot_item_base_point04(),
            row.lot_item_base_point05(),
            row.lot_item_base_point06(),
            row.lot_item_base_point07(),
            row.lot_item_base_point08(),
        ];
        if row.lot_item_id01() != *good as i32
            || row.lot_item_category01() != 1
            || row.lot_item_num01() != 1
            || row.lot_item_base_point01() != 1000
            || alternative_weights.iter().any(|n| *n != 0)
            || row.get_item_flag_id() != 0
            || row.get_item_flag_id01() != 0
            // The native Golden Seed template and official ItemLot definition
            // both use GameClearOffset=-1. Requiring 0 rejected our
            // generated deterministic rows before the first marker grant.
            || row.game_clear_offset() != -1
        {
            return Err(format!(
                "identity ItemLot row {lot} is not a deterministic unflagged grant: \
                 item_id={} (expected {good}), category={} (expected 1), \
                 quantity={} (expected 1), weight={} (expected 1000), \
                 alternative_weights={alternative_weights:?} (expected all 0), \
                 shared_flag={} (expected 0), item_flag={} (expected 0), \
                 GameClearOffset={} (expected -1)",
                row.lot_item_id01(),
                row.lot_item_category01(),
                row.lot_item_num01(),
                row.lot_item_base_point01(),
                row.get_item_flag_id(),
                row.get_item_flag_id01(),
                row.game_clear_offset(),
            ));
        }
    }
    Ok(())
}

pub enum NativeBindingStep {
    Progress(&'static str),
    AwaitingPersistence(MarkerObservation),
}
enum PendingOperation {
    Grant(usize),
    Quantity(usize, u32),
    Commit,
}
/// A single PREPARED transaction. No native pointer or TalkScript is retained.
pub struct NativeBinder {
    manifest: MarkerManifest,
    prepared: PreparedBinding,
    pending: Option<PendingOperation>,
    save_requested: bool,
    failed: bool,
}
impl NativeBinder {
    pub fn new(manifest: MarkerManifest, prepared: PreparedBinding) -> Result<Self> {
        manifest.validate()?;
        if prepared.intent.schema != 1 || prepared.intent.manifest_sha256 != manifest.digest() {
            return Err("prepared identity manifest changed".into());
        }
        Ok(Self {
            manifest,
            prepared,
            pending: None,
            save_requested: false,
            failed: false,
        })
    }
    /// Root passes its verified safe-state gate; no filesystem operation occurs.
    /// # Safety
    /// Main game task only. Increment epoch/reset binder on world unload, and
    /// do not call while another native mutation holds game references.
    pub unsafe fn step(&mut self, epoch: u64, allow_write: bool) -> Result<NativeBindingStep> {
        if self.failed {
            return Err("identity transaction stopped after a previous error".into());
        }
        let result = unsafe { self.step_inner(epoch, allow_write) };
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    unsafe fn step_inner(&mut self, epoch: u64, allow_write: bool) -> Result<NativeBindingStep> {
        let observation = unsafe { observe(&self.manifest, epoch) }?;
        if observation.anchor != self.prepared.anchor {
            return Err("character/world changed during identity transaction".into());
        }
        if !allow_write {
            return Ok(NativeBindingStep::Progress(
                "waiting for safe identity write frame",
            ));
        }
        if let Some(operation) = self.pending.take() {
            let valid = match operation {
                PendingOperation::Grant(i) => observation.token.quantities[i] == 1,
                PendingOperation::Quantity(i, n) => observation.token.quantities[i] == n,
                PendingOperation::Commit => observation.token.commit_quantity == 1,
            };
            if !valid {
                return Err("engine identity operation did not produce its exact expected result; no retry or duplicate grant".into());
            }
        }
        if observation.token.commit_quantity != 0 {
            if observation.token.decode()? != self.prepared.intent.uuid {
                return Err("committed marker differs from prepared UUID".into());
            }
            if !self.save_requested {
                unsafe { invoke_event(50, &[0]) }?;
                self.save_requested = true;
                return Ok(NativeBindingStep::Progress(
                    "normal game save requested for committed UUID",
                ));
            }
            return Ok(NativeBindingStep::AwaitingPersistence(observation));
        }
        unsafe { validate_native_rows(&self.manifest) }?;
        for i in 0..FRAGMENTS {
            let actual = observation.token.quantities[i];
            let expected = self.prepared.intent.uuid[i] as u32 + 1;
            if actual == 0 {
                unsafe {
                    ensure_capacity(
                        observation
                            .token
                            .quantities
                            .iter()
                            .filter(|q| **q == 0)
                            .count()
                            + 1,
                    )
                }?;
                unsafe { invoke_event(104, &[self.manifest.init_item_lots[i] as i32]) }?;
                self.pending = Some(PendingOperation::Grant(i));
                return Ok(NativeBindingStep::Progress(
                    "writing UUID fragment via engine ItemLot",
                ));
            }
            if actual != 1 && actual != expected {
                return Err(
                    "unexpected partial UUID payload; automatic overwrite forbidden".into(),
                );
            }
            if actual != expected {
                unsafe {
                    invoke_event(
                        52,
                        &[
                            3,
                            self.manifest.payload_goods[i] as i32,
                            (expected - actual) as i32,
                        ],
                    )
                }?;
                self.pending = Some(PendingOperation::Quantity(i, expected));
                return Ok(NativeBindingStep::Progress(
                    "setting UUID fragment quantity through engine",
                ));
            }
        }
        unsafe { ensure_capacity(1) }?;
        unsafe { invoke_event(104, &[self.manifest.commit_item_lot as i32]) }?;
        self.pending = Some(PendingOperation::Commit);
        Ok(NativeBindingStep::Progress(
            "committing completed UUID marker",
        ))
    }
}
unsafe fn ensure_capacity(required: usize) -> Result<()> {
    let world = unsafe { WorldChrMan::instance() }.map_err(|e| e.to_string())?;
    let player = world
        .main_player
        .as_ref()
        .ok_or("local player unavailable")?;
    let data = unsafe { player.player_game_data.as_ref() };
    // Conservatively require spare capacity in both partitions. Never remove
    // somebody else's item to make room for the mod's identity marker.
    let inventory = &data.equipment.equip_inventory_data.items_data;
    if (inventory
        .normal_items_capacity
        .saturating_sub(inventory.normal_items_len) as usize)
        < required
        || (inventory
            .key_items_capacity
            .saturating_sub(inventory.key_items_len) as usize)
            < required
    {
        return Err(
            "inventory has insufficient reserved space for the complete Ascension identity marker"
                .into(),
        );
    }
    Ok(())
}
unsafe fn invoke_event(command: i32, arguments: &[i32]) -> Result<()> {
    // Copy only the handle, then drop all game references before the ESD event
    // mutates player data. Script lifetime is limited to this one invocation.
    let handle = {
        let world = unsafe { WorldChrMan::instance() }.map_err(|e| e.to_string())?;
        world
            .main_player
            .as_ref()
            .ok_or("local player unavailable")?
            .chr_ins
            .field_ins_handle
    };
    let mut talk = TalkScript::new(BlockId::none(), 1000, handle);
    talk.event((command, arguments.iter().copied().map(EzStateValue::Int32)))
        .map_err(|e| e.to_string())
}
