//! Acceptance uses the existing artifact port and normal invocation journal. Publishing the
//! accepted marker last ensures a missing decision or publication cannot open a workflow gate.

use milkdrift_capability::{
    InputReference, InvocationEvent, InvocationEventKind, InvocationTerminal, SideEffectClass,
    TerminalStatus,
};
use milkdrift_capability_host::{AdapterError, AdapterInvocation, AdapterReporter};

use super::WorkflowControlAdapter;
use crate::{
    ACCEPTED_RESULT_OUTPUT, AcceptanceReason, RESULT_ACCEPTANCE_INPUT, RESULT_ACCEPTANCE_OUTPUT,
    ResultAcceptance, ResultAcceptanceContract,
};

impl WorkflowControlAdapter {
    pub(super) fn execute_acceptance(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let contract = super::inline_input(invocation, RESULT_ACCEPTANCE_INPUT)
            .ok()
            .and_then(|value| {
                serde_json::from_value::<ResultAcceptanceContract>(value.clone()).ok()
            });
        let result = match contract {
            Some(contract) => self.assess_result(invocation, &contract),
            None => ResultAcceptance::rejected(AcceptanceReason::InvalidStructure),
        };
        let bytes = serde_json::to_vec(&result)
            .map_err(|_| AdapterError::rejected("acceptance result encoding failed"))?;
        let reference = self
            .results
            .publish(invocation, RESULT_ACCEPTANCE_OUTPUT, &bytes)
            .map_err(|_| AdapterError::rejected("acceptance result publication failed"))?;
        let mut sequence = 1;
        let mut output = |name: &str| -> Result<(), AdapterError> {
            reporter.invocation(
                InvocationEvent::new(
                    invocation.request().invocation().clone(),
                    sequence,
                    InvocationEventKind::Output {
                        name: name.to_owned(),
                        reference: reference.clone(),
                    },
                )
                .map_err(|_| AdapterError::rejected("acceptance output is invalid"))?,
            )?;
            sequence += 1;
            Ok(())
        };
        output(RESULT_ACCEPTANCE_OUTPUT)?;
        if result.accepted {
            output(ACCEPTED_RESULT_OUTPUT)?;
        }
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                sequence,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        Vec::new(),
                        None,
                        None,
                        SideEffectClass::ReadOnly,
                    )
                    .map_err(|_| AdapterError::rejected("acceptance terminal is invalid"))?,
                },
            )
            .map_err(|_| AdapterError::rejected("acceptance terminal event is invalid"))?,
        )
    }

    fn assess_result(
        &self,
        invocation: &AdapterInvocation<'_>,
        contract: &ResultAcceptanceContract,
    ) -> ResultAcceptance {
        if invocation
            .context()
            .and_then(|context| context.entry_authorization())
            .is_none()
        {
            return ResultAcceptance::rejected(AcceptanceReason::EvidenceUnavailable);
        }
        let input = |name: &str| -> Option<&InputReference> {
            invocation
                .request()
                .inputs()
                .iter()
                .find(|input| input.name() == name)
        };
        let Some(source) = input("result") else {
            return ResultAcceptance::rejected(AcceptanceReason::MissingRequiredOutput);
        };
        let Ok((_, bytes)) = self.results.read(invocation, source) else {
            return ResultAcceptance::rejected(AcceptanceReason::EvidenceUnavailable);
        };
        let mut evidence = Vec::new();
        for name in contract.evidence_inputs() {
            let Some(input) = input(name) else {
                return ResultAcceptance::rejected(AcceptanceReason::EvidenceUnavailable);
            };
            let Ok((reference, _)) = self.results.read(invocation, input) else {
                return ResultAcceptance::rejected(AcceptanceReason::EvidenceUnavailable);
            };
            evidence.push(reference);
        }
        contract.evaluate(&bytes, &evidence)
    }
}
