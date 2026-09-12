//! Redacted presentation of the normalized configuration.
use super::{ConfigError, DaemonConfig};

pub(super) fn redacted_value(config: &DaemonConfig) -> Result<serde_json::Value, ConfigError> {
    let mut value =
        serde_json::to_value(config).map_err(|error| ConfigError::Invalid(error.to_string()))?;
    let sources = value
        .as_object_mut()
        .ok_or_else(|| ConfigError::Invalid("configuration root is not an object".to_owned()))?;
    sources.insert(
        "budget_scopes".to_owned(),
        serde_json::json!({
            "actors_authority_budget": "per-command/per-request permission ceiling; not lifetime consumption",
            "adapter_profiles": "per-attempt enforceable limits; provider estimates and missing metering are separate",
            "runtime_concurrency_and_queues": "worker capacity; not cumulative usage",
            "controller_accounting": "immutable cumulative account: committed = settled + outstanding reservations; remaining is unknown when blocked",
            "application_receipts_and_audits": "retained storage bounds; cold exact-replay history can grow"
        }),
    );
    sources.insert(
        "secret_sources".to_owned(),
        serde_json::json!({
            "configured_references": config.secret_sources.len(),
            "values": "[redacted]",
        }),
    );
    Ok(value)
}

pub(super) fn redacted_toml(redacted: &serde_json::Value) -> Result<String, ConfigError> {
    let value = json_to_toml(redacted)?
        .ok_or_else(|| ConfigError::Toml("effective configuration cannot be null".to_owned()))?;
    toml::to_string_pretty(&value).map_err(|error| ConfigError::Toml(error.to_string()))
}

fn json_to_toml(value: &serde_json::Value) -> Result<Option<toml::Value>, ConfigError> {
    let converted = match value {
        serde_json::Value::Null => return Ok(None),
        serde_json::Value::Bool(value) => toml::Value::Boolean(*value),
        serde_json::Value::String(value) => toml::Value::String(value.clone()),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                toml::Value::Integer(value)
            } else if let Some(value) = value.as_u64() {
                match i64::try_from(value) {
                    Ok(value) => toml::Value::Integer(value),
                    Err(_) => toml::Value::String(value.to_string()),
                }
            } else if let Some(value) = value.as_f64() {
                toml::Value::Float(value)
            } else {
                return Err(ConfigError::Toml(
                    "effective configuration contains an unsupported number".to_owned(),
                ));
            }
        }
        serde_json::Value::Array(values) => toml::Value::Array(
            values
                .iter()
                .map(|value| {
                    json_to_toml(value)?.ok_or_else(|| {
                        ConfigError::Toml(
                            "effective configuration contains null inside an array".to_owned(),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        serde_json::Value::Object(values) => {
            let mut table = toml::Table::new();
            for (key, value) in values {
                if let Some(value) = json_to_toml(value)? {
                    table.insert(key.clone(), value);
                }
            }
            toml::Value::Table(table)
        }
    };
    Ok(Some(converted))
}
