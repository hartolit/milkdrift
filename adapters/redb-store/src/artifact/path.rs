use super::publication::publication_in_transaction;
use super::*;
impl RedbStore {
    pub(crate) fn lock_artifact_publications(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, ()>, PersistenceError> {
        self.artifact_serialization
            .lock()
            .map_err(|_| PersistenceError::Storage {
                class: StorageFailureClass::Internal,
                message: "artifact publication serialization lock was poisoned".to_owned(),
            })
    }

    pub(crate) fn temp_path(&self, publication: &ArtifactPublicationId) -> PathBuf {
        self.temp_root.join(publication_temp_name(publication))
    }

    pub(crate) fn content_path(&self, digest: ContentDigest) -> PathBuf {
        let hex = digest.to_hex();
        self.artifact_root.join(&hex[..2]).join(&hex[2..])
    }
}

pub(crate) fn publication_temp_name(publication: &ArtifactPublicationId) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.artifact-publication-temp.v1\0");
    hasher.update(publication.as_str().as_bytes());
    format!("{}.part", hasher.finalize())
}

pub(crate) fn publication_age_key(
    created_at_millis: u64,
    publication: &ArtifactPublicationId,
) -> Result<Vec<u8>, PersistenceError> {
    let publication = codec::component(publication.as_str())?;
    let mut key = Vec::with_capacity(std::mem::size_of::<u64>() + publication.len());
    key.extend_from_slice(&created_at_millis.to_be_bytes());
    key.extend_from_slice(&publication);
    Ok(key)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactPathKind {
    TempPending,
    TempReady,
    ContentIntent,
}

impl ArtifactPathKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::TempPending => "temp_pending",
            Self::TempReady => "temp_ready",
            Self::ContentIntent => "content_intent",
        }
    }

    pub(crate) const fn ordered_tag(self) -> u8 {
        match self {
            Self::TempPending | Self::TempReady => 0,
            Self::ContentIntent => 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactPathEntry {
    pub(crate) kind: ArtifactPathKind,
    pub(crate) created_at_millis: u64,
    pub(crate) publication: ArtifactPublicationId,
    pub(crate) identity: String,
    pub(crate) logical_key: Vec<u8>,
    pub(crate) storage_key: Vec<u8>,
}

pub(crate) fn artifact_path_entry(
    record: &PublicationRecord,
    kind: ArtifactPathKind,
) -> Result<ArtifactPathEntry, PersistenceError> {
    let identity = match kind {
        ArtifactPathKind::TempPending | ArtifactPathKind::TempReady => {
            publication_temp_name(&record.publication)
        }
        ArtifactPathKind::ContentIntent => record.metadata.reference().digest().to_hex(),
    };
    let created_at = format!("{:020}", record.created_at_millis);
    let logical_key = codec::components(&[
        kind.label(),
        &created_at,
        &identity,
        record.publication.as_str(),
    ])?;
    let mut storage_key = Vec::with_capacity(9 + logical_key.len());
    storage_key.push(kind.ordered_tag());
    storage_key.extend_from_slice(&record.created_at_millis.to_be_bytes());
    storage_key.extend_from_slice(&logical_key);
    Ok(ArtifactPathEntry {
        kind,
        created_at_millis: record.created_at_millis,
        publication: record.publication.clone(),
        identity,
        storage_key,
        logical_key,
    })
}

pub(crate) fn decode_artifact_path_entry(
    storage_key: &[u8],
    logical_key: &[u8],
) -> Result<ArtifactPathEntry, PersistenceError> {
    let components = codec::decode_components(logical_key, 4)?;
    let kind = match components[0] {
        "temp_pending" => ArtifactPathKind::TempPending,
        "temp_ready" => ArtifactPathKind::TempReady,
        "content_intent" => ArtifactPathKind::ContentIntent,
        _ => {
            return Err(error::corruption(
                "artifact path inventory contains an unknown kind",
            ));
        }
    };
    if components[1].len() != 20 || !components[1].bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error::corruption(
            "artifact path inventory contains an invalid timestamp",
        ));
    }
    let created_at_millis = components[1]
        .parse::<u64>()
        .map_err(|cause| error::corruption(format!("invalid artifact path timestamp: {cause}")))?;
    let publication = ArtifactPublicationId::new(components[3]).map_err(|cause| {
        error::corruption(format!(
            "invalid artifact path publication identity: {cause}"
        ))
    })?;
    let mut expected_key = Vec::with_capacity(9 + logical_key.len());
    expected_key.push(kind.ordered_tag());
    expected_key.extend_from_slice(&created_at_millis.to_be_bytes());
    expected_key.extend_from_slice(logical_key);
    if storage_key != expected_key {
        return Err(error::corruption(
            "artifact path inventory key disagrees with its document",
        ));
    }
    Ok(ArtifactPathEntry {
        kind,
        created_at_millis,
        publication,
        identity: components[2].to_owned(),
        logical_key: logical_key.to_vec(),
        storage_key: expected_key,
    })
}

