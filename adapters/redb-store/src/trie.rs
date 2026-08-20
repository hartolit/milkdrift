use milkdrift_persistence::PersistenceError;
use redb::{ReadableTable, ReadableTableMetadata};
use serde::{Deserialize, Serialize};

use crate::{
    error, json,
    schema::{INTEGRITY_ROOTS, INTEGRITY_TRIE_NODES},
};

const ROOT_KEY: &str = "authenticated_catalog_roots";
const ROOT_SCHEMA_VERSION: u32 = 1;
const NODE_SCHEMA_VERSION: u32 = 1;
const RADIX: usize = 16;
const DEPTH: usize = 64;
const HASH_BYTES: usize = 32;
const NODE_KEY_BYTES: usize = 34;
const EMPTY_DOMAIN: &[u8] = b"milkdrift.redb.integrity-trie.empty.v1\0";
const BRANCH_DOMAIN: &[u8] = b"milkdrift.redb.integrity-trie.branch.v1\0";
const LEAF_DOMAIN: &[u8] = b"milkdrift.redb.integrity-trie.leaf.v1\0";
const PATH_DOMAIN: &[u8] = b"milkdrift.redb.integrity-trie.path.v1\0";
const PAYLOAD_DOMAIN: &[u8] = b"milkdrift.redb.integrity-trie.payload.v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum CatalogFamily {
    RunMembership = 1,
    Event = 2,
    Command = 3,
    RevisionIdentity = 4,
    RevisionContent = 5,
    RunnableIdentity = 6,
    RunnableOrdered = 7,
    TimerIdentity = 8,
    TimerOrdered = 9,
    LeaseIdentity = 10,
    LeaseOrdered = 11,
    WorkspaceDomain = 12,
    WorkspaceScope = 13,
    WorkspaceValue = 14,
    WorkspaceValueHead = 15,
    Artifact = 16,
    ArtifactPublication = 17,
    ArtifactPath = 18,
    RunArtifactOwnership = 19,
    NonterminalRun = 20,
    RunnableRunHead = 21,
    HistoryAccumulator = 22,
    EventHistoryCheckpoint = 23,
    SnapshotIdentity = 24,
    SnapshotOrdered = 25,
    SnapshotLatest = 26,
    RunnableBucket = 27,
    RunnableBucketEntry = 28,
    ArtifactReferenceOccurrence = 29,
    ArtifactDeleteGuard = 30,
}

impl CatalogFamily {
    pub(crate) const ALL: [Self; 30] = [
        Self::RunMembership,
        Self::Event,
        Self::Command,
        Self::RevisionIdentity,
        Self::RevisionContent,
        Self::RunnableIdentity,
        Self::RunnableOrdered,
        Self::TimerIdentity,
        Self::TimerOrdered,
        Self::LeaseIdentity,
        Self::LeaseOrdered,
        Self::WorkspaceDomain,
        Self::WorkspaceScope,
        Self::WorkspaceValue,
        Self::WorkspaceValueHead,
        Self::Artifact,
        Self::ArtifactPublication,
        Self::ArtifactPath,
        Self::RunArtifactOwnership,
        Self::NonterminalRun,
        Self::RunnableRunHead,
        Self::HistoryAccumulator,
        Self::EventHistoryCheckpoint,
        Self::SnapshotIdentity,
        Self::SnapshotOrdered,
        Self::SnapshotLatest,
        Self::RunnableBucket,
        Self::RunnableBucketEntry,
        Self::ArtifactReferenceOccurrence,
        Self::ArtifactDeleteGuard,
    ];

