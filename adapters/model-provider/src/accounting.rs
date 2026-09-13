//! Operator billing and token contracts, applied to the complete encoded text request.
//!
//! Byte BPE starts with at most one token per UTF-8 byte and only merges. The operator
//! must establish non-expanding normalization and a bound on everything the server's
//! template adds. This is not an estimated characters-to-tokens ratio or discovery.
use milkdrift_capability::{AdmissionBound, AdmissionMonetaryBound, InvocationAdmissionEnvelope};
use milkdrift_model::{ModelResponse, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EndpointProfile, ProfileError, ProviderProtocol};

/// Billing selected by the operator for this exact profile generation, independent of locality.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BillingTerms {
    /// No supported charge contract. Controlled admission remains refused.
    Unknown,
    /// The operator declares that this endpoint incurs no provider charge.
    Unbilled {
        /// Recorded operator declaration and version; never supplied by workflow input.
        source: String,
    },
    /// All charges are input, cached input, or generated text tokens (including reasoning).
    /// Additional billable categories require a different supported mapping and are refused.
    TextTariff {
        /// Exact uppercase three-letter currency.
        currency: String,
        /// Millionths of the currency per million uncached input tokens.
        input_micros_per_million: u64,
        /// Millionths of the currency per million cached input tokens.
        cached_input_micros_per_million: u64,
        /// Millionths of the currency per million output tokens, including reasoning.
        output_micros_per_million: u64,
        /// Tariff source/version and attestation that no additional charges apply.
        source: String,
    },
}

/// The endpoint's supported output parameter. Both contracts cover all generated tokens
/// (including hidden reasoning), for exactly one choice, even after a client disconnect.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputTokenControl {
    /// Servers whose `max_tokens` bounds total generation, including reasoning.
    MaxTokens,
    /// Chat endpoints/models requiring `max_completion_tokens` instead.
    MaxCompletionTokens,
}

impl OutputTokenControl {
    pub(crate) const fn field(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
        }
    }
}

/// A supported counting contract for the exact prepared text request.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelTokenLimits {
    /// No hard counting/output contract; ordinary generation remains available.
    Unknown,
    /// UTF-8 byte BPE with non-expanding normalization and bounded server templating.
    /// Milkdrift counts the entire encoded request plus the declared template overhead.
    /// Counts logical prompt tokens once; cache re-evaluation is not additional input usage.
    /// The server must perform no hidden extra calls. A tariff must charge only these logical
    /// categories, never additional context-shift/re-evaluation work.
    ByteBpe {
        /// Upper bound on extra template tokens per message, beyond the encoded body bytes.
        template_tokens_per_message: u64,
        /// Upper bound on fixed hidden/template tokens, including generation prefixes.
        template_tokens_per_request: u64,
        /// Local refusal ceiling for the computed input bound; not a model context-window claim.
        maximum_input_tokens: u64,
        /// Maximum output allowance this server/model supports.
        maximum_output_tokens: u64,
        /// Parameter with an operator-established total-generation enforcement contract.
        output_control: OutputTokenControl,
        /// Tokenizer/template/server identities and evidence for all of these obligations.
        source: String,
    },
}

fn source_valid(source: &str) -> bool {
    !source.trim().is_empty() && source.len() <= 2048 && !source.chars().any(char::is_control)
}

