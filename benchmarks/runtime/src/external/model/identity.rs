//! Single ownership of the immutable external model identity.

use application_runtime::{
    ApplicationScalarType, ChatCompatibility, ModelSelection, ResolvedModel,
};

use crate::error::{BenchmarkError, BenchmarkResult};

use super::{MODEL_REPOSITORY, MODEL_REVISION};

pub(super) const EXPECTED_VOCABULARY_SIZE: u32 = 32_000;
pub(super) const EXPECTED_CONTEXT_TOKENS: u32 = 2_048;
pub(super) const MODEL_CONFIGURATION_DECLARED_SCALAR: Option<ApplicationScalarType> =
    Some(ApplicationScalarType::Bf16);

pub(super) fn validate_exact_selection(selection: &ModelSelection) -> BenchmarkResult {
    if selection.repository() != MODEL_REPOSITORY || selection.revision() != MODEL_REVISION {
        return Err(BenchmarkError::new(format!(
            "external model selection must be exactly {MODEL_REPOSITORY}@{MODEL_REVISION}, received {}@{}",
            selection.repository(),
            selection.revision()
        )));
    }
    Ok(())
}

pub(super) fn validate_resolved_facts(
    model: &ResolvedModel,
    selection: &ModelSelection,
) -> BenchmarkResult<Option<ApplicationScalarType>> {
    validate_exact_selection(selection)?;
    let configuration_declared_scalar_type = model.configuration_declared_scalar_type();
    if model.selection() != selection
        || model.identity().repository() != MODEL_REPOSITORY
        || model.identity().commit() != MODEL_REVISION
        || configuration_declared_scalar_type != MODEL_CONFIGURATION_DECLARED_SCALAR
        || model.vocabulary_size() != EXPECTED_VOCABULARY_SIZE
        || model.chat_compatibility() != ChatCompatibility::Supported
    {
        return Err(BenchmarkError::new(format!(
            "resolved model did not retain the exact immutable TinyLlama selection, identity, optional BF16 declaration, vocabulary, and chat-compatibility facts: {model:?}"
        )));
    }
    Ok(configuration_declared_scalar_type)
}

#[cfg(test)]
mod tests {
    use application_runtime::{ApplicationScalarType, ModelSelection};

    use super::{
        EXPECTED_CONTEXT_TOKENS, EXPECTED_VOCABULARY_SIZE, MODEL_CONFIGURATION_DECLARED_SCALAR,
        MODEL_REPOSITORY, MODEL_REVISION, validate_exact_selection,
    };
    use crate::external::model::MODEL_ARCHITECTURE;

    #[test]
    fn fixed_external_identity_and_declared_capacities_are_exact() {
        assert_eq!(MODEL_REPOSITORY, "TinyLlama/TinyLlama-1.1B-Chat-v1.0");
        assert_eq!(MODEL_REVISION, "fe8a4ea1ffedaf415f4da2f062534de366a451e6");
        assert_eq!(MODEL_ARCHITECTURE, "Llama");
        assert_eq!(EXPECTED_VOCABULARY_SIZE, 32_000);
        assert_eq!(EXPECTED_CONTEXT_TOKENS, 2_048);
        assert_eq!(
            MODEL_CONFIGURATION_DECLARED_SCALAR,
            Some(ApplicationScalarType::Bf16)
        );
    }

    #[test]
    fn exact_selection_rejects_repository_or_revision_substitution() -> Result<(), String> {
        validate_exact_selection(&ModelSelection::new(MODEL_REPOSITORY, MODEL_REVISION))
            .map_err(|error| error.to_string())?;
        assert!(
            validate_exact_selection(&ModelSelection::new("other/repository", MODEL_REVISION))
                .is_err()
        );
        assert!(
            validate_exact_selection(&ModelSelection::new(MODEL_REPOSITORY, "other-revision"))
                .is_err()
        );
        Ok(())
    }
}