    const fn index(self) -> usize {
        self as usize - 1
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IntegrityRoots {
    schema_version: u32,
    roots: Vec<[u8; HASH_BYTES]>,
}

impl IntegrityRoots {
    fn empty() -> Self {
        Self {
            schema_version: ROOT_SCHEMA_VERSION,
            roots: CatalogFamily::ALL
                .iter()
                .map(|family| empty_hashes(*family)[0])
                .collect(),
        }
    }

    fn root(&self, family: CatalogFamily) -> Result<[u8; HASH_BYTES], PersistenceError> {
        self.validate()?;
        self.roots
            .get(family.index())
            .copied()
            .ok_or_else(|| error::corruption("integrity root family is absent"))
    }

    fn set_root(
        &mut self,
        family: CatalogFamily,
        root: [u8; HASH_BYTES],
    ) -> Result<(), PersistenceError> {
        self.validate()?;
        *self
            .roots
            .get_mut(family.index())
            .ok_or_else(|| error::corruption("integrity root family is absent"))? = root;
        Ok(())
    }

    fn validate(&self) -> Result<(), PersistenceError> {
        if self.schema_version != ROOT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedVersion {
                document: "integrity_roots",
                found: self.schema_version,
                supported: ROOT_SCHEMA_VERSION,
            });
        }
        if self.roots.len() != CatalogFamily::ALL.len() {
            return Err(error::corruption(
                "integrity root document does not contain every fixed family",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrieLeaf {
    pub(crate) path: [u8; HASH_BYTES],
    pub(crate) logical_key: Vec<u8>,
    pub(crate) payload_digest: [u8; HASH_BYTES],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TriePage {
    pub(crate) root: [u8; HASH_BYTES],
    pub(crate) leaves: Vec<TrieLeaf>,
    pub(crate) next_path: Option<[u8; HASH_BYTES]>,
}

pub(crate) fn initialize(write: &redb::WriteTransaction) -> Result<(), PersistenceError> {
    let mut roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    if roots.len().map_err(error::redb)? != 0 {
        return Err(error::corruption(
            "integrity roots already exist during schema initialization",
        ));
    }
    let bytes = json::encode(&IntegrityRoots::empty(), "integrity roots")?;
    roots
        .insert(ROOT_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    drop(roots);
    let _nodes = write
        .open_table(INTEGRITY_TRIE_NODES)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn validate_roots(read: &redb::ReadTransaction) -> Result<(), PersistenceError> {
    let roots = read.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let document = load_roots(&roots)?;
    let nodes = read.open_table(INTEGRITY_TRIE_NODES).map_err(error::redb)?;
    for family in CatalogFamily::ALL {
        let root = document.root(family)?;
        let empty = empty_hashes(family);
        if root == empty[0] {
            continue;
        }
        let key = node_key(family, 0, &[0; HASH_BYTES]);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie top node is missing"))?
            .value()
            .to_vec();
        let _children = decode_branch(&bytes, family, 0, root)?;
    }
    Ok(())
}

pub(crate) fn validate_roots_in_transaction(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let document = load_roots(&roots)?;
    let nodes = write
        .open_table(INTEGRITY_TRIE_NODES)
        .map_err(error::redb)?;
    for family in CatalogFamily::ALL {
        let root = document.root(family)?;
        let empty = empty_hashes(family);
        if root == empty[0] {
            continue;
        }
        let key = node_key(family, 0, &[0; HASH_BYTES]);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie top node is missing"))?
            .value()
            .to_vec();
        let _children = decode_branch(&bytes, family, 0, root)?;
    }
    Ok(())
}

pub(crate) fn family_root(
    read: &redb::ReadTransaction,
    family: CatalogFamily,
) -> Result<[u8; HASH_BYTES], PersistenceError> {
    let roots = read.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    load_roots(&roots)?.root(family)
}

pub(crate) fn root_anchor(
    read: &redb::ReadTransaction,
) -> Result<[u8; HASH_BYTES], PersistenceError> {
    let roots = read.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let document = load_roots(&roots)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.redb.integrity-root-anchor.v1\0");
    hasher.update(&document.schema_version.to_be_bytes());
    for root in document.roots {
        hasher.update(&root);
    }
    Ok(*hasher.finalize().as_bytes())
}

pub(crate) fn family_root_in_transaction(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
) -> Result<[u8; HASH_BYTES], PersistenceError> {
    let roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    load_roots(&roots)?.root(family)
}

pub(crate) fn digest_payload(family: CatalogFamily, bytes: &[u8]) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PAYLOAD_DOMAIN);
    hasher.update(&[family as u8]);
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

pub(crate) fn hashed_path(family: CatalogFamily, logical_key: &[u8]) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PATH_DOMAIN);
    hasher.update(&[family as u8]);
    hasher.update(&(logical_key.len() as u64).to_be_bytes());
    hasher.update(logical_key);
    *hasher.finalize().as_bytes()
}

pub(crate) fn ordered_path(
    family: CatalogFamily,
    ordered_prefix: &[u8],
    logical_key: &[u8],
) -> Result<[u8; HASH_BYTES], PersistenceError> {
    if ordered_prefix.len() >= HASH_BYTES {
        return Err(PersistenceError::Bounds {
            location: "integrity_trie.ordered_prefix",
            reason: "ordered prefix must leave bytes for collision-resistant identity".to_owned(),
        });
    }
    let mut path = hashed_path(family, logical_key);
    path[..ordered_prefix.len()].copy_from_slice(ordered_prefix);
    Ok(path)
}

pub(crate) fn put(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
    path: [u8; HASH_BYTES],
    logical_key: &[u8],
    payload_digest: [u8; HASH_BYTES],
) -> Result<Option<[u8; HASH_BYTES]>, PersistenceError> {
    mutate(write, family, path, logical_key, Some(payload_digest))
}

pub(crate) fn remove(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
    path: [u8; HASH_BYTES],
    logical_key: &[u8],
) -> Result<Option<[u8; HASH_BYTES]>, PersistenceError> {
    mutate(write, family, path, logical_key, None)
}

pub(crate) fn verify_member(
    read: &redb::ReadTransaction,
    family: CatalogFamily,
    path: [u8; HASH_BYTES],
    logical_key: &[u8],
) -> Result<Option<[u8; HASH_BYTES]>, PersistenceError> {
    let roots = read.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let root = load_roots(&roots)?.root(family)?;
    drop(roots);
    let nodes = read.open_table(INTEGRITY_TRIE_NODES).map_err(error::redb)?;
    verify_member_in_table(&nodes, family, root, path, logical_key)
}

pub(crate) fn verify_member_in_transaction(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
    path: [u8; HASH_BYTES],
    logical_key: &[u8],
) -> Result<Option<[u8; HASH_BYTES]>, PersistenceError> {
    let roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let root = load_roots(&roots)?.root(family)?;
    drop(roots);
    let nodes = write
        .open_table(INTEGRITY_TRIE_NODES)
        .map_err(error::redb)?;
    verify_member_in_table(&nodes, family, root, path, logical_key)
}

pub(crate) fn page(
    read: &redb::ReadTransaction,
    family: CatalogFamily,
    expected_root: Option<[u8; HASH_BYTES]>,
    after: Option<[u8; HASH_BYTES]>,
    limit: usize,
) -> Result<TriePage, PersistenceError> {
    if limit == 0 {
        return Err(PersistenceError::Bounds {
            location: "integrity_trie.page.limit",
            reason: "must be nonzero".to_owned(),
        });
    }
    let roots = read.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let root = load_roots(&roots)?.root(family)?;
    if expected_root.is_some_and(|expected| expected != root) {
        return Err(PersistenceError::InvalidCursor(
            "authenticated catalog root changed since the previous page".to_owned(),
        ));
    }
    drop(roots);
    let nodes = read.open_table(INTEGRITY_TRIE_NODES).map_err(error::redb)?;
    page_from_root(&nodes, family, root, after, limit)
}

pub(crate) fn page_in_transaction(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
    expected_root: Option<[u8; HASH_BYTES]>,
    after: Option<[u8; HASH_BYTES]>,
    limit: usize,
) -> Result<TriePage, PersistenceError> {
    if limit == 0 {
        return Err(PersistenceError::Bounds {
            location: "integrity_trie.page.limit",
            reason: "must be nonzero".to_owned(),
        });
    }
    let roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let root = load_roots(&roots)?.root(family)?;
    if expected_root.is_some_and(|expected| expected != root) {
        return Err(PersistenceError::InvalidCursor(
            "authenticated catalog root changed since the previous page".to_owned(),
        ));
    }
    drop(roots);
    let nodes = write
        .open_table(INTEGRITY_TRIE_NODES)
        .map_err(error::redb)?;
    page_from_root(&nodes, family, root, after, limit)
}

pub(crate) fn predecessor_in_transaction(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
    expected_root: Option<[u8; HASH_BYTES]>,
    before: [u8; HASH_BYTES],
) -> Result<Option<TrieLeaf>, PersistenceError> {
    let roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
    let root = load_roots(&roots)?.root(family)?;
    if expected_root.is_some_and(|expected| expected != root) {
        return Err(PersistenceError::InvalidCursor(
            "authenticated catalog root changed since the previous lookup".to_owned(),
        ));
    }
    drop(roots);
    let nodes = write
        .open_table(INTEGRITY_TRIE_NODES)
        .map_err(error::redb)?;
    predecessor_from_root(&nodes, family, root, before)
}

fn page_from_root<T>(
    nodes: &T,
    family: CatalogFamily,
    root: [u8; HASH_BYTES],
    after: Option<[u8; HASH_BYTES]>,
    limit: usize,
) -> Result<TriePage, PersistenceError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    let empty = empty_hashes(family);
    if root == empty[0] {
        return Ok(TriePage {
            root,
            leaves: Vec::new(),
            next_path: None,
        });
    }
    let mut leaves = Vec::with_capacity(limit);
    let mut position = after;
    let mut has_more = false;
    while leaves.len() <= limit {
        let Some(leaf) = successor(nodes, family, root, &empty, position)? else {
            break;
        };
        position = Some(leaf.path);
        if leaves.len() == limit {
            has_more = true;
            break;
        }
        leaves.push(leaf);
    }
    let next_path = if has_more {
        Some(
            leaves
                .last()
                .ok_or_else(|| error::corruption("authenticated page lost its cursor"))?
                .path,
        )
    } else {
        None
    };
    Ok(TriePage {
        root,
        leaves,
        next_path,
    })
}

fn predecessor_from_root<T>(
    nodes: &T,
    family: CatalogFamily,
    root: [u8; HASH_BYTES],
    before: [u8; HASH_BYTES],
) -> Result<Option<TrieLeaf>, PersistenceError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    let empty = empty_hashes(family);
    if root == empty[0] {
        return Ok(None);
    }
    let mut expected = root;
    let mut branches = Vec::with_capacity(DEPTH);
    for depth in 0..DEPTH {
        if expected == empty[depth] {
            break;
        }
        let key = node_key(family, depth, &before);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie branch is missing"))?
            .value()
            .to_vec();
        let children = decode_branch(&bytes, family, depth, expected)?;
        expected = children[nibble(&before, depth)];
        branches.push(children);
    }
    for depth in (0..branches.len()).rev() {
        let current = nibble(&before, depth);
        for candidate in (0..current).rev() {
            let child = branches[depth][candidate];
            if child != empty[depth + 1] {
                let mut prefix = before;
                set_nibble(&mut prefix, depth, candidate);
                fill_after(&mut prefix, depth + 1);
                return rightmost(nodes, family, child, &empty, depth + 1, prefix);
            }
        }
    }
    Ok(None)
}

fn mutate(
    write: &redb::WriteTransaction,
    family: CatalogFamily,
    path: [u8; HASH_BYTES],
    logical_key: &[u8],
    replacement: Option<[u8; HASH_BYTES]>,
) -> Result<Option<[u8; HASH_BYTES]>, PersistenceError> {
    let mut roots_document = {
        let roots = write.open_table(INTEGRITY_ROOTS).map_err(error::redb)?;
        load_roots(&roots)?
    };
    let root = roots_document.root(family)?;
    let empty = empty_hashes(family);
    let mut nodes = write
        .open_table(INTEGRITY_TRIE_NODES)
        .map_err(error::redb)?;
    let mut branches = Vec::with_capacity(DEPTH);
    let mut expected = root;
    let mut empty_tail = false;
    for depth in 0..DEPTH {
        let children = if empty_tail || expected == empty[depth] {
            empty_tail = true;
            vec![empty[depth + 1]; RADIX]
        } else {
            let key = node_key(family, depth, &path);
            let bytes = nodes
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("authenticated trie branch is missing"))?
                .value()
                .to_vec();
            let children = decode_branch(&bytes, family, depth, expected)?;
            children.to_vec()
        };
        expected = children[nibble(&path, depth)];
        branches.push(children);
    }

    let leaf_key = node_key(family, DEPTH, &path);
    let previous = if expected == empty[DEPTH] {
        if nodes
            .get(leaf_key.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(
                "authenticated trie contains an uncommitted leaf",
            ));
        }
        None
    } else {
        let bytes = nodes
            .get(leaf_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie leaf is missing"))?
            .value()
            .to_vec();
        let leaf = decode_leaf(&bytes, family, expected)?;
        if leaf.path != path || leaf.logical_key != logical_key {
            return Err(error::corruption(
                "authenticated trie path collision or logical-key mismatch",
            ));
        }
        Some(leaf.payload_digest)
    };

    let mut child_hash = if let Some(payload_digest) = replacement {
        let leaf = TrieLeaf {
            path,
            logical_key: logical_key.to_vec(),
            payload_digest,
        };
        let bytes = encode_leaf(&leaf)?;
        nodes
            .insert(leaf_key.as_slice(), bytes.as_slice())
            .map_err(error::redb)?;
        hash_leaf(family, &leaf)
    } else {
        nodes.remove(leaf_key.as_slice()).map_err(error::redb)?;
        empty[DEPTH]
    };

    for depth in (0..DEPTH).rev() {
        let mut children = branches
            .pop()
            .ok_or_else(|| error::corruption("authenticated trie proof lost a branch"))?;
        children[nibble(&path, depth)] = child_hash;
        let key = node_key(family, depth, &path);
        if children.iter().all(|child| *child == empty[depth + 1]) {
            nodes.remove(key.as_slice()).map_err(error::redb)?;
            child_hash = empty[depth];
        } else {
            let child_array: [[u8; HASH_BYTES]; RADIX] = children
                .try_into()
                .map_err(|_| error::corruption("authenticated trie branch has wrong arity"))?;
            child_hash = hash_branch(family, depth, &child_array);
            let bytes = encode_branch(depth, &child_array);
            nodes
                .insert(key.as_slice(), bytes.as_slice())
                .map_err(error::redb)?;
        }
    }
    drop(nodes);
    roots_document.set_root(family, child_hash)?;
    persist_roots(write, &roots_document)?;
    Ok(previous)
}

fn verify_member_in_table<T>(
    nodes: &T,
    family: CatalogFamily,
    root: [u8; HASH_BYTES],
    path: [u8; HASH_BYTES],
    logical_key: &[u8],
) -> Result<Option<[u8; HASH_BYTES]>, PersistenceError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    let empty = empty_hashes(family);
    let mut expected = root;
    for depth in 0..DEPTH {
        if expected == empty[depth] {
            return Ok(None);
        }
        let key = node_key(family, depth, &path);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie branch is missing"))?
            .value()
            .to_vec();
        let children = decode_branch(&bytes, family, depth, expected)?;
        expected = children[nibble(&path, depth)];
    }
    if expected == empty[DEPTH] {
        return Ok(None);
    }
    let key = node_key(family, DEPTH, &path);
    let bytes = nodes
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("authenticated trie leaf is missing"))?
        .value()
        .to_vec();
    let leaf = decode_leaf(&bytes, family, expected)?;
    if leaf.path != path || leaf.logical_key != logical_key {
        return Err(error::corruption(
            "authenticated trie path collision or logical-key mismatch",
        ));
    }
    Ok(Some(leaf.payload_digest))
}