pub(crate) fn put_artifact_path(
    write: &redb::WriteTransaction,
    entry: &ArtifactPathEntry,
) -> Result<(), PersistenceError> {
    let mut paths = write.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    if paths
        .insert(entry.storage_key.as_slice(), entry.logical_key.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "artifact path inventory changed outside its authoritative transaction",
        ));
    }
    Ok(())
}

pub(crate) fn remove_artifact_path(
    write: &redb::WriteTransaction,
    entry: &ArtifactPathEntry,
) -> Result<(), PersistenceError> {
    let mut paths = write.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    let removed = paths
        .remove(entry.storage_key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_vec());
    if removed.as_deref() != Some(entry.logical_key.as_slice()) {
        return Err(error::corruption(
            "artifact path inventory is absent during finalization",
        ));
    }
    Ok(())
}

pub(crate) fn artifact_path_exists(
    write: &redb::WriteTransaction,
    entry: &ArtifactPathEntry,
) -> Result<bool, PersistenceError> {
    let paths = write.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    match paths
        .get(entry.storage_key.as_slice())
        .map_err(error::redb)?
    {
        None => Ok(false),
        Some(value) if value.value() == entry.logical_key.as_slice() => Ok(true),
        Some(_) => Err(error::corruption(
            "artifact path inventory document is invalid",
        )),
    }
}

