use crate::{RedbStore, error, json, schema::NATIVE_PATH_CLAIMS};
use milkdrift_persistence::{NativePathClaim, NativePathClaimStore, PersistenceError};
use redb::{ReadableTable, ReadableTableMetadata};

const MAX_CLAIMS: u64 = 4096;

impl NativePathClaimStore for RedbStore {
    fn claim_native_paths(&self, claim: &NativePathClaim) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        claim_in_transaction(&write, claim)?;
        write.commit().map_err(error::redb)
    }

    fn claim_serving_native_paths(
        &self,
        record: &milkdrift_persistence::PeerExecutionRecord,
        claim: &NativePathClaim,
    ) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        crate::peer::guard_native_path_claim(&write, record, &claim.owner)?;
        claim_in_transaction(&write, claim)?;
        write.commit().map_err(error::redb)
    }

    fn native_path_claim(&self, owner: &str) -> Result<Option<NativePathClaim>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(NATIVE_PATH_CLAIMS).map_err(error::redb)?;
        table
            .get(owner)
            .map_err(error::redb)?
            .map(|bytes| {
                let claim = json::decode(bytes.value(), "native path claim")?;
                validate_claim(owner, &claim)?;
                Ok(claim)
            })
            .transpose()
    }

    fn release_native_paths(&self, owner: &str) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        release_in_transaction(&write, owner)?;
        write.commit().map_err(error::redb)
    }
}

fn claim_in_transaction(
    write: &redb::WriteTransaction,
    claim: &NativePathClaim,
) -> Result<(), PersistenceError> {
    validate_claim(&claim.owner, claim)?;
    let bytes = json::encode(claim, "native path claim")?;
    let mut table = write.open_table(NATIVE_PATH_CLAIMS).map_err(error::redb)?;
    if let Some(value) = table.get(claim.owner.as_str()).map_err(error::redb)? {
        let existing: NativePathClaim = json::decode(value.value(), "native path claim")?;
        validate_claim(&claim.owner, &existing)?;
        return if existing == *claim {
            Ok(())
        } else {
            Err(PersistenceError::ImmutableConflict {
                entity: "native path claim",
                identity: claim.owner.clone(),
            })
        };
    }
    if table.len().map_err(error::redb)? >= MAX_CLAIMS {
        return Err(PersistenceError::Bounds {
            location: "native_path_claims",
            reason: "native claim capacity is full; unresolved owners remain protected".to_owned(),
        });
    }
    for row in table.iter().map_err(error::redb)? {
        let (key, value) = row.map_err(error::redb)?;
        let existing: NativePathClaim = json::decode(value.value(), "native path claim")?;
        validate_claim(key.value(), &existing)?;
        if claim.roots.iter().any(|root| {
            existing.roots.iter().any(|other| {
                (root.write || other.write)
                    && (root.identity == other.identity
                        || root.physical.starts_with(&other.physical)
                        || other.physical.starts_with(&root.physical))
            })
        }) {
            return Err(PersistenceError::InvalidDocument(
                "native project paths overlap an unresolved reader or writer".to_owned(),
            ));
        }
    }
    table
        .insert(claim.owner.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn release_in_transaction(
    write: &redb::WriteTransaction,
    owner: &str,
) -> Result<(), PersistenceError> {
    write
        .open_table(NATIVE_PATH_CLAIMS)
        .map_err(error::redb)?
        .remove(owner)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn validate_claim(key: &str, claim: &NativePathClaim) -> Result<(), PersistenceError> {
    if key != claim.owner
        || claim.owner.is_empty()
        || claim.owner.len() > 512
        || claim.owner.chars().any(char::is_control)
        || claim.roots.is_empty()
        || claim.roots.len() > 32
        || claim.roots.iter().any(|root| {
            !root.physical.is_absolute()
                || !root.requested.is_absolute()
                || root.identity.is_empty()
                || root.identity.len() > 512
                || root.physical.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
                || root.physical.to_str().is_none_or(|path| path.len() > 4096)
                || root.requested.to_str().is_none_or(|path| path.len() > 4096)
        })
    {
        return Err(PersistenceError::InvalidDocument(
            "native path claim has invalid owner, canonical roots or identities".into(),
        ));
    }
    Ok(())
}