fn successor<T>(
    nodes: &T,
    family: CatalogFamily,
    root: [u8; HASH_BYTES],
    empty: &[[u8; HASH_BYTES]; DEPTH + 1],
    after: Option<[u8; HASH_BYTES]>,
) -> Result<Option<TrieLeaf>, PersistenceError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    let Some(after) = after else {
        return leftmost(nodes, family, root, empty, 0, [0; HASH_BYTES]);
    };
    let mut expected = root;
    let mut branches = Vec::with_capacity(DEPTH);
    for depth in 0..DEPTH {
        if expected == empty[depth] {
            break;
        }
        let key = node_key(family, depth, &after);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie branch is missing"))?
            .value()
            .to_vec();
        let children = decode_branch(&bytes, family, depth, expected)?;
        expected = children[nibble(&after, depth)];
        branches.push(children);
    }
    for depth in (0..branches.len()).rev() {
        let current = nibble(&after, depth);
        for (candidate, child) in branches[depth]
            .iter()
            .copied()
            .enumerate()
            .skip(current + 1)
        {
            if child != empty[depth + 1] {
                let mut prefix = after;
                set_nibble(&mut prefix, depth, candidate);
                clear_after(&mut prefix, depth + 1);
                return leftmost(nodes, family, child, empty, depth + 1, prefix);
            }
        }
    }
    Ok(None)
}

