//! Measured usage is distinct from the pre-entry reservation envelope.
use crate::{BoundedJson, ContractError, ExtensionKey, bounded::validate_extensions};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Adapter-reported resource and usage measurements.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageObservation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nested_work: Option<crate::NestedWorkUsage>,
    /// Provider-defined input units when observed.
    input_units: Option<u64>,
    /// Provider-defined output units when observed.
    output_units: Option<u64>,
    /// Total wall duration observed by the adapter.
    duration_ms: Option<u64>,
    /// Observed monetary cost in millionths when supplied.
    cost_micros: Option<u64>,
    /// Currency for `cost_micros`.
    currency: Option<String>,
    /// Namespaced bounded provider-specific observations.
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageObservationWire {
    #[serde(default)]
    nested_work: Option<crate::NestedWorkUsage>,
    input_units: Option<u64>,
    output_units: Option<u64>,
    duration_ms: Option<u64>,
    cost_micros: Option<u64>,
    currency: Option<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

milkdrift_contracts::deserialize_via!(UsageObservation, UsageObservationWire, |wire| {
    Self::new(
        wire.input_units,
        wire.output_units,
        wire.duration_ms,
        wire.cost_micros,
        wire.currency,
        wire.extensions,
    )
    .map(|mut value| {
        value.nested_work = wire.nested_work;
        value
    })
});

impl UsageObservation {
    /// Constructs validated adapter-reported usage measurements.
    pub fn new(
        input_units: Option<u64>,
        output_units: Option<u64>,
        duration_ms: Option<u64>,
        cost_micros: Option<u64>,
        currency: Option<String>,
        extensions: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ContractError> {
        let usage = Self {
            nested_work: None,
            input_units,
            output_units,
            duration_ms,
            cost_micros,
            currency,
            extensions,
        };
        usage.validate()?;
        Ok(usage)
    }

    /// Returns observed input units.
    #[must_use]
    pub const fn input_units(&self) -> Option<u64> {
        self.input_units
    }

    /// Returns observed output units.
    #[must_use]
    pub const fn output_units(&self) -> Option<u64> {
        self.output_units
    }

    /// Returns observed wall duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Returns observed cost in millionths.
    #[must_use]
    pub const fn cost_micros(&self) -> Option<u64> {
        self.cost_micros
    }

    /// Returns the currency associated with observed cost.
    #[must_use]
    pub fn currency(&self) -> Option<&str> {
        self.currency.as_deref()
    }

    /// Attach exact internal counts and artifact bytes for a composed invocation.
    #[must_use]
    pub const fn with_nested_work(mut self, usage: crate::NestedWorkUsage) -> Self {
        self.nested_work = Some(usage);
        self
    }

    /// Nested work observation; absence remains unknown when the admission reserved nested work.
    #[must_use]
    pub const fn nested_work(&self) -> Option<crate::NestedWorkUsage> {
        self.nested_work
    }

    /// Returns bounded namespaced provider-specific observations.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
    }

    pub(super) fn validate(&self) -> Result<(), ContractError> {
        if self.cost_micros.is_some() != self.currency.is_some() {
            return Err(ContractError::InvalidContract(
                "usage cost and currency must be supplied together".to_owned(),
            ));
        }
        if let Some(currency) = &self.currency
            && (currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()))
        {
            return Err(ContractError::InvalidContract(
                "usage currency must be a three-letter uppercase ISO code".to_owned(),
            ));
        }
        validate_extensions(&self.extensions)
    }
}
