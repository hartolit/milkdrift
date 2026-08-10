use std::fmt::{self, Formatter};

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};

use crate::{ArtifactScalarType, HubError};

/// One MiB accommodates realistic Llama configuration files with ample extension headroom.
pub(crate) const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

struct ModelConfiguration {
    dtype: Option<String>,
    torch_dtype: Option<String>,
}

impl<'de> Deserialize<'de> for ModelConfiguration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ModelConfigurationVisitor)
    }
}

struct ModelConfigurationVisitor;

impl<'de> Visitor<'de> for ModelConfigurationVisitor {
    type Value = ModelConfiguration;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Hugging Face model configuration object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut dtype = None;
        let mut torch_dtype = None;
        while let Some(field) = map.next_key::<ConfigurationField>()? {
            match field {
                ConfigurationField::Dtype => {
                    if dtype.is_some() {
                        return Err(de::Error::duplicate_field("dtype"));
                    }
                    dtype = Some(map.next_value::<Option<String>>()?);
                }
                ConfigurationField::TorchDtype => {
                    if torch_dtype.is_some() {
                        return Err(de::Error::duplicate_field("torch_dtype"));
                    }
                    torch_dtype = Some(map.next_value::<Option<String>>()?);
                }
                ConfigurationField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(ModelConfiguration {
            dtype: dtype.unwrap_or(None),
            torch_dtype: torch_dtype.unwrap_or(None),
        })
    }
}

enum ConfigurationField {
    Dtype,
    TorchDtype,
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

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a model configuration field")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value {
            "dtype" => ConfigurationField::Dtype,
            "torch_dtype" => ConfigurationField::TorchDtype,
            _ => ConfigurationField::Other,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScalarDeclaration {
    Absent,
    Recognized(ArtifactScalarType),
    Unsupported,
}

pub(crate) fn parse_configuration_declared_scalar_type(
    bytes: &[u8],
) -> Result<Option<ArtifactScalarType>, HubError> {
    let configuration: ModelConfiguration =
        serde_json::from_slice(bytes).map_err(|_| HubError::InvalidConfiguration)?;
    let modern = classify_scalar_declaration(configuration.dtype.as_deref());
    let legacy = classify_scalar_declaration(configuration.torch_dtype.as_deref());

    match (modern, legacy) {
        (ScalarDeclaration::Unsupported, _) | (_, ScalarDeclaration::Unsupported) => {
            Err(HubError::UnsupportedScalarDeclaration)
        }
        (ScalarDeclaration::Absent, ScalarDeclaration::Absent) => Ok(None),
        (ScalarDeclaration::Recognized(value), ScalarDeclaration::Absent)
        | (ScalarDeclaration::Absent, ScalarDeclaration::Recognized(value)) => Ok(Some(value)),
        (ScalarDeclaration::Recognized(modern), ScalarDeclaration::Recognized(legacy))
            if modern == legacy =>
        {
            Ok(Some(modern))
        }
        (ScalarDeclaration::Recognized(_), ScalarDeclaration::Recognized(_)) => {
            Err(HubError::ConflictingScalarDeclarations)
        }
    }
}

fn classify_scalar_declaration(value: Option<&str>) -> ScalarDeclaration {
    match value {
        None => ScalarDeclaration::Absent,
        Some(value) => parse_scalar_type(value).map_or(
            ScalarDeclaration::Unsupported,
            ScalarDeclaration::Recognized,
        ),
    }
}

fn parse_scalar_type(value: &str) -> Option<ArtifactScalarType> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("float32") || value.eq_ignore_ascii_case("f32") {
        Some(ArtifactScalarType::F32)
    } else if value.eq_ignore_ascii_case("float16")
        || value.eq_ignore_ascii_case("half")
        || value.eq_ignore_ascii_case("f16")
    {
        Some(ArtifactScalarType::F16)
    } else if value.eq_ignore_ascii_case("bfloat16") || value.eq_ignore_ascii_case("bf16") {
        Some(ArtifactScalarType::Bf16)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{MAX_CONFIG_BYTES, parse_configuration_declared_scalar_type, parse_scalar_type};
    use crate::bounded::{BoundedReadError, read_bounded};
    use crate::{ArtifactScalarType, HubError};

    #[test]
    fn scalar_type_parser_is_explicit() {
        assert_eq!(parse_scalar_type("float32"), Some(ArtifactScalarType::F32));
        assert_eq!(parse_scalar_type("HALF"), Some(ArtifactScalarType::F16));
        assert_eq!(parse_scalar_type(" bf16 "), Some(ArtifactScalarType::Bf16));
        assert_eq!(parse_scalar_type("float8_e4m3fn"), None);
    }

    #[test]
    fn configuration_declaration_matrix_is_strict() -> Result<(), HubError> {
        for (input, expected) in [
            (br"{}".as_slice(), None),
            (br#"{"dtype":null,"torch_dtype":null}"#.as_slice(), None),
            (
                br#"{"dtype":"bfloat16"}"#.as_slice(),
                Some(ArtifactScalarType::Bf16),
            ),
            (
                br#"{"dtype":"float16","torch_dtype":null}"#.as_slice(),
                Some(ArtifactScalarType::F16),
            ),
            (
                br#"{"torch_dtype":"float16"}"#.as_slice(),
                Some(ArtifactScalarType::F16),
            ),
            (
                br#"{"dtype":null,"torch_dtype":"half"}"#.as_slice(),
                Some(ArtifactScalarType::F16),
            ),
            (
                br#"{"dtype":"f32","torch_dtype":"float32"}"#.as_slice(),
                Some(ArtifactScalarType::F32),
            ),
        ] {
            assert_eq!(parse_configuration_declared_scalar_type(input)?, expected);
        }

        for input in [
            br#"{"dtype":"float8_e4m3fn"}"#.as_slice(),
            br#"{"torch_dtype":"float8_e4m3fn"}"#.as_slice(),
            br#"{"dtype":"float8_e4m3fn","torch_dtype":"float16"}"#.as_slice(),
            br#"{"dtype":"float16","torch_dtype":"float8_e4m3fn"}"#.as_slice(),
        ] {
            let error = parse_configuration_declared_scalar_type(input)
                .err()
                .ok_or(HubError::InvalidConfiguration)?;
            assert!(matches!(error, HubError::UnsupportedScalarDeclaration));
            assert!(!error.to_string().contains("float8_e4m3fn"));
        }

        assert!(matches!(
            parse_configuration_declared_scalar_type(
                br#"{"dtype":"bfloat16","torch_dtype":"float16"}"#
            ),
            Err(HubError::ConflictingScalarDeclarations)
        ));
        Ok(())
    }

    #[test]
    fn malformed_configuration_and_declaration_field_types_are_explicit() {
        for input in [
            br#"{"dtype":16}"#.as_slice(),
            br#"{"torch_dtype":false}"#.as_slice(),
            br#"{"dtype":{"name":"float16"}}"#.as_slice(),
            br#"{"dtype":"float32","dtype":"float16"}"#.as_slice(),
            br#"{"dtype":"float32""#.as_slice(),
            br"[]".as_slice(),
        ] {
            assert!(matches!(
                parse_configuration_declared_scalar_type(input),
                Err(HubError::InvalidConfiguration)
            ));
        }
    }

    #[test]
    fn configuration_read_limit_is_enforced_before_or_during_read() {
        assert!(matches!(
            read_bounded(
                Cursor::new(Vec::<u8>::new()),
                MAX_CONFIG_BYTES + 1,
                MAX_CONFIG_BYTES
            ),
            Err(BoundedReadError::Limit)
        ));
        assert!(matches!(
            read_bounded(Cursor::new(vec![0_u8; 2]), 1, 1),
            Err(BoundedReadError::Limit)
        ));
    }
}