fn leftmost<T>(
    nodes: &T,
    family: CatalogFamily,
    mut expected: [u8; HASH_BYTES],
    empty: &[[u8; HASH_BYTES]; DEPTH + 1],
    mut depth: usize,
    mut path: [u8; HASH_BYTES],
) -> Result<Option<TrieLeaf>, PersistenceError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    if expected == empty[depth] {
        return Ok(None);
    }
    while depth < DEPTH {
        let key = node_key(family, depth, &path);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie branch is missing"))?
            .value()
            .to_vec();
        let children = decode_branch(&bytes, family, depth, expected)?;
        let (child_index, child_hash) = children
            .iter()
            .enumerate()
            .find(|(_, child)| **child != empty[depth + 1])
            .ok_or_else(|| error::corruption("authenticated trie branch has no committed child"))?;
        set_nibble(&mut path, depth, child_index);
        expected = *child_hash;
        depth += 1;
    }
    let key = node_key(family, DEPTH, &path);
    let bytes = nodes
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("authenticated trie leaf is missing"))?
        .value()
        .to_vec();
    let leaf = decode_leaf(&bytes, family, expected)?;
    if leaf.path != path {
        return Err(error::corruption(
            "authenticated trie leaf path disagrees with traversal",
        ));
    }
    Ok(Some(leaf))
}

