//! Exact bounded configuration reading, declaration policy, and Llama identity.

use std::fmt::Formatter;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use candle_transformers::models::llama::{Config, LlamaConfig};
use domain_contracts::{BackendFailureKind, BackendId, LoadError, LoadFailureStage, ScalarType};
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::failure::{
    CODE_ARCHITECTURE, CODE_CONFIG_ALLOCATION, CODE_CONFIG_DECODE, CODE_CONFIG_LIMIT,
    CODE_CONFIG_READ, CODE_DECLARATION_CONFLICT, CODE_DECLARATION_MALFORMED,
    CODE_DECLARATION_UNSUPPORTED, CODE_NUMERIC_OVERFLOW, load_failure,
};

use super::configuration_policy::validate_numeric_config;
use super::{host_memory_failure, invalid_model_failure};

/// One MiB accommodates realistic Llama configuration files while strictly
/// bounding retained bytes and any JSON parser work.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const CONFIG_READ_BUFFER_BYTES: usize = 64 * 1024;
const CONFIG_READ_BUFFER_BYTES_U64: u64 = 64 * 1024;

#[cfg(test)]
thread_local! {
    static TEST_CONFIG_ALLOCATION_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug)]
pub(super) struct ParsedConfig {
    pub(super) config: Config,
    pub(super) declaration: Option<ScalarType>,
}

pub(super) fn read_and_parse(backend: BackendId, path: &Path) -> Result<ParsedConfig, LoadError> {
    let mut file =
        File::open(path).map_err(|_| invalid_model_failure(backend, CODE_CONFIG_READ))?;
    let bytes = read_bounded(backend, &mut file)?;
    parse_bytes(backend, bytes.as_slice())
}

fn read_bounded<R: Read>(backend: BackendId, reader: &mut R) -> Result<Vec<u8>, LoadError> {
    let maximum_with_sentinel = MAX_CONFIG_BYTES
        .checked_add(1)
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    let mut bytes = Vec::new();
    let mut buffer = Vec::new();
    try_reserve_config(&mut buffer, CONFIG_READ_BUFFER_BYTES)
        .map_err(|()| host_memory_failure(backend, CODE_CONFIG_ALLOCATION))?;
    buffer.resize(CONFIG_READ_BUFFER_BYTES, 0);

    loop {
        let retained = u64::try_from(bytes.len())
            .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        if retained == maximum_with_sentinel {
            return Err(invalid_model_failure(backend, CODE_CONFIG_LIMIT));
        }
        let remaining = maximum_with_sentinel
            .checked_sub(retained)
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        let request = usize::try_from(remaining.min(CONFIG_READ_BUFFER_BYTES_U64))
            .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        let destination = buffer
            .get_mut(..request)
            .ok_or_else(|| invalid_model_failure(backend, CODE_CONFIG_READ))?;
        let read = reader
            .read(destination)
            .map_err(|_| invalid_model_failure(backend, CODE_CONFIG_READ))?;
        if read == 0 {
            break;
        }
        try_reserve_config(&mut bytes, read)
            .map_err(|()| host_memory_failure(backend, CODE_CONFIG_ALLOCATION))?;
        bytes.extend_from_slice(
            destination
                .get(..read)
                .ok_or_else(|| invalid_model_failure(backend, CODE_CONFIG_READ))?,
        );
    }

    if u64::try_from(bytes.len())
        .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?
        > MAX_CONFIG_BYTES
    {
        return Err(invalid_model_failure(backend, CODE_CONFIG_LIMIT));
    }
    Ok(bytes)
}

fn try_reserve_config(bytes: &mut Vec<u8>, additional: usize) -> Result<(), ()> {
    #[cfg(test)]
    if TEST_CONFIG_ALLOCATION_FAILURES.with(|remaining| {
        let value = remaining.get();
        if value == 0 {
            false
        } else {
            remaining.set(value - 1);
            true
        }
    }) {
        return Err(());
    }
    bytes.try_reserve_exact(additional).map_err(|_| ())
}

