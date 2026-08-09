use std::cell::Cell;
use std::collections::BTreeSet;
use std::fmt::{self, Formatter};
use std::path::{Component, Path};

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, Visitor};

use crate::bounded::{BoundedReadError, read_bounded_file};
use crate::{HubError, HubStructuralLimit};

/// Thirty-two MiB accommodates very large Llama indexes while bounding raw JSON ownership.
const MAX_WEIGHT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
/// Realistic Llama models have thousands of tensors; this retains substantial extension headroom.
const MAX_INDEX_WEIGHT_ENTRIES: usize = 65_536;
/// Tensor names in supported model families are normally well below 256 bytes.
const MAX_INDEX_TENSOR_NAME_BYTES: usize = 1024;
/// Repository paths in supported layouts are short, even when shards live below one directory.
const MAX_REPOSITORY_PATH_BYTES: usize = 1024;
pub(crate) const MAX_SELECTED_WEIGHT_SHARDS: usize = 256;

pub(crate) fn read_index(path: &Path) -> Result<Vec<u8>, HubError> {
    read_bounded_file(path, MAX_WEIGHT_INDEX_BYTES).map_err(|error| match error {
        BoundedReadError::Io(error) => HubError::ReadIndex(error),
        BoundedReadError::Limit => {
            HubError::StructuralLimitExceeded(HubStructuralLimit::WeightIndexBytes)
        }
    })
}

pub(crate) fn indexed_weights(
    bytes: &[u8],
    available: &BTreeSet<String>,
) -> Result<Vec<String>, HubError> {
    let weights = deserialize_index_weight_filenames(bytes)?;
    if weights.is_empty() {
        return Err(HubError::UnsupportedWeightLayout);
    }
    for filename in &weights {
        validate_artifact_path(filename)?;
        if !filename.ends_with(".safetensors") || !available.contains(filename) {
            return Err(HubError::UnsupportedWeightLayout);
        }
    }
    Ok(weights.into_iter().collect())
}

fn deserialize_index_weight_filenames(bytes: &[u8]) -> Result<BTreeSet<String>, HubError> {
    let structural_limit = Cell::new(None);
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result = WeightIndexSeed {
        structural_limit: &structural_limit,
    }
    .deserialize(&mut deserializer);
    let Ok(weights) = result else {
        return Err(match structural_limit.get() {
            Some(limit) => HubError::StructuralLimitExceeded(limit),
            None => HubError::InvalidIndex,
        });
    };
    if deserializer.end().is_err() {
        return Err(HubError::InvalidIndex);
    }
    Ok(weights)
}

struct WeightIndexSeed<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl<'de> DeserializeSeed<'de> for WeightIndexSeed<'_> {
    type Value = BTreeSet<String>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(WeightIndexVisitor {
            structural_limit: self.structural_limit,
        })
    }
}

struct WeightIndexVisitor<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl<'de> Visitor<'de> for WeightIndexVisitor<'_> {
    type Value = BTreeSet<String>;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Safetensors index object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut weight_filenames = None;
        while let Some(field) = map.next_key::<IndexField>()? {
            match field {
                IndexField::WeightMap => {
                    if weight_filenames.is_some() {
                        return Err(de::Error::duplicate_field("weight_map"));
                    }
                    weight_filenames = Some(map.next_value_seed(WeightMapSeed {
                        structural_limit: self.structural_limit,
                    })?);
                }
                IndexField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        weight_filenames.ok_or_else(|| de::Error::missing_field("weight_map"))
    }
}

enum IndexField {
    WeightMap,
    Other,
}

impl<'de> Deserialize<'de> for IndexField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(IndexFieldVisitor)
    }
}

struct IndexFieldVisitor;

impl Visitor<'_> for IndexFieldVisitor {
    type Value = IndexField;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Safetensors index field")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(if value == "weight_map" {
            IndexField::WeightMap
        } else {
            IndexField::Other
        })
    }
}