fn rightmost<T>(
    nodes: &T,
    family: CatalogFamily,
    mut expected: [u8; HASH_BYTES],
    empty: &[[u8; HASH_BYTES]; DEPTH + 1],
    mut depth: usize,
    mut path: [u8; HASH_BYTES],
) -> Result<Option<TrieLeaf>, PersistenceError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    if expected == empty[depth] {
        return Ok(None);
    }
    while depth < DEPTH {
        let key = node_key(family, depth, &path);
        let bytes = nodes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated trie branch is missing"))?
            .value()
            .to_vec();
        let children = decode_branch(&bytes, family, depth, expected)?;
        let (child_index, child_hash) = children
            .iter()
            .enumerate()
            .rev()
            .find(|(_, child)| **child != empty[depth + 1])
            .ok_or_else(|| error::corruption("authenticated trie branch has no committed child"))?;
        set_nibble(&mut path, depth, child_index);
        expected = *child_hash;
        depth += 1;
    }
    let key = node_key(family, DEPTH, &path);
    let bytes = nodes
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("authenticated trie leaf is missing"))?
        .value()
        .to_vec();
    let leaf = decode_leaf(&bytes, family, expected)?;
    if leaf.path != path {
        return Err(error::corruption(
            "authenticated trie leaf path disagrees with traversal",
        ));
    }
    Ok(Some(leaf))
}