pub(super) fn parse_bytes(backend: BackendId, bytes: &[u8]) -> Result<ParsedConfig, LoadError> {
    let byte_length = u64::try_from(bytes.len())
        .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    if byte_length > MAX_CONFIG_BYTES {
        return Err(invalid_model_failure(backend, CODE_CONFIG_LIMIT));
    }

    let facts = parse_facts(bytes)
        .map_err(|_| invalid_model_failure(backend, CODE_DECLARATION_MALFORMED))?;
    let declaration = facts.resolve_declaration(backend)?;
    facts.validate_architecture(backend)?;

    // Candle and the adapter-specific fact parser consume the same retained
    // byte slice. No caller-injected declaration can diverge from this config.
    let hugging_face: LlamaConfig = serde_json::from_slice(bytes)
        .map_err(|_| invalid_model_failure(backend, CODE_CONFIG_DECODE))?;
    let config = hugging_face.into_config(false);
    validate_numeric_config(backend, &config)?;
    Ok(ParsedConfig {
        config,
        declaration,
    })
}

fn parse_facts(bytes: &[u8]) -> Result<ConfigurationFacts, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let facts = ConfigurationFacts::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(facts)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScalarDeclaration {
    Absent,
    Null,
    Recognized(ScalarType),
    Unsupported,
    Malformed,
}

impl ScalarDeclaration {
    const fn is_absent(self) -> bool {
        matches!(self, Self::Absent | Self::Null)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelTypeFact {
    Absent,
    Null,
    Llama,
    Other,
    Malformed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchitecturesFact {
    Absent,
    Null,
    Llama,
    Contradictory,
    Malformed,
}

#[derive(Debug)]
struct ConfigurationFacts {
    dtype: ScalarDeclaration,
    torch_dtype: ScalarDeclaration,
    model_type: ModelTypeFact,
    architectures: ArchitecturesFact,
}

impl ConfigurationFacts {
    fn resolve_declaration(&self, backend: BackendId) -> Result<Option<ScalarType>, LoadError> {
        if matches!(self.dtype, ScalarDeclaration::Malformed)
            || matches!(self.torch_dtype, ScalarDeclaration::Malformed)
        {
            return Err(invalid_model_failure(backend, CODE_DECLARATION_MALFORMED));
        }
        if matches!(self.dtype, ScalarDeclaration::Unsupported)
            || matches!(self.torch_dtype, ScalarDeclaration::Unsupported)
        {
            return Err(load_failure(
                backend,
                BackendFailureKind::Unsupported,
                CODE_DECLARATION_UNSUPPORTED,
                LoadFailureStage::CompatibilityValidation,
            ));
        }

        match (self.dtype, self.torch_dtype) {
            (modern, legacy) if modern.is_absent() && legacy.is_absent() => Ok(None),
            (ScalarDeclaration::Recognized(value), legacy) if legacy.is_absent() => Ok(Some(value)),
            (modern, ScalarDeclaration::Recognized(value)) if modern.is_absent() => Ok(Some(value)),
            (ScalarDeclaration::Recognized(modern), ScalarDeclaration::Recognized(legacy))
                if modern == legacy =>
            {
                Ok(Some(modern))
            }
            (ScalarDeclaration::Recognized(_), ScalarDeclaration::Recognized(_)) => {
                Err(load_failure(
                    backend,
                    BackendFailureKind::Unsupported,
                    CODE_DECLARATION_CONFLICT,
                    LoadFailureStage::CompatibilityValidation,
                ))
            }
            _ => Err(invalid_model_failure(backend, CODE_DECLARATION_MALFORMED)),
        }
    }

    fn validate_architecture(&self, backend: BackendId) -> Result<(), LoadError> {
        let model_is_llama = matches!(self.model_type, ModelTypeFact::Llama);
        let architectures_are_llama = matches!(
            self.architectures,
            ArchitecturesFact::Absent | ArchitecturesFact::Null | ArchitecturesFact::Llama
        );
        if model_is_llama && architectures_are_llama {
            Ok(())
        } else {
            Err(load_failure(
                backend,
                BackendFailureKind::Unsupported,
                CODE_ARCHITECTURE,
                LoadFailureStage::CompatibilityValidation,
            ))
        }
    }
}

impl<'de> Deserialize<'de> for ConfigurationFacts {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ConfigurationFactsVisitor)
    }
}

struct ConfigurationFactsVisitor;

impl<'de> Visitor<'de> for ConfigurationFactsVisitor {
    type Value = ConfigurationFacts;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a Hugging Face model configuration object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut dtype = ScalarDeclaration::Absent;
        let mut torch_dtype = ScalarDeclaration::Absent;
        let mut model_type = ModelTypeFact::Absent;
        let mut architectures = ArchitecturesFact::Absent;
        let mut seen_dtype = false;
        let mut seen_torch_dtype = false;
        let mut seen_model_type = false;
        let mut seen_architectures = false;