struct WeightMapSeed<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl<'de> DeserializeSeed<'de> for WeightMapSeed<'_> {
    type Value = BTreeSet<String>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(WeightMapVisitor {
            structural_limit: self.structural_limit,
        })
    }
}

struct WeightMapVisitor<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl<'de> Visitor<'de> for WeightMapVisitor<'_> {
    type Value = BTreeSet<String>;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a tensor-name to Safetensors-shard map")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut entry_count = 0_usize;
        let mut weight_filenames = BTreeSet::new();
        while let Some(()) = map.next_key_seed(TensorNameSeed {
            structural_limit: self.structural_limit,
        })? {
            let Some(next_entry_count) = entry_count.checked_add(1) else {
                return Err(structural_limit_error(
                    self.structural_limit,
                    HubStructuralLimit::WeightIndexEntries,
                ));
            };
            if next_entry_count > MAX_INDEX_WEIGHT_ENTRIES {
                return Err(structural_limit_error(
                    self.structural_limit,
                    HubStructuralLimit::WeightIndexEntries,
                ));
            }
            entry_count = next_entry_count;

            let filename = map.next_value_seed(RepositoryPathSeed {
                structural_limit: self.structural_limit,
            })?;
            weight_filenames.insert(filename);
            if weight_filenames.len() > MAX_SELECTED_WEIGHT_SHARDS {
                return Err(structural_limit_error(
                    self.structural_limit,
                    HubStructuralLimit::SelectedWeightShards,
                ));
            }
        }
        Ok(weight_filenames)
    }
}

struct TensorNameSeed<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl<'de> DeserializeSeed<'de> for TensorNameSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(TensorNameVisitor {
            structural_limit: self.structural_limit,
        })
    }
}

struct TensorNameVisitor<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl Visitor<'_> for TensorNameVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded tensor name")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_INDEX_TENSOR_NAME_BYTES {
            return Err(structural_limit_error(
                self.structural_limit,
                HubStructuralLimit::WeightIndexTensorNameBytes,
            ));
        }
        Ok(())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value.as_str())
    }
}

struct RepositoryPathSeed<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl<'de> DeserializeSeed<'de> for RepositoryPathSeed<'_> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(RepositoryPathVisitor {
            structural_limit: self.structural_limit,
        })
    }
}

struct RepositoryPathVisitor<'a> {
    structural_limit: &'a Cell<Option<HubStructuralLimit>>,
}

impl Visitor<'_> for RepositoryPathVisitor<'_> {
    type Value = String;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded repository-relative shard path")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_REPOSITORY_PATH_BYTES {
            return Err(structural_limit_error(
                self.structural_limit,
                HubStructuralLimit::RepositoryPathBytes,
            ));
        }
        Ok(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_REPOSITORY_PATH_BYTES {
            return Err(structural_limit_error(
                self.structural_limit,
                HubStructuralLimit::RepositoryPathBytes,
            ));
        }
        Ok(value)
    }
}

fn structural_limit_error<E>(
    structural_limit: &Cell<Option<HubStructuralLimit>>,
    limit: HubStructuralLimit,
) -> E
where
    E: de::Error,
{
    structural_limit.set(Some(limit));
    E::custom("Safetensors index structural limit exceeded")
}

