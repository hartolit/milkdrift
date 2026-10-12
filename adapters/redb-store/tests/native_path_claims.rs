//! Complete native path sets conflict atomically and retain exact physical identity across reopen.
use milkdrift_persistence::{NativePathClaim, NativePathClaimStore, NativePathRoot};
use milkdrift_redb_store::RedbStore;

fn root(path: &std::path::Path, identity: &str, write: bool) -> NativePathRoot {
    NativePathRoot {
        requested: path.to_owned(),
        physical: path.to_owned(),
        identity: identity.into(),
        write,
    }
}

#[test]
fn physical_aliases_and_partial_overlap_cannot_create_another_owner()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let left = directory.path().join("left");
    let right = directory.path().join("right");
    let first = NativePathClaim {
        owner: "first".into(),
        roots: vec![root(&left, "same-physical-root", true)],
    };
    store.claim_native_paths(&first)?;
    // A platform can expose the same physical directory through distinct canonical spellings.
    let alias = NativePathClaim {
        owner: "alias".into(),
        roots: vec![root(&right, "same-physical-root", false)],
    };
    assert!(store.claim_native_paths(&alias).is_err());
    let partial = NativePathClaim {
        owner: "partial".into(),
        roots: vec![
            root(&right, "independent-root", true),
            root(&left.join("child"), "child", false),
        ],
    };
    assert!(store.claim_native_paths(&partial).is_err());
    assert!(store.native_path_claim("partial")?.is_none());
    let independent = NativePathClaim {
        owner: "independent".into(),
        roots: vec![root(&right, "independent-root", true)],
    };
    store.claim_native_paths(&independent)?;
    drop(store);
    let store = RedbStore::open(directory.path())?;
    store.claim_native_paths(&first)?;
    let mut changed = first.clone();
    changed.roots[0].write = false;
    assert!(store.claim_native_paths(&changed).is_err());
    assert!(store.claim_native_paths(&alias).is_err());
    store.release_native_paths("first")?;
    store.release_native_paths("independent")?;
    store.claim_native_paths(&alias)?;
    let shared = NativePathClaim {
        owner: "reader".into(),
        roots: vec![root(&right.join("child"), "child", false)],
    };
    store.claim_native_paths(&shared)?;
    assert!(store.claim_native_paths(&first).is_err());
    Ok(())
}