        while let Some(field) = map.next_key::<ConfigurationField>()? {
            match field {
                ConfigurationField::Dtype => {
                    let value = map.next_value_seed(ScalarDeclarationSeed)?;
                    if seen_dtype {
                        dtype = ScalarDeclaration::Malformed;
                    } else {
                        seen_dtype = true;
                        dtype = value;
                    }
                }
                ConfigurationField::TorchDtype => {
                    let value = map.next_value_seed(ScalarDeclarationSeed)?;
                    if seen_torch_dtype {
                        torch_dtype = ScalarDeclaration::Malformed;
                    } else {
                        seen_torch_dtype = true;
                        torch_dtype = value;
                    }
                }
                ConfigurationField::ModelType => {
                    let value = map.next_value_seed(ModelTypeSeed)?;
                    if seen_model_type {
                        model_type = ModelTypeFact::Malformed;
                    } else {
                        seen_model_type = true;
                        model_type = value;
                    }
                }
                ConfigurationField::Architectures => {
                    let value = map.next_value_seed(ArchitecturesSeed)?;
                    if seen_architectures {
                        architectures = ArchitecturesFact::Malformed;
                    } else {
                        seen_architectures = true;
                        architectures = value;
                    }
                }
                ConfigurationField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        Ok(ConfigurationFacts {
            dtype,
            torch_dtype,
            model_type,
            architectures,
        })
    }
}

#[derive(Clone, Copy)]
enum ConfigurationField {
    Dtype,
    TorchDtype,
    ModelType,
    Architectures,
    Other,
}

impl<'de> Deserialize<'de> for ConfigurationField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(ConfigurationFieldVisitor)
    }
}

struct ConfigurationFieldVisitor;

impl Visitor<'_> for ConfigurationFieldVisitor {
    type Value = ConfigurationField;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a model configuration field")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value {
            "dtype" => ConfigurationField::Dtype,
            "torch_dtype" => ConfigurationField::TorchDtype,
            "model_type" => ConfigurationField::ModelType,
            "architectures" => ConfigurationField::Architectures,
            _ => ConfigurationField::Other,
        })
    }
}

struct ScalarDeclarationSeed;

impl<'de> de::DeserializeSeed<'de> for ScalarDeclarationSeed {
    type Value = ScalarDeclaration;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ScalarDeclarationVisitor)
    }
}

struct ScalarDeclarationVisitor;

impl<'de> Visitor<'de> for ScalarDeclarationVisitor {
    type Value = ScalarDeclaration;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a null or recognized scalar declaration string")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ScalarDeclaration::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ScalarDeclaration::Null)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(parse_scalar(value).map_or(
            ScalarDeclaration::Unsupported,
            ScalarDeclaration::Recognized,
        ))
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ScalarDeclaration::Malformed)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ScalarDeclaration::Malformed)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ScalarDeclaration::Malformed)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ScalarDeclaration::Malformed)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(ScalarDeclaration::Malformed)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(ScalarDeclaration::Malformed)
    }
}

