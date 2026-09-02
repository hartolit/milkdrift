use serde::{Deserialize, Deserializer, Serialize};

use crate::ContractError;

/// One request-specific resource fact used at the last enforceable entry boundary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "maximum",
    deny_unknown_fields
)]
pub enum AdmissionBound<T> {
    /// The host enforces this inclusive maximum for the exact immutable request.
    Bounded(T),
    /// The exact operation cannot consume this resource dimension.
    NotApplicable,
    /// No enforceable pre-entry maximum is available.
    Unknown,
}

impl<T> AdmissionBound<T> {
    /// Returns the enforceable maximum when one exists.
    #[must_use]
    pub const fn bounded(&self) -> Option<&T> {
        match self {
            Self::Bounded(value) => Some(value),
            Self::NotApplicable | Self::Unknown => None,
        }
    }

    /// Whether the dimension lacks an enforceable pre-entry fact.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Enforceable monetary maximum in one exact currency.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionMonetaryBound {
    maximum_micros: u64,
    currency: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionMonetaryBoundWire {
    maximum_micros: u64,
    currency: String,
}

impl<'de> Deserialize<'de> for AdmissionMonetaryBound {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AdmissionMonetaryBoundWire::deserialize(deserializer)?;
        Self::new(wire.maximum_micros, wire.currency).map_err(serde::de::Error::custom)
    }
}

impl AdmissionMonetaryBound {
    /// Constructs an exact-currency inclusive cost maximum.
    pub fn new(maximum_micros: u64, currency: impl Into<String>) -> Result<Self, ContractError> {
        let currency = currency.into();
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ContractError::InvalidContract(
                "admission currency must be a three-letter uppercase code".to_owned(),
            ));
        }
        Ok(Self {
            maximum_micros,
            currency,
        })
    }

    /// Inclusive maximum in millionths of the named currency.
    #[must_use]
    pub const fn maximum_micros(&self) -> u64 {
        self.maximum_micros
    }

    /// Exact admission currency; no conversion is implied.
    #[must_use]
    pub fn currency(&self) -> &str {
        &self.currency
    }
}

/// Exact immutable-request admission envelope returned before adapter entry.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationAdmissionEnvelope {
    input_units: AdmissionBound<u64>,
    output_units: AdmissionBound<u64>,
    artifact_bytes: AdmissionBound<u64>,
    monetary_cost: AdmissionBound<AdmissionMonetaryBound>,
}

impl InvocationAdmissionEnvelope {
    /// Constructs the complete four-dimensional admission contract.
    #[must_use]
    pub const fn new(
        input_units: AdmissionBound<u64>,
        output_units: AdmissionBound<u64>,
        artifact_bytes: AdmissionBound<u64>,
        monetary_cost: AdmissionBound<AdmissionMonetaryBound>,
    ) -> Self {
        Self {
            input_units,
            output_units,
            artifact_bytes,
            monetary_cost,
        }
    }

    /// An envelope for an operation that cannot consume any ledger-owned resource.
    #[must_use]
    pub const fn not_applicable() -> Self {
        Self::new(
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        )
    }

    /// An envelope that deliberately fails closed for every metered dimension.
    #[must_use]
    pub const fn unknown() -> Self {
        Self::new(
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
        )
    }

    /// Provider-defined input-unit maximum.
    #[must_use]
    pub const fn input_units(&self) -> &AdmissionBound<u64> {
        &self.input_units
    }

    /// Provider-defined output-unit maximum.
    #[must_use]
    pub const fn output_units(&self) -> &AdmissionBound<u64> {
        &self.output_units
    }

    /// Logical artifact-publication maximum.
    #[must_use]
    pub const fn artifact_bytes(&self) -> &AdmissionBound<u64> {
        &self.artifact_bytes
    }

    /// Exact-currency monetary maximum.
    #[must_use]
    pub const fn monetary_cost(&self) -> &AdmissionBound<AdmissionMonetaryBound> {
        &self.monetary_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monetary_currency_is_exact_and_bounded() -> Result<(), ContractError> {
        assert!(AdmissionMonetaryBound::new(1, "usd").is_err());
        let bound = AdmissionMonetaryBound::new(0, "USD")?;
        assert_eq!(bound.currency(), "USD");
        assert_eq!(bound.maximum_micros(), 0);
        Ok(())
    }

    #[test]
    fn monetary_currency_cannot_bypass_validation_during_decode() {
        assert!(
            serde_json::from_value::<AdmissionMonetaryBound>(serde_json::json!({
                "maximum_micros": 1,
                "currency": "usd"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<AdmissionMonetaryBound>(serde_json::json!({
                "maximum_micros": 1,
                "currency": "USD",
                "ignored": true
            }))
            .is_err()
        );
    }

    #[test]
    fn common_envelope_contract_preserves_every_explicit_bound_kind()
    -> Result<(), Box<dyn std::error::Error>> {
        let envelope = InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(7),
            AdmissionBound::NotApplicable,
            AdmissionBound::Unknown,
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(11, "EUR")?),
        );
        let bytes = serde_json::to_vec(&envelope)?;
        let decoded: InvocationAdmissionEnvelope = serde_json::from_slice(&bytes)?;
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.input_units().bounded(), Some(&7));
        assert!(decoded.output_units().bounded().is_none());
        assert!(!decoded.output_units().is_unknown());
        assert!(decoded.artifact_bytes().is_unknown());
        assert_eq!(
            decoded
                .monetary_cost()
                .bounded()
                .map(AdmissionMonetaryBound::currency),
            Some("EUR")
        );
        assert_ne!(
            InvocationAdmissionEnvelope::not_applicable(),
            InvocationAdmissionEnvelope::unknown()
        );
        Ok(())
    }
}