pub(crate) fn direct_weights(available: &BTreeSet<String>) -> Result<Vec<String>, HubError> {
    if available.contains(crate::SINGLE_WEIGHT_FILE) {
        return Ok(vec![crate::SINGLE_WEIGHT_FILE.to_owned()]);
    }

    let mut shards = Vec::new();
    for filename in available {
        if let Some((index, total)) = parse_standard_shard(filename) {
            validate_selected_weight_shard_count(total)?;
            let next_shard_count =
                shards
                    .len()
                    .checked_add(1)
                    .ok_or(HubError::StructuralLimitExceeded(
                        HubStructuralLimit::SelectedWeightShards,
                    ))?;
            validate_selected_weight_shard_count(next_shard_count)?;
            shards.push((index, total, filename.clone()));
        }
    }
    let Some(expected_total) = shards.first().map(|(_, total, _)| *total) else {
        return Err(HubError::UnsupportedWeightLayout);
    };
    if expected_total == 0
        || shards.len() != expected_total
        || shards.iter().any(|(_, total, _)| *total != expected_total)
    {
        return Err(HubError::UnsupportedWeightLayout);
    }
    shards.sort_unstable_by_key(|(index, _, _)| *index);
    if shards
        .iter()
        .enumerate()
        .any(|(offset, (index, _, _))| *index != offset + 1)
    {
        return Err(HubError::UnsupportedWeightLayout);
    }
    Ok(shards
        .into_iter()
        .map(|(_, _, filename)| filename)
        .collect())
}

fn parse_standard_shard(filename: &str) -> Option<(usize, usize)> {
    let stem = filename
        .strip_prefix("model-")?
        .strip_suffix(".safetensors")?;
    let (index, total) = stem.split_once("-of-")?;
    if index.len() != 5
        || total.len() != 5
        || !index.bytes().all(|byte| byte.is_ascii_digit())
        || !total.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((index.parse().ok()?, total.parse().ok()?))
}

pub(crate) fn validate_artifact_path(filename: &str) -> Result<(), HubError> {
    validate_maximum_usize(
        filename.len(),
        MAX_REPOSITORY_PATH_BYTES,
        HubStructuralLimit::RepositoryPathBytes,
    )?;
    let path = Path::new(filename);
    if filename.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(HubError::UnsafeArtifactPath(filename.to_owned()));
    }
    Ok(())
}

pub(crate) fn validate_selected_weight_shard_count(shard_count: usize) -> Result<(), HubError> {
    validate_maximum_usize(
        shard_count,
        MAX_SELECTED_WEIGHT_SHARDS,
        HubStructuralLimit::SelectedWeightShards,
    )
}