fn parse_scalar(value: &str) -> Option<ScalarType> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("float32") || value.eq_ignore_ascii_case("f32") {
        Some(ScalarType::F32)
    } else if value.eq_ignore_ascii_case("float16")
        || value.eq_ignore_ascii_case("half")
        || value.eq_ignore_ascii_case("f16")
    {
        Some(ScalarType::F16)
    } else if value.eq_ignore_ascii_case("bfloat16") || value.eq_ignore_ascii_case("bf16") {
        Some(ScalarType::Bf16)
    } else {
        None
    }
}

struct ModelTypeSeed;

impl<'de> de::DeserializeSeed<'de> for ModelTypeSeed {
    type Value = ModelTypeFact;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ModelTypeVisitor)
    }
}

struct ModelTypeVisitor;

impl<'de> Visitor<'de> for ModelTypeVisitor {
    type Value = ModelTypeFact;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a Llama model_type string")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ModelTypeFact::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ModelTypeFact::Null)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.eq_ignore_ascii_case("llama") {
            Ok(ModelTypeFact::Llama)
        } else {
            Ok(ModelTypeFact::Other)
        }
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ModelTypeFact::Malformed)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ModelTypeFact::Malformed)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ModelTypeFact::Malformed)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ModelTypeFact::Malformed)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(ModelTypeFact::Malformed)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(ModelTypeFact::Malformed)
    }
}

struct ArchitecturesSeed;

impl<'de> de::DeserializeSeed<'de> for ArchitecturesSeed {
    type Value = ArchitecturesFact;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ArchitecturesVisitor)
    }
}

struct ArchitecturesVisitor;

impl<'de> Visitor<'de> for ArchitecturesVisitor {
    type Value = ArchitecturesFact;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("null or an array of recognized Llama architectures")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut count = 0_usize;
        let mut result = ArchitecturesFact::Llama;
        while let Some(entry) = sequence.next_element_seed(ArchitectureNameSeed)? {
            count = count.saturating_add(1);
            match entry {
                ArchitectureNameFact::Other if !matches!(result, ArchitecturesFact::Malformed) => {
                    result = ArchitecturesFact::Contradictory;
                }
                ArchitectureNameFact::Llama | ArchitectureNameFact::Other => {}
                ArchitectureNameFact::Malformed => result = ArchitecturesFact::Malformed,
            }
        }
        if count == 0 {
            Ok(ArchitecturesFact::Contradictory)
        } else {
            Ok(result)
        }
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Malformed)
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Malformed)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Malformed)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Malformed)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitecturesFact::Malformed)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(ArchitecturesFact::Malformed)
    }
}

#[derive(Clone, Copy)]
enum ArchitectureNameFact {
    Llama,
    Other,
    Malformed,
}

struct ArchitectureNameSeed;

impl<'de> de::DeserializeSeed<'de> for ArchitectureNameSeed {
    type Value = ArchitectureNameFact;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ArchitectureNameVisitor)
    }
}

struct ArchitectureNameVisitor;

impl<'de> Visitor<'de> for ArchitectureNameVisitor {
    type Value = ArchitectureNameFact;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a recognized Llama architecture string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if matches!(value, "LlamaForCausalLM" | "LlamaModel") {
            Ok(ArchitectureNameFact::Llama)
        } else {
            Ok(ArchitectureNameFact::Other)
        }
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitectureNameFact::Malformed)
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitectureNameFact::Malformed)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitectureNameFact::Malformed)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitectureNameFact::Malformed)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ArchitectureNameFact::Malformed)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(ArchitectureNameFact::Malformed)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(ArchitectureNameFact::Malformed)
    }
}

#[cfg(test)]
mod tests;