fn load_roots<T>(roots: &T) -> Result<IntegrityRoots, PersistenceError>
where
    T: ReadableTable<&'static str, &'static [u8]>,
{
    if roots.len().map_err(error::redb)? != 1 {
        return Err(error::corruption(
            "integrity roots must contain exactly one trust anchor",
        ));
    }
    let bytes = roots
        .get(ROOT_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("integrity root trust anchor is missing"))?;
    let document: IntegrityRoots = json::decode(bytes.value(), "integrity roots")?;
    document.validate()?;
    Ok(document)
}

fn persist_roots(
    write: &redb::WriteTransaction,
    roots: &IntegrityRoots,
) -> Result<(), PersistenceError> {
    roots.validate()?;
    let bytes = json::encode(roots, "integrity roots")?;
    write
        .open_table(INTEGRITY_ROOTS)
        .map_err(error::redb)?
        .insert(ROOT_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn decode_branch(
    bytes: &[u8],
    family: CatalogFamily,
    depth: usize,
    expected: [u8; HASH_BYTES],
) -> Result<[[u8; HASH_BYTES]; RADIX], PersistenceError> {
    const BRANCH_BYTES: usize = 3 + RADIX * HASH_BYTES;
    if bytes.len() != BRANCH_BYTES || bytes[0] != 1 {
        return Err(error::corruption(
            "authenticated trie branch has an invalid binary shape",
        ));
    }
    if u32::from(bytes[1]) != NODE_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "integrity_trie_node",
            found: u32::from(bytes[1]),
            supported: NODE_SCHEMA_VERSION,
        });
    }
    if usize::from(bytes[2]) != depth {
        return Err(error::corruption(
            "authenticated trie branch depth disagrees with its key",
        ));
    }
    let mut children = [[0; HASH_BYTES]; RADIX];
    for (index, child) in children.iter_mut().enumerate() {
        let start = 3 + index * HASH_BYTES;
        child.copy_from_slice(&bytes[start..start + HASH_BYTES]);
    }
    if hash_branch(family, depth, &children) != expected {
        return Err(error::corruption(
            "authenticated trie branch hash disagrees with its parent",
        ));
    }
    Ok(children)
}

fn decode_leaf(
    bytes: &[u8],
    family: CatalogFamily,
    expected: [u8; HASH_BYTES],
) -> Result<TrieLeaf, PersistenceError> {
    const FIXED_BYTES: usize = 2 + HASH_BYTES + 2 + HASH_BYTES;
    if bytes.len() < FIXED_BYTES || bytes[0] != 2 {
        return Err(error::corruption(
            "authenticated trie leaf has an invalid binary shape",
        ));
    }
    if u32::from(bytes[1]) != NODE_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "integrity_trie_node",
            found: u32::from(bytes[1]),
            supported: NODE_SCHEMA_VERSION,
        });
    }
    let mut path = [0; HASH_BYTES];
    path.copy_from_slice(&bytes[2..2 + HASH_BYTES]);
    let key_length_offset = 2 + HASH_BYTES;
    let key_length = usize::from(u16::from_be_bytes([
        bytes[key_length_offset],
        bytes[key_length_offset + 1],
    ]));
    let expected_length = FIXED_BYTES
        .checked_add(key_length)
        .ok_or_else(|| error::corruption("authenticated trie leaf length overflowed"))?;
    if bytes.len() != expected_length {
        return Err(error::corruption(
            "authenticated trie leaf key length is inconsistent",
        ));
    }
    let key_start = key_length_offset + 2;
    let key_end = key_start + key_length;
    let logical_key = bytes[key_start..key_end].to_vec();
    let mut payload_digest = [0; HASH_BYTES];
    payload_digest.copy_from_slice(&bytes[key_end..key_end + HASH_BYTES]);
    let leaf = TrieLeaf {
        path,
        logical_key,
        payload_digest,
    };
    if hash_leaf(family, &leaf) != expected {
        return Err(error::corruption(
            "authenticated trie leaf hash disagrees with its parent",
        ));
    }
    Ok(leaf)
}