pub(crate) fn artifact_delete_guard_key(
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<Vec<u8>, PersistenceError> {
    let label = match kind {
        ArtifactPathKind::TempPending | ArtifactPathKind::TempReady => "temp",
        ArtifactPathKind::ContentIntent => "content",
    };
    codec::components(&[label, identity])
}

pub(crate) fn artifact_delete_guard_exists(
    write: &redb::WriteTransaction,
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<bool, PersistenceError> {
    let key = artifact_delete_guard_key(kind, identity)?;
    Ok(write
        .open_table(ARTIFACT_DELETE_GUARDS)
        .map_err(error::redb)?
        .get(key.as_slice())
        .map_err(error::redb)?
        .is_some())
}

pub(crate) fn put_artifact_delete_guard(
    write: &redb::WriteTransaction,
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<(), PersistenceError> {
    let key = artifact_delete_guard_key(kind, identity)?;
    if write
        .open_table(ARTIFACT_DELETE_GUARDS)
        .map_err(error::redb)?
        .insert(key.as_slice(), 1)
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption("artifact delete guard was created twice"));
    }
    Ok(())
}

pub(crate) fn remove_artifact_delete_guard(
    write: &redb::WriteTransaction,
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<(), PersistenceError> {
    let key = artifact_delete_guard_key(kind, identity)?;
    let mut guards = write
        .open_table(ARTIFACT_DELETE_GUARDS)
        .map_err(error::redb)?;
    let removed = guards.remove(key.as_slice()).map_err(error::redb)?;
    if removed.is_none() {
        return Err(error::corruption(
            "artifact delete guard is absent at finalization",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TempInventoryState {
    Pending,
    Ready,
}

pub(crate) fn temp_inventory_state(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<TempInventoryState, PersistenceError> {
    let pending = artifact_path_entry(record, ArtifactPathKind::TempPending)?;
    let ready = artifact_path_entry(record, ArtifactPathKind::TempReady)?;
    match (
        artifact_path_exists(write, &pending)?,
        artifact_path_exists(write, &ready)?,
    ) {
        (true, false) => Ok(TempInventoryState::Pending),
        (false, true) => Ok(TempInventoryState::Ready),
        (false, false) => Err(error::corruption(
            "artifact publication is missing its temporary-path inventory",
        )),
        (true, true) => Err(error::corruption(
            "artifact publication has conflicting temporary-path inventory states",
        )),
    }
}

pub(crate) fn ensure_temp_inventory_ready(
    store: &RedbStore,
    publication: &ArtifactPublicationId,
) -> Result<(), PersistenceError> {
    let write = store.database().begin_write().map_err(error::redb)?;
    let record = publication_in_transaction(&write, publication)?;
    if !matches!(record.state, PublicationState::Writable) {
        return Err(error::corruption(
            "only a writable publication may materialize a temporary path",
        ));
    }
    match temp_inventory_state(&write, &record)? {
        TempInventoryState::Ready => return Ok(()),
        TempInventoryState::Pending => {}
    }
    let temp_name = publication_temp_name(publication);
    if artifact_delete_guard_exists(&write, ArtifactPathKind::TempReady, &temp_name)? {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            message: "temporary artifact path is being durably finalized".to_owned(),
        });
    }
    let path = store.temp_path(publication);
    store.faults.check(FaultPoint::BeforeArtifactTempCreate)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if metadata.len() != 0 {
                return Err(error::corruption(
                    "pending temporary artifact path contains nonempty bytes",
                ));
            }
            open_regular_for_read(&path)?
                .sync_all()
                .map_err(error::io)?;
            sync_directory(&store.temp_root)?;
        }
        Ok(_) => {
            return Err(error::corruption(
                "pending temporary artifact path is not a regular file",
            ));
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            prepare_new_temp(&path, &store.temp_root)?;
        }
        Err(cause) => return Err(error::io(cause)),
    }
    store.faults.check(FaultPoint::AfterArtifactTempCreate)?;
    let pending = artifact_path_entry(&record, ArtifactPathKind::TempPending)?;
    let ready = artifact_path_entry(&record, ArtifactPathKind::TempReady)?;
    remove_artifact_path(&write, &pending)?;
    put_artifact_path(&write, &ready)?;
    store
        .faults
        .check(FaultPoint::BeforeArtifactTempReadyCommit)?;
    write.commit().map_err(error::redb)?;
    store.faults.check(FaultPoint::AfterArtifactTempReadyCommit)
}

pub(crate) fn require_temp_inventory_ready(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    if temp_inventory_state(write, record)? != TempInventoryState::Ready {
        return Err(error::corruption(
            "writable artifact publication has not materialized its temporary path",
        ));
    }
    let temp_name = publication_temp_name(&record.publication);
    if artifact_delete_guard_exists(write, ArtifactPathKind::TempReady, &temp_name)? {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            message: "temporary artifact path is being durably finalized".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn content_intent_state(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<bool, PersistenceError> {
    artifact_path_exists(
        write,
        &artifact_path_entry(record, ArtifactPathKind::ContentIntent)?,
    )
}

pub(crate) fn require_content_intent(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    if !content_intent_state(write, record)? {
        return Err(error::corruption(
            "artifact publication is missing final-content-path intent",
        ));
    }
    Ok(())
}

pub(crate) fn prepare_new_temp(path: &Path, directory: &Path) -> Result<(), PersistenceError> {
    crate::store::prepare_owned_directory(directory, "artifact temporary directory")?;
    if path.exists() {
        let metadata = fs::symlink_metadata(path).map_err(error::io)?;
        if !metadata.file_type().is_file() {
            return Err(error::corruption(
                "artifact temporary path is not a regular file",
            ));
        }
        fs::remove_file(path).map_err(error::io)?;
    }
    let file = create_private_file(path)?;
    file.sync_all().map_err(error::io)?;
    sync_directory(directory)
}

pub(crate) fn create_private_file(path: &Path) -> Result<File, PersistenceError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(error::io)
}

pub(crate) fn open_regular_for_append(path: &Path) -> Result<File, PersistenceError> {
    ensure_regular(path)?;
    open_regular_no_follow(path, true)
}

pub(crate) fn open_regular_for_read(path: &Path) -> Result<File, PersistenceError> {
    ensure_regular(path)?;
    open_regular_no_follow(path, false)
}

#[cfg(unix)]
pub(crate) fn open_regular_no_follow(
    path: &Path,
    writable: bool,
) -> Result<File, PersistenceError> {
    use rustix::fs::{Mode, OFlags};

    let access = if writable {
        OFlags::RDWR
    } else {
        OFlags::RDONLY
    };
    let file = rustix::fs::open(
        path,
        access | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|cause| {
        if cause == rustix::io::Errno::LOOP {
            error::corruption("artifact path changed into a symlink while opening")
        } else {
            error::io(cause.into())
        }
    })?;
    verify_opened_regular_identity(path, &file)?;
    Ok(file)
}

#[cfg(not(unix))]
pub(crate) fn open_regular_no_follow(
    path: &Path,
    writable: bool,
) -> Result<File, PersistenceError> {
    let file = OpenOptions::new()
        .read(true)
        .write(writable)
        .open(path)
        .map_err(error::io)?;
    verify_opened_regular_identity(path, &file)?;
    Ok(file)
}

pub(crate) fn verify_opened_regular_identity(
    path: &Path,
    file: &File,
) -> Result<(), PersistenceError> {
    let opened = file.metadata().map_err(error::io)?;
    let path_metadata = fs::symlink_metadata(path).map_err(error::io)?;
    if !opened.is_file()
        || !path_metadata.file_type().is_file()
        || path_metadata.file_type().is_symlink()
    {
        return Err(error::corruption(
            "artifact path changed type or became a symlink while opening",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if opened.dev() != path_metadata.dev() || opened.ino() != path_metadata.ino() {
            return Err(error::corruption(
                "artifact path identity changed while opening",
            ));
        }
    }
    Ok(())
}

pub(crate) fn ensure_regular(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path).map_err(|cause| {
        if cause.kind() == std::io::ErrorKind::NotFound {
            error::corruption("artifact content file is missing")
        } else {
            error::io(cause)
        }
    })?;
    if !metadata.file_type().is_file() {
        return Err(error::corruption(
            "artifact content path is not a regular file",
        ));
    }
    Ok(())
}

pub(crate) fn publication_length_or_published(
    temp: &Path,
    published: &Path,
    reference: &ArtifactReference,
    maximum: u64,
) -> Result<u64, PersistenceError> {
    if temp.exists() {
        ensure_regular(temp)?;
        let size = fs::metadata(temp).map_err(error::io)?.len();
        if size > reference.size_bytes() || size > maximum {
            return Err(error::corruption(
                "artifact temporary stream exceeds its declared bound",
            ));
        }
        return Ok(size);
    }
    if published.exists() {
        verify_blob(published, reference, maximum)?;
        return Ok(reference.size_bytes());
    }
    Err(error::corruption(
        "writable artifact publication has neither temporary nor published content",
    ))
}

pub(crate) fn verify_blob(
    path: &Path,
    reference: &ArtifactReference,
    maximum: u64,
) -> Result<(), PersistenceError> {
    let mut file = open_regular_for_read(path)?;
    verify_opened_blob(&mut file, reference, maximum)
}

pub(crate) fn verify_opened_blob(
    file: &mut File,
    reference: &ArtifactReference,
    maximum: u64,
) -> Result<(), PersistenceError> {
    if reference.size_bytes() > maximum {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            message: format!("artifact verification exceeds configured bound {maximum}"),
        });
    }
    let size = file.metadata().map_err(error::io)?.len();
    if size != reference.size_bytes() {
        return Err(error::corruption(format!(
            "artifact size mismatch: expected {}, stored {size}",
            reference.size_bytes()
        )));
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; MAX_CHUNK_BYTES];
    let mut read_total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(error::io)?;
        if count == 0 {
            break;
        }
        read_total = read_total
            .checked_add(count as u64)
            .ok_or_else(|| error::corruption("artifact read length overflow"))?;
        if read_total > maximum || read_total > reference.size_bytes() {
            return Err(error::corruption(
                "artifact changed size during verification",
            ));
        }
        hasher.update(&buffer[..count]);
    }
    if read_total != reference.size_bytes()
        || hasher.finalize().as_bytes() != reference.digest().as_bytes()
    {
        return Err(error::corruption("artifact content digest mismatch"));
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), PersistenceError> {
    crate::store::sync_owned_directory(path)
}
