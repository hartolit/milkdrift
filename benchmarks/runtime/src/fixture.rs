//! Verification and construction of the committed deterministic Candle fixture.

use std::fs;
use std::path::{Path, PathBuf};

use candle_backend::CandleLlamaSource;
use domain_contracts::ScalarType;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{BenchmarkError, BenchmarkResult};

pub(crate) const CONFIG_SHA256: &str =
    "052b5c325859dc723ed0825f711950cbff112a140239953273cebacdb36afdd0";
pub(crate) const WEIGHTS_SHA256: &str =
    "cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672";
pub(crate) const CONFIG_BYTES: u64 = 360;
pub(crate) const WEIGHTS_BYTES: u64 = 4_800;
pub(crate) const VOCABULARY_SIZE: u32 = 16;
pub(crate) const CONTEXT_CAPACITY: u32 = 16;
pub(crate) const RELATIVE_DIRECTORY: &str =
    "../../crates/runtime/inference-runtime/tests/fixtures/candle-llama";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FixtureIdentity {
    pub(crate) directory: &'static str,
    pub(crate) provenance: &'static str,
    pub(crate) verification: &'static str,
    pub(crate) config: FixtureFileIdentity,
    pub(crate) weights: FixtureFileIdentity,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FixtureFileIdentity {
    pub(crate) file: &'static str,
    pub(crate) bytes: u64,
    pub(crate) reviewed_sha256: &'static str,
}

pub(crate) struct VerifiedFixture {
    directory: PathBuf,
    pub(crate) identity: FixtureIdentity,
}

#[derive(Deserialize)]
struct FixtureConfiguration {
    model_type: String,
    vocab_size: u32,
    hidden_size: u32,
    intermediate_size: u32,
    num_hidden_layers: u32,
    num_attention_heads: u32,
    num_key_value_heads: u32,
    max_position_embeddings: u32,
}

impl VerifiedFixture {
    pub(crate) fn verify() -> BenchmarkResult<Self> {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(RELATIVE_DIRECTORY);
        verify_file(
            &directory.join("config.json"),
            CONFIG_BYTES,
            CONFIG_SHA256,
            "configuration",
        )?;
        verify_file(
            &directory.join("model.safetensors"),
            WEIGHTS_BYTES,
            WEIGHTS_SHA256,
            "Safetensors weights",
        )?;
        verify_configuration(&directory.join("config.json"))?;

        Ok(Self {
            directory,
            identity: FixtureIdentity {
                directory: RELATIVE_DIRECTORY,
                provenance: "project-authored deterministic synthetic Candle integration fixture",
                verification: "presence, exact byte sizes, recomputed SHA-256, parsed configuration, and loaded public descriptor",
                config: FixtureFileIdentity {
                    file: "config.json",
                    bytes: CONFIG_BYTES,
                    reviewed_sha256: CONFIG_SHA256,
                },
                weights: FixtureFileIdentity {
                    file: "model.safetensors",
                    bytes: WEIGHTS_BYTES,
                    reviewed_sha256: WEIGHTS_SHA256,
                },
            },
        })
    }

    pub(crate) fn source(&self) -> BenchmarkResult<CandleLlamaSource> {
        CandleLlamaSource::new(
            self.directory.join("config.json"),
            vec![self.directory.join("model.safetensors")],
            Some(ScalarType::F32),
        )
        .map_err(|error| BenchmarkError::new(format!("fixture source is invalid: {error}")))
    }
}

fn verify_file(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    label: &str,
) -> BenchmarkResult {
    let metadata = fs::metadata(path).map_err(|error| {
        BenchmarkError::new(format!(
            "required fixture {label} {} is unavailable: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(BenchmarkError::new(format!(
            "required fixture {label} {} is not a regular file",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        BenchmarkError::new(format!(
            "required fixture {label} {} could not be read: {error}",
            path.display()
        ))
    })?;
    let observed_bytes = u64::try_from(bytes.len())
        .map_err(|_| BenchmarkError::new(format!("fixture {label} byte length overflowed u64")))?;
    if observed_bytes != expected_bytes {
        return Err(BenchmarkError::new(format!(
            "fixture {label} {} has {observed_bytes} bytes; reviewed identity requires {expected_bytes}",
            path.display()
        )));
    }
    let observed_sha256 = sha256_hex(&bytes);
    if observed_sha256 != expected_sha256 {
        return Err(BenchmarkError::new(format!(
            "fixture {label} {} has SHA-256 {observed_sha256}; reviewed identity requires {expected_sha256}",
            path.display()
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        encoded.push(hex_digit(byte >> 4));
        encoded.push(hex_digit(byte & 0x0f));
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + (value - 10)),
        _ => '?',
    }
}

fn verify_configuration(path: &Path) -> BenchmarkResult {
    let bytes = fs::read(path).map_err(|error| {
        BenchmarkError::new(format!(
            "fixture configuration {} could not be read: {error}",
            path.display()
        ))
    })?;
    let configuration =
        serde_json::from_slice::<FixtureConfiguration>(&bytes).map_err(|error| {
            BenchmarkError::new(format!(
                "fixture configuration {} is invalid JSON: {error}",
                path.display()
            ))
        })?;
    let expected = ("llama", VOCABULARY_SIZE, 8, 16, 1, 2, 2, CONTEXT_CAPACITY);
    let observed = (
        configuration.model_type.as_str(),
        configuration.vocab_size,
        configuration.hidden_size,
        configuration.intermediate_size,
        configuration.num_hidden_layers,
        configuration.num_attention_heads,
        configuration.num_key_value_heads,
        configuration.max_position_embeddings,
    );
    if observed != expected {
        return Err(BenchmarkError::new(format!(
            "fixture configuration identity mismatch: observed {observed:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FixtureConfiguration, VerifiedFixture, sha256_hex, verify_configuration};
    use std::path::PathBuf;

    #[test]
    fn fixture_configuration_parser_accepts_committed_identity() -> Result<(), String> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/config.json");
        verify_configuration(&path).map_err(|error| error.to_string())
    }

    #[test]
    fn committed_fixture_hashes_are_recomputed() -> Result<(), String> {
        VerifiedFixture::verify()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn sha256_encoding_matches_the_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn configuration_shape_is_deserializable_without_unknown_contracts() -> Result<(), String> {
        let input = br#"{
            "model_type":"llama",
            "vocab_size":16,
            "hidden_size":8,
            "intermediate_size":16,
            "num_hidden_layers":1,
            "num_attention_heads":2,
            "num_key_value_heads":2,
            "max_position_embeddings":16
        }"#;
        let parsed = serde_json::from_slice::<FixtureConfiguration>(input)
            .map_err(|error| error.to_string())?;
        assert_eq!(parsed.vocab_size, 16);
        assert_eq!(parsed.max_position_embeddings, 16);
        Ok(())
    }
}