fn encode_branch(depth: usize, children: &[[u8; HASH_BYTES]; RADIX]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(3 + RADIX * HASH_BYTES);
    bytes.push(1);
    bytes.push(NODE_SCHEMA_VERSION as u8);
    bytes.push(depth as u8);
    for child in children {
        bytes.extend_from_slice(child);
    }
    bytes
}

fn encode_leaf(leaf: &TrieLeaf) -> Result<Vec<u8>, PersistenceError> {
    let key_length =
        u16::try_from(leaf.logical_key.len()).map_err(|_| PersistenceError::Bounds {
            location: "integrity_trie.logical_key",
            reason: "logical key exceeds the fixed binary leaf encoding".to_owned(),
        })?;
    let mut bytes = Vec::with_capacity(2 + HASH_BYTES + 2 + leaf.logical_key.len() + HASH_BYTES);
    bytes.push(2);
    bytes.push(NODE_SCHEMA_VERSION as u8);
    bytes.extend_from_slice(&leaf.path);
    bytes.extend_from_slice(&key_length.to_be_bytes());
    bytes.extend_from_slice(&leaf.logical_key);
    bytes.extend_from_slice(&leaf.payload_digest);
    Ok(bytes)
}

fn empty_hashes(family: CatalogFamily) -> [[u8; HASH_BYTES]; DEPTH + 1] {
    let mut hashes = [[0; HASH_BYTES]; DEPTH + 1];
    let mut leaf = blake3::Hasher::new();
    leaf.update(EMPTY_DOMAIN);
    leaf.update(&[family as u8]);
    leaf.update(&(DEPTH as u16).to_be_bytes());
    hashes[DEPTH] = *leaf.finalize().as_bytes();
    for depth in (0..DEPTH).rev() {
        hashes[depth] = hash_branch(family, depth, &[hashes[depth + 1]; RADIX]);
    }
    hashes
}

fn hash_branch(
    family: CatalogFamily,
    depth: usize,
    children: &[[u8; HASH_BYTES]; RADIX],
) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(BRANCH_DOMAIN);
    hasher.update(&[family as u8]);
    hasher.update(&(depth as u16).to_be_bytes());
    for child in children {
        hasher.update(child);
    }
    *hasher.finalize().as_bytes()
}

fn hash_leaf(family: CatalogFamily, leaf: &TrieLeaf) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LEAF_DOMAIN);
    hasher.update(&[family as u8]);
    hasher.update(&leaf.path);
    hasher.update(&(leaf.logical_key.len() as u64).to_be_bytes());
    hasher.update(&leaf.logical_key);
    hasher.update(&leaf.payload_digest);
    *hasher.finalize().as_bytes()
}

fn node_key(family: CatalogFamily, depth: usize, path: &[u8; HASH_BYTES]) -> [u8; NODE_KEY_BYTES] {
    let mut key = [0; NODE_KEY_BYTES];
    key[0] = family as u8;
    // `depth` is supplied only by loops bounded by the fixed `DEPTH == 64`.
    key[1] = depth as u8;
    key[2..].copy_from_slice(path);
    let full_nibbles = depth / 2;
    let odd = depth % 2;
    let keep_bytes = full_nibbles + usize::from(odd != 0);
    if odd != 0 {
        key[2 + full_nibbles] &= 0xf0;
    }
    key[2 + keep_bytes..].fill(0);
    key
}

const fn nibble(path: &[u8; HASH_BYTES], depth: usize) -> usize {
    let byte = path[depth / 2];
    if depth.is_multiple_of(2) {
        (byte >> 4) as usize
    } else {
        (byte & 0x0f) as usize
    }
}

fn set_nibble(path: &mut [u8; HASH_BYTES], depth: usize, value: usize) {
    let byte = &mut path[depth / 2];
    if depth.is_multiple_of(2) {
        *byte = (*byte & 0x0f) | ((value as u8) << 4);
    } else {
        *byte = (*byte & 0xf0) | value as u8;
    }
}

fn clear_after(path: &mut [u8; HASH_BYTES], depth: usize) {
    if depth >= DEPTH {
        return;
    }
    let full_bytes = depth / 2;
    if depth.is_multiple_of(2) {
        path[full_bytes..].fill(0);
    } else {
        path[full_bytes] &= 0xf0;
        path[full_bytes + 1..].fill(0);
    }
}

fn fill_after(path: &mut [u8; HASH_BYTES], depth: usize) {
    if depth >= DEPTH {
        return;
    }
    let full_bytes = depth / 2;
    if depth.is_multiple_of(2) {
        path[full_bytes..].fill(u8::MAX);
    } else {
        path[full_bytes] |= 0x0f;
        path[full_bytes + 1..].fill(u8::MAX);
    }
}

