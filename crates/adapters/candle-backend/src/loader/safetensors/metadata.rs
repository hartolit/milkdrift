use std::fmt::Formatter;

use serde::Deserializer;
use serde::de::{self, DeserializeSeed, MapAccess, Visitor};

use super::limits::{InspectionBudget, ParseFailure, checked_string, reserve_bounded_slot};

pub(crate) struct MetadataSeed<'a, 'limits> {
    pub(crate) budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for MetadataSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(MetadataVisitor {
            budget: self.budget,
        })
    }
}

struct MetadataVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> Visitor<'de> for MetadataVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded string Safetensors metadata")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut keys: Vec<String> = Vec::new();
        while let Some(key) = map.next_key_seed(MetadataKeySeed {
            budget: self.budget,
        })? {
            reserve_bounded_slot(&mut keys, self.budget.limits.metadata_entries).map_err(|()| {
                self.budget.fail(
                    ParseFailure::Allocation,
                    "metadata key inventory allocation failed",
                )
            })?;
            keys.push(key);
            map.next_value_seed(MetadataValueSeed {
                budget: self.budget,
            })?;
        }
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair.first() == pair.get(1)) {
            return Err(self.budget.fail(
                ParseFailure::DuplicateTensor,
                "duplicate Safetensors metadata key",
            ));
        }
        Ok(())
    }
}

struct MetadataKeySeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for MetadataKeySeed<'_, '_> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(MetadataKeyVisitor {
            budget: self.budget,
        })
    }
}

struct MetadataKeyVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl MetadataKeyVisitor<'_, '_> {
    fn retain<E: de::Error>(&mut self, value: &str) -> Result<String, E> {
        self.budget.add_metadata_string(
            value.len(),
            self.budget.limits.metadata_key_bytes,
            true,
        )?;
        checked_string(value).map_err(|()| {
            self.budget
                .fail(ParseFailure::Allocation, "metadata key allocation failed")
        })
    }
}

impl<'de> Visitor<'de> for MetadataKeyVisitor<'_, '_> {
    type Value = String;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors metadata key")
    }

    fn visit_str<E>(mut self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.retain(value)
    }

    fn visit_borrowed_str<E>(mut self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.retain(value)
    }
}

struct MetadataValueSeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for MetadataValueSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(MetadataValueVisitor {
            budget: self.budget,
        })
    }
}

struct MetadataValueVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl Visitor<'_> for MetadataValueVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors metadata value")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.budget
            .add_metadata_string(value.len(), self.budget.limits.metadata_value_bytes, false)
    }
}