impl EndpointProfile {
    pub(crate) fn validate_accounting(&self) -> Result<(), ProfileError> {
        let valid_billing = match self.billing() {
            BillingTerms::Unknown => true,
            BillingTerms::Unbilled { source } => source_valid(source),
            BillingTerms::TextTariff {
                currency, source, ..
            } => source_valid(source) && AdmissionMonetaryBound::new(0, currency).is_ok(),
        };
        let valid_limits = match self.token_limits() {
            ModelTokenLimits::Unknown => true,
            ModelTokenLimits::ByteBpe {
                maximum_input_tokens,
                maximum_output_tokens,
                source,
                ..
            } => {
                matches!(self.protocol(), ProviderProtocol::OpenAiCompatible { .. })
                    && *maximum_input_tokens > 0
                    && *maximum_output_tokens > 0
                    && *maximum_output_tokens <= milkdrift_model::MAX_MODEL_OUTPUT_UNITS
                    && source_valid(source)
            }
        };
        if !valid_billing
            || !valid_limits
            || (!matches!(self.billing(), BillingTerms::Unknown)
                && !matches!(self.protocol(), ProviderProtocol::OpenAiCompatible { .. }))
        {
            return Err(ProfileError::Invalid(
                "invalid model billing/token contract".to_owned(),
            ));
        }
        if matches!(self.billing(), BillingTerms::TextTariff { .. })
            && matches!(self.token_limits(), ModelTokenLimits::Unknown)
        {
            return Err(ProfileError::Invalid(
                "text tariff requires supported text token limits".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn output_control(&self) -> OutputTokenControl {
        match self.token_limits() {
            ModelTokenLimits::Unknown => OutputTokenControl::MaxTokens,
            ModelTokenLimits::ByteBpe { output_control, .. } => *output_control,
        }
    }

    pub(crate) fn authority_cost_minor(&self) -> Result<Option<u64>, &'static str> {
        let ModelTokenLimits::ByteBpe {
            maximum_input_tokens,
            maximum_output_tokens,
            ..
        } = self.token_limits()
        else {
            return Ok(None);
        };
        // Authority is checked before request preparation, so it needs the largest call
        // this generation permits. The shared account later reserves the exact request.
        let bound = self.monetary_bound(
            &AdmissionBound::Bounded(*maximum_input_tokens),
            &AdmissionBound::Bounded(*maximum_output_tokens),
        )?;
        Ok(bound
            .bounded()
            .map(|cost| cost.maximum_micros().div_ceil(10_000)))
    }

    pub(crate) fn prepared_envelope(
        &self,
        wire: &Value,
        bytes: &[u8],
    ) -> Result<InvocationAdmissionEnvelope, &'static str> {
        let (input, output) = match self.token_limits() {
            ModelTokenLimits::Unknown => (AdmissionBound::Unknown, AdmissionBound::Unknown),
            ModelTokenLimits::ByteBpe {
                template_tokens_per_message,
                template_tokens_per_request,
                maximum_input_tokens,
                maximum_output_tokens,
                output_control,
                ..
            } => {
                // An allowlist prevents extensions from introducing images, tool billing, multiple
                // choices, extra generation, alternate limits, or server-side prompt expansion.
                let root = wire.as_object().ok_or("model request is not an object")?;
                if root.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "model"
                            | "messages"
                            | "stream"
                            | "stream_options"
                            | "max_tokens"
                            | "max_completion_tokens"
                    )
                }) || root.contains_key(if *output_control == OutputTokenControl::MaxTokens {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                }) {
                    return Err(
                        "bounded text accounting refuses unsupported or conflicting provider options",
                    );
                }
                let messages = wire["messages"]
                    .as_array()
                    .ok_or("text messages are absent")?;
                for message in messages {
                    let object = message.as_object().ok_or("invalid text message")?;
                    if object.keys().any(|key| key != "role" && key != "content")
                        || !matches!(
                            message["role"].as_str(),
                            Some("system" | "user" | "developer")
                        )
                    {
                        return Err(
                            "bounded text accounting supports system, developer and user text only",
                        );
                    }
                    let content = &message["content"];
                    if !content.is_string()
                        && !content.as_array().is_some_and(|parts| {
                            parts.iter().all(|part| {
                                part.as_object().is_some_and(|p| p.len() == 2)
                                    && part["type"] == "text"
                                    && part["text"].is_string()
                            })
                        })
                    {
                        return Err("bounded text accounting refuses non-text input parts");
                    }
                }
                let input = u64::try_from(bytes.len())
                    .ok()
                    .and_then(|n| {
                        template_tokens_per_message
                            .checked_mul(messages.len() as u64)
                            .and_then(|overhead| n.checked_add(overhead))
                    })
                    .and_then(|n| n.checked_add(*template_tokens_per_request))
                    .ok_or("model input bound arithmetic overflow")?;
                let output = wire[output_control.field()]
                    .as_u64()
                    .ok_or("supported output limit is absent")?;
                if input > *maximum_input_tokens || output == 0 || output > *maximum_output_tokens {
                    return Err(
                        "prepared model request exceeds its input or output token contract",
                    );
                }
                (
                    AdmissionBound::Bounded(input),
                    AdmissionBound::Bounded(output),
                )
            }
        };
        let cost = self.monetary_bound(&input, &output)?;
        Ok(InvocationAdmissionEnvelope::new(
            milkdrift_capability::AdmissionUnit::ModelTokens,
            input,
            output,
            AdmissionBound::Bounded(self.limits().max_response_bytes * 4),
            cost,
        ))
    }

    fn monetary_bound(
        &self,
        input: &AdmissionBound<u64>,
        output: &AdmissionBound<u64>,
    ) -> Result<AdmissionBound<AdmissionMonetaryBound>, &'static str> {
        Ok(match self.billing() {
            BillingTerms::Unknown => AdmissionBound::Unknown,
            BillingTerms::Unbilled { .. } => AdmissionBound::NotApplicable,
            BillingTerms::TextTariff {
                currency,
                input_micros_per_million,
                cached_input_micros_per_million,
                output_micros_per_million,
                ..
            } => {
                let input = *input.bounded().ok_or("tariff input bound is unknown")?;
                let output = *output.bounded().ok_or("tariff output bound is unknown")?;
                let cost = charge(&[
                    (
                        input,
                        (*input_micros_per_million).max(*cached_input_micros_per_million),
                    ),
                    (output, *output_micros_per_million),
                ])?;
                AdmissionBound::Bounded(
                    AdmissionMonetaryBound::new(cost, currency)
                        .map_err(|_| "invalid tariff currency")?,
                )
            }
        })
    }

    pub(crate) fn accounted_usage(&self, response: &ModelResponse) -> Result<Usage, &'static str> {
        let mut usage = response.usage().clone();
        match self.billing() {
            BillingTerms::Unknown => {}
            BillingTerms::Unbilled { .. } => {
                if usage.cost_micros.is_some_and(|cost| cost != 0) {
                    return Err("provider charge contradicts the unbilled declaration");
                }
                // No fabricated zero-cost invoice or currency is needed for a non-applicable charge.
                usage.cost_micros = None;
                usage.currency = None;
            }
            BillingTerms::TextTariff {
                currency,
                input_micros_per_million,
                cached_input_micros_per_million,
                output_micros_per_million,
                ..
            } => {
                if usage
                    .currency
                    .as_ref()
                    .is_some_and(|value| value != currency)
                {
                    return Err("provider charge currency contradicts the frozen tariff");
                }
                let calculated = usage
                    .input_units
                    .zip(usage.output_units)
                    .map(|(input, output)| {
                        let cached = if input_micros_per_million == cached_input_micros_per_million
                        {
                            usage.cached_input_units.unwrap_or(0)
                        } else {
                            usage
                                .cached_input_units
                                .ok_or("cached token evidence required by tariff is missing")?
                        };
                        let uncached = input
                            .checked_sub(cached)
                            .ok_or("cached input exceeds total input")?;
                        charge(&[
                            (uncached, *input_micros_per_million),
                            (cached, *cached_input_micros_per_million),
                            (output, *output_micros_per_million),
                        ])
                    })
                    .transpose()?;
                if usage.cost_micros.is_some() && usage.cost_micros != calculated {
                    return Err("provider charge contradicts the frozen text tariff");
                }
                usage.cost_micros = calculated;
                usage.currency = calculated.map(|_| currency.clone());
            }
        }
        Ok(usage)
    }
}

fn charge(categories: &[(u64, u64)]) -> Result<u64, &'static str> {
    let numerator = categories
        .iter()
        .try_fold(0u128, |total, (tokens, rate)| {
            total.checked_add(u128::from(*tokens) * u128::from(*rate))
        })
        .ok_or("tariff arithmetic overflow")?;
    u64::try_from(numerator.div_ceil(1_000_000)).map_err(|_| "tariff arithmetic overflow")
}

#[cfg(test)]
mod tests;
