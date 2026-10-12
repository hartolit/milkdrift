use serde::{Deserialize, Serialize};

use crate::ContractError;

/// Counts of ordinary process and model entries performed within one composed invocation.
/// A wrapper's own entry is accounted separately by its frozen capability category.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationCounts {
    process: u64,
    model: u64,
}

impl InvocationCounts {
    /// Independent inclusive counts; zero explicitly forbids that kind of nested work.
    #[must_use]
    pub const fn new(process: u64, model: u64) -> Self {
        Self { process, model }
    }
    /// Process entries, including failed or uncertain entries that consumed admission.
    #[must_use]
    pub const fn process(self) -> u64 {
        self.process
    }
    /// Model entries, including failed or uncertain entries that consumed admission.
    #[must_use]
    pub const fn model(self) -> u64 {
        self.model
    }
    /// Whether both requested counts fit their independent ceilings.
    #[must_use]
    pub const fn contains(self, requested: Self) -> bool {
        requested.process <= self.process && requested.model <= self.model
    }
}

/// Attributable nested work not already charged by publishing the wrapper's public outputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NestedWorkUsage {
    invocations: InvocationCounts,
    artifact_bytes: u64,
}

impl NestedWorkUsage {
    /// Observed internal entries and logical internal artifact bytes. Public output copies are
    /// charged through their own publication transactions and must be excluded here.
    #[must_use]
    pub const fn new(invocations: InvocationCounts, artifact_bytes: u64) -> Self {
        Self {
            invocations,
            artifact_bytes,
        }
    }
    /// Exact attributable nested entries.
    #[must_use]
    pub const fn invocations(self) -> InvocationCounts {
        self.invocations
    }
    /// Logical artifact bytes consumed inside the composition boundary.
    #[must_use]
    pub const fn artifact_bytes(self) -> u64 {
        self.artifact_bytes
    }
}

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

milkdrift_contracts::deserialize_via!(AdmissionMonetaryBound, AdmissionMonetaryBoundWire, |wire| {
    Self::new(wire.maximum_micros, wire.currency)
},);

impl AdmissionMonetaryBound {
    /// Constructs an exact-currency inclusive cost maximum.
    ///
    /// # Errors
    /// Rejects a currency that is not three uppercase ASCII letters.
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

/// Meaning shared by input and output quantities in an admission envelope.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionUnit {
    /// No compatible unit contract, including historical envelopes without a unit.
    #[default]
    Unknown,
    /// Tokens in the complete submitted prompt and generated sequence, including reasoning.
    /// Counts are logical model tokens, not bytes, GPU work, or repeated cache evaluation.
    ModelTokens,
}

impl AdmissionUnit {
    fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}

/// Resource maxima an adapter can enforce for one request before external entry.
///
/// Runtime uses these facts to reserve cumulative workflow resources before committing entry intent.
/// Keep a missing bound [`AdmissionBound::Unknown`]; descriptor estimates and eventual usage
/// observations cannot substitute for a limit the adapter can enforce.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationAdmissionEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nested_invocations: Option<InvocationCounts>,
    #[serde(default, skip_serializing_if = "AdmissionUnit::is_unknown")]
    unit: AdmissionUnit,
    input_units: AdmissionBound<u64>,
    output_units: AdmissionBound<u64>,
    artifact_bytes: AdmissionBound<u64>,
    monetary_cost: AdmissionBound<AdmissionMonetaryBound>,
    execution_duration_ms: AdmissionBound<u64>,
}

impl InvocationAdmissionEnvelope {
    /// Constructs the resource contract with an unknown execution duration.
    /// Use [`Self::with_execution_duration_ms`] only for an enforced attempt deadline.
    #[must_use]
    pub const fn new(
        unit: AdmissionUnit,
        input_units: AdmissionBound<u64>,
        output_units: AdmissionBound<u64>,
        artifact_bytes: AdmissionBound<u64>,
        monetary_cost: AdmissionBound<AdmissionMonetaryBound>,
    ) -> Self {
        Self {
            nested_invocations: None,
            unit,
            input_units,
            output_units,
            artifact_bytes,
            monetary_cost,
            execution_duration_ms: AdmissionBound::Unknown,
        }
    }

    /// Declares the enforced local attempt duration, including bounded cleanup waits.
    /// This does not prove that a remote effect or an unconfirmed process has stopped.
    #[must_use]
    pub const fn with_execution_duration_ms(mut self, bound: AdmissionBound<u64>) -> Self {
        self.execution_duration_ms = bound;
        self
    }

    /// Maximum authorized attempt duration; uncertainty retains its reservation.
    #[must_use]
    pub const fn execution_duration_ms(&self) -> &AdmissionBound<u64> {
        &self.execution_duration_ms
    }

    /// Declare an enforceable bound for nested process/model work in this invocation.
    #[must_use]
    pub const fn with_nested_invocations(mut self, counts: InvocationCounts) -> Self {
        self.nested_invocations = Some(counts);
        self
    }

    /// Nested entry allowance, absent for an ordinary operation without composed work.
    #[must_use]
    pub const fn nested_invocations(&self) -> Option<InvocationCounts> {
        self.nested_invocations
    }

    /// An envelope for an operation that cannot consume any ledger-owned resource.
    #[must_use]
    pub const fn not_applicable() -> Self {
        Self::new(
            AdmissionUnit::Unknown,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        )
        .with_execution_duration_ms(AdmissionBound::NotApplicable)
    }

    /// An envelope that deliberately fails closed for every metered dimension.
    #[must_use]
    pub const fn unknown() -> Self {
        Self::new(
            AdmissionUnit::Unknown,
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
        )
    }

    /// Unit contract for input and output. Unknown units cannot enter a token account.
    #[must_use]
    pub const fn unit(&self) -> AdmissionUnit {
        self.unit
    }

    /// Input maximum in the declared unit.
    #[must_use]
    pub const fn input_units(&self) -> &AdmissionBound<u64> {
        &self.input_units
    }

    /// Output maximum in the declared unit.
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
            AdmissionUnit::ModelTokens,
            AdmissionBound::Bounded(7),
            AdmissionBound::NotApplicable,
            AdmissionBound::Unknown,
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(11, "EUR")?),
        )
        .with_execution_duration_ms(AdmissionBound::Bounded(17));
        let bytes = serde_json::to_vec(&envelope)?;
        let decoded: InvocationAdmissionEnvelope = serde_json::from_slice(&bytes)?;
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.input_units().bounded(), Some(&7));
        assert!(decoded.output_units().bounded().is_none());
        assert!(!decoded.output_units().is_unknown());
        assert!(decoded.artifact_bytes().is_unknown());
        assert_eq!(decoded.execution_duration_ms().bounded(), Some(&17));
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
        let mut historical = serde_json::to_value(envelope)?;
        historical
            .as_object_mut()
            .ok_or("envelope is not an object")?
            .remove("execution_duration_ms");
        assert!(serde_json::from_value::<InvocationAdmissionEnvelope>(historical).is_err());
        Ok(())
    }
}