const fn validate_maximum_usize(
    actual: usize,
    maximum: usize,
    limit: HubStructuralLimit,
) -> Result<(), HubError> {
    if actual > maximum {
        Err(HubError::StructuralLimitExceeded(limit))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::Cursor;

    use super::{
        MAX_INDEX_TENSOR_NAME_BYTES, MAX_INDEX_WEIGHT_ENTRIES, MAX_REPOSITORY_PATH_BYTES,
        MAX_SELECTED_WEIGHT_SHARDS, MAX_WEIGHT_INDEX_BYTES, direct_weights, indexed_weights,
        validate_artifact_path,
    };
    use crate::bounded::{BoundedReadError, read_bounded};
    use crate::{HubError, HubStructuralLimit};

    #[test]
    fn index_deduplicates_and_orders_shards() -> Result<(), HubError> {
        let available = BTreeSet::from([
            "model-00001-of-00002.safetensors".to_owned(),
            "model-00002-of-00002.safetensors".to_owned(),
        ]);
        let index = br#"{
            "metadata": {"total_size": 12},
            "weight_map": {
                "layer.1": "model-00002-of-00002.safetensors",
                "layer.0": "model-00001-of-00002.safetensors",
                "layer.2": "model-00002-of-00002.safetensors"
            }
        }"#;

        assert_eq!(
            indexed_weights(index, &available)?,
            [
                "model-00001-of-00002.safetensors".to_owned(),
                "model-00002-of-00002.safetensors".to_owned(),
            ]
        );
        Ok(())
    }

    #[test]
    fn malformed_index_shapes_are_explicit() {
        let available = BTreeSet::new();
        for index in [
            br"[]".as_slice(),
            br"{}".as_slice(),
            br#"{"weight_map":[]}"#.as_slice(),
            br#"{"weight_map":{"tensor":3}}"#.as_slice(),
            br#"{"weight_map":{},"weight_map":{}}"#.as_slice(),
            br#"{"weight_map":{}"#.as_slice(),
        ] {
            assert!(matches!(
                indexed_weights(index, &available),
                Err(HubError::InvalidIndex)
            ));
        }
    }

    #[test]
    fn direct_layout_rejects_unrelated_and_incomplete_safetensors() {
        let unrelated = BTreeSet::from(["adapter_model.safetensors".to_owned()]);
        assert!(matches!(
            direct_weights(&unrelated),
            Err(HubError::UnsupportedWeightLayout)
        ));

        let incomplete = BTreeSet::from([
            "model-00001-of-00003.safetensors".to_owned(),
            "model-00003-of-00003.safetensors".to_owned(),
        ]);
        assert!(matches!(
            direct_weights(&incomplete),
            Err(HubError::UnsupportedWeightLayout)
        ));
    }

    #[test]
    fn index_limits_are_enforced_during_custom_deserialization() {
        let available = BTreeSet::new();

        let long_name = "x".repeat(MAX_INDEX_TENSOR_NAME_BYTES + 1);
        let index = format!(r#"{{"weight_map":{{"{long_name}":"model.safetensors"}}}}"#);
        assert!(matches!(
            indexed_weights(index.as_bytes(), &available),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::WeightIndexTensorNameBytes
            ))
        ));

        let long_path = "x".repeat(MAX_REPOSITORY_PATH_BYTES + 1);
        let index = format!(r#"{{"weight_map":{{"tensor":"{long_path}"}}}}"#);
        assert!(matches!(
            indexed_weights(index.as_bytes(), &available),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::RepositoryPathBytes
            ))
        ));

        let index = generated_index(MAX_SELECTED_WEIGHT_SHARDS + 1, |entry| {
            format!("model-{entry:05}-of-00257.safetensors")
        });
        assert!(matches!(
            indexed_weights(index.as_bytes(), &available),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::SelectedWeightShards
            ))
        ));

        let index = generated_index(MAX_INDEX_WEIGHT_ENTRIES + 1, |_| {
            "model.safetensors".to_owned()
        });
        assert!(matches!(
            indexed_weights(index.as_bytes(), &available),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::WeightIndexEntries
            ))
        ));
    }

    #[test]
    fn byte_and_direct_shard_limits_are_structural() {
        assert!(matches!(
            read_bounded(
                Cursor::new(Vec::<u8>::new()),
                MAX_WEIGHT_INDEX_BYTES + 1,
                MAX_WEIGHT_INDEX_BYTES,
            ),
            Err(BoundedReadError::Limit)
        ));
        assert!(matches!(
            read_bounded(Cursor::new(vec![0_u8; 2]), 1, 1),
            Err(BoundedReadError::Limit)
        ));

        let total = MAX_SELECTED_WEIGHT_SHARDS + 1;
        let mut shards = BTreeSet::new();
        for index in 1..=total {
            shards.insert(format!("model-{index:05}-of-{total:05}.safetensors"));
        }
        assert!(matches!(
            direct_weights(&shards),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::SelectedWeightShards
            ))
        ));
    }

    #[test]
    fn unsafe_and_oversized_artifact_paths_are_rejected() {
        assert!(validate_artifact_path("../model.safetensors").is_err());
        assert!(validate_artifact_path("/tmp/model.safetensors").is_err());
        assert!(validate_artifact_path("weights/model.safetensors").is_ok());
        let oversized = "x".repeat(MAX_REPOSITORY_PATH_BYTES + 1);
        assert!(matches!(
            validate_artifact_path(oversized.as_str()),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::RepositoryPathBytes
            ))
        ));
    }

    fn generated_index(entry_count: usize, filename: impl Fn(usize) -> String) -> String {
        let mut index = String::from("{\"weight_map\":{");
        for entry in 0..entry_count {
            if entry != 0 {
                index.push(',');
            }
            index.push_str("\"tensor.");
            index.push_str(entry.to_string().as_str());
            index.push_str("\":\"");
            index.push_str(filename(entry).as_str());
            index.push('"');
        }
        index.push_str("}}");
        index
    }
}
