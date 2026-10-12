use crate::{RedbStore, error, json, schema::NATIVE_PATH_CLAIMS};
use milkdrift_persistence::{NativePathClaim, NativePathClaimStore, PersistenceError};
use redb::{ReadableTable, ReadableTableMetadata};

const MAX_CLAIMS: u64 = 4096;

impl NativePathClaimStore for RedbStore {
    fn claim_native_paths(&self, claim: &NativePathClaim) -> Result<(), PersistenceError> {
        if claim.owner.is_empty()
            || claim.owner.len() > 512
            || claim.roots.is_empty()
            || claim.roots.len() > 32
            || claim.roots.iter().any(|root| {
                !root.physical.is_absolute()
                    || root.identity.is_empty()
                    || root.identity.len() > 512
                    || root.physical.to_str().is_none_or(|path| path.len() > 4096)
                    || root.requested.to_str().is_none_or(|path| path.len() > 4096)
            })
        {
            return Err(PersistenceError::InvalidDocument(
                "native path claim has invalid owner, roots or identities".to_owned(),
            ));
        }
        let bytes = json::encode(claim, "native path claim")?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut table = write.open_table(NATIVE_PATH_CLAIMS).map_err(error::redb)?;
        if let Some(value) = table.get(claim.owner.as_str()).map_err(error::redb)? {
            let existing: NativePathClaim = json::decode(value.value(), "native path claim")?;
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
                reason: "native claim capacity is full; unresolved owners remain protected"
                    .to_owned(),
            });
        }
        for row in table.iter().map_err(error::redb)? {
            let (_, value) = row.map_err(error::redb)?;
            let existing: NativePathClaim = json::decode(value.value(), "native path claim")?;
            if claim.roots.iter().any(|root| {
                existing.roots.iter().any(|other| {
                    (root.write || other.write)
                        && (root.physical.starts_with(&other.physical)
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
        drop(table);
        write.commit().map_err(error::redb)
    }

    fn native_path_claim(&self, owner: &str) -> Result<Option<NativePathClaim>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(NATIVE_PATH_CLAIMS).map_err(error::redb)?;
        table
            .get(owner)
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "native path claim"))
            .transpose()
    }

    fn release_native_paths(&self, owner: &str) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        {
            write
                .open_table(NATIVE_PATH_CLAIMS)
                .map_err(error::redb)?
                .remove(owner)
                .map_err(error::redb)?;
        }
        write.commit().map_err(error::redb)
    }
}