#[cfg(test)]
mod tests {
    use redb::Database;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn authenticated_membership_absence_collision_and_tamper_are_fail_closed()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let database = Database::create(directory.path().join("trie.redb"))?;
        let write = database.begin_write()?;
        initialize(&write)?;
        write.commit()?;

        let logical_key = b"command/run-one/command-one";
        let path = hashed_path(CatalogFamily::Command, logical_key);
        let payload = digest_payload(CatalogFamily::Command, b"checked-command-document");
        let write = database.begin_write()?;
        assert_eq!(
            put(&write, CatalogFamily::Command, path, logical_key, payload,)?,
            None
        );
        assert_eq!(
            put(&write, CatalogFamily::Command, path, logical_key, payload,)?,
            Some(payload)
        );
        assert!(matches!(
            put(
                &write,
                CatalogFamily::Command,
                path,
                b"different-logical-key-with-forced-path",
                payload,
            ),
            Err(PersistenceError::Corruption(_))
                | Err(PersistenceError::Storage {
                    class: milkdrift_persistence::StorageFailureClass::Corruption,
                    ..
                })
        ));
        write.commit()?;

        let read = database.begin_read()?;
        assert_eq!(
            verify_member(&read, CatalogFamily::Command, path, logical_key)?,
            Some(payload)
        );
        let absent_key = b"command/run-one/never-issued";
        assert_eq!(
            verify_member(
                &read,
                CatalogFamily::Command,
                hashed_path(CatalogFamily::Command, absent_key),
                absent_key,
            )?,
            None
        );
        drop(read);

        let write = database.begin_write()?;
        write
            .open_table(INTEGRITY_TRIE_NODES)?
            .remove(node_key(CatalogFamily::Command, DEPTH, &path).as_slice())?;
        write.commit()?;
        let read = database.begin_read()?;
        assert!(matches!(
            verify_member(&read, CatalogFamily::Command, path, logical_key),
            Err(PersistenceError::Corruption(_))
                | Err(PersistenceError::Storage {
                    class: milkdrift_persistence::StorageFailureClass::Corruption,
                    ..
                })
        ));
        Ok(())
    }

    #[test]
    fn ordered_pages_are_complete_bounded_and_root_bound() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = TempDir::new()?;
        let database = Database::create(directory.path().join("trie.redb"))?;
        let write = database.begin_write()?;
        initialize(&write)?;
        for index in (0_u64..37).rev() {
            let key = format!("timer-{index:03}");
            let path = ordered_path(
                CatalogFamily::TimerOrdered,
                &index.to_be_bytes(),
                key.as_bytes(),
            )?;
            let payload = digest_payload(CatalogFamily::TimerOrdered, key.as_bytes());
            put(
                &write,
                CatalogFamily::TimerOrdered,
                path,
                key.as_bytes(),
                payload,
            )?;
        }
        write.commit()?;

        let mut after = None;
        let mut expected_root = None;
        let mut observed = Vec::new();
        loop {
            let read = database.begin_read()?;
            let page = page(&read, CatalogFamily::TimerOrdered, expected_root, after, 5)?;
            expected_root = Some(page.root);
            observed.extend(page.leaves.iter().map(|leaf| leaf.logical_key.clone()));
            let Some(next) = page.next_path else {
                break;
            };
            after = Some(next);
        }
        assert_eq!(observed.len(), 37);
        for (index, key) in observed.iter().enumerate() {
            assert_eq!(key, format!("timer-{index:03}").as_bytes());
        }

        let removed_key = b"timer-004";
        let removed_path = ordered_path(
            CatalogFamily::TimerOrdered,
            &4_u64.to_be_bytes(),
            removed_key,
        )?;
        let write = database.begin_write()?;
        assert!(
            remove(
                &write,
                CatalogFamily::TimerOrdered,
                removed_path,
                removed_key,
            )?
            .is_some()
        );
        let late_key = b"timer-999";
        let late_path = ordered_path(
            CatalogFamily::TimerOrdered,
            &999_u64.to_be_bytes(),
            late_key,
        )?;
        put(
            &write,
            CatalogFamily::TimerOrdered,
            late_path,
            late_key,
            digest_payload(CatalogFamily::TimerOrdered, late_key),
        )?;
        write.commit()?;
        let read = database.begin_read()?;
        let resumed = page(
            &read,
            CatalogFamily::TimerOrdered,
            None,
            Some(removed_path),
            1,
        )?;
        assert_eq!(resumed.leaves[0].logical_key, b"timer-005");
        assert!(matches!(
            page(&read, CatalogFamily::TimerOrdered, expected_root, None, 5,),
            Err(PersistenceError::InvalidCursor(_))
        ));
        Ok(())
    }
}
