//! One strict polling driver for public E1 lifecycle observation.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationEvent, ApplicationModelCleanupDisposition, ApplicationRetainedModelResource,
    ApplicationRuntime,
};
use domain_contracts::RequestId;

use super::cleanup::{generation_cleanup_error, retained_model_cleanup_error};
use super::deadline::Deadline;
use crate::error::{BenchmarkError, BenchmarkResult};

pub(crate) enum WaitStatus<T> {
    Pending,
    Complete(T),
}

pub(crate) trait ApplicationEventSource {
    fn poll_application_event(&mut self) -> Option<ApplicationEvent>;

    fn retained_cleanup_error(
        &self,
        stage: &'static str,
        _resource: ApplicationRetainedModelResource,
        _disposition: ApplicationModelCleanupDisposition,
    ) -> BenchmarkError {
        BenchmarkError::new(format!(
            "{stage} observed retained cleanup without a durable-state diagnostic"
        ))
    }
}

impl ApplicationEventSource for ApplicationRuntime {
    fn poll_application_event(&mut self) -> Option<ApplicationEvent> {
        self.poll_event()
    }

    fn retained_cleanup_error(
        &self,
        stage: &'static str,
        resource: ApplicationRetainedModelResource,
        disposition: ApplicationModelCleanupDisposition,
    ) -> BenchmarkError {
        let Some(retained) = self.state().retained_model() else {
            return BenchmarkError::new(format!(
                "{stage} observed retained model cleanup without durable public cleanup state"
            ));
        };
        if retained.resource() != resource || retained.cleanup() != disposition {
            return BenchmarkError::new(format!(
                "{stage} cleanup event disagreed with durable retained-model identity or disposition"
            ));
        }
        retained_model_cleanup_error(
            stage,
            resource,
            disposition,
            retained.primary_failure(),
            retained.cleanup_failure(),
        )
    }
}

pub(crate) trait ApplicationWaitStage<Source: ApplicationEventSource> {
    type Output;

    fn name(&self) -> &'static str;

    fn expected_request_id(&self) -> Option<RequestId> {
        None
    }

    fn observe_event(
        &mut self,
        source: &mut Source,
        event: ApplicationEvent,
        observed_at: Instant,
    ) -> BenchmarkResult<WaitStatus<Self::Output>>;

    // Returning Pending accepts only this stage's explicitly matched progress. The maintained
    // stages reject every other valid event; none silently permits unrelated traffic.

    fn observe_progress(
        &mut self,
        _source: &mut Source,
    ) -> BenchmarkResult<WaitStatus<Self::Output>> {
        Ok(WaitStatus::Pending)
    }
}

pub(crate) fn drive_application_wait<Source, Stage>(
    source: &mut Source,
    timeout: Duration,
    mut stage: Stage,
) -> BenchmarkResult<Stage::Output>
where
    Source: ApplicationEventSource,
    Stage: ApplicationWaitStage<Source>,
{
    let deadline = Deadline::after(timeout, stage.name())?;
    loop {
        if let Some(event) = source.poll_application_event() {
            let event = normalize_common_event(source, &stage, event)?;
            if let WaitStatus::Complete(output) =
                stage.observe_event(source, event, Instant::now())?
            {
                return Ok(output);
            }
        }
        if let WaitStatus::Complete(output) = stage.observe_progress(source)? {
            return Ok(output);
        }
        deadline.wait_for_poll(stage.name())?;
    }
}

fn normalize_common_event<Source, Stage>(
    source: &Source,
    stage: &Stage,
    event: ApplicationEvent,
) -> BenchmarkResult<ApplicationEvent>
where
    Source: ApplicationEventSource,
    Stage: ApplicationWaitStage<Source>,
{
    match event {
        ApplicationEvent::HubDisconnected => Err(BenchmarkError::new(format!(
            "Hub worker disconnected while waiting for {}",
            stage.name()
        ))),
        ApplicationEvent::RuntimeDisconnected => Err(BenchmarkError::new(format!(
            "inference worker disconnected while waiting for {}",
            stage.name()
        ))),
        ApplicationEvent::ModelCleanupPending {
            resource,
            disposition,
        } => Err(source.retained_cleanup_error(stage.name(), resource, disposition)),
        ApplicationEvent::GenerationCleanupPending {
            request_id,
            exhausted,
            failure,
        } => Err(generation_cleanup_error(
            stage.name(),
            stage.expected_request_id(),
            request_id,
            exhausted,
            &failure,
        )),
        event => Ok(event),
    }
}

pub(crate) fn unexpected_event(stage: &'static str, event: &ApplicationEvent) -> BenchmarkError {
    BenchmarkError::new(format!(
        "unexpected application event while waiting for {stage}: {event:?}"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    use application_runtime::{ApplicationEvent, ApplicationFailure, ApplicationFailureKind};

    use super::{
        ApplicationEventSource, ApplicationWaitStage, WaitStatus, drive_application_wait,
        unexpected_event,
    };
    use crate::error::BenchmarkResult;

    struct FakeSource {
        events: VecDeque<ApplicationEvent>,
    }

    impl ApplicationEventSource for FakeSource {
        fn poll_application_event(&mut self) -> Option<ApplicationEvent> {
            self.events.pop_front()
        }
    }

    struct RejectUnexpected;

    impl ApplicationWaitStage<FakeSource> for RejectUnexpected {
        type Output = ();

        fn name(&self) -> &'static str {
            "driver regression stage"
        }

        fn observe_event(
            &mut self,
            _source: &mut FakeSource,
            event: ApplicationEvent,
            _observed_at: Instant,
        ) -> BenchmarkResult<WaitStatus<Self::Output>> {
            Err(unexpected_event(self.name(), &event))
        }
    }

    fn wait_error(event: ApplicationEvent) -> Result<String, &'static str> {
        let mut source = FakeSource {
            events: VecDeque::from([event]),
        };
        let Err(error) =
            drive_application_wait(&mut source, Duration::from_secs(1), RejectUnexpected)
        else {
            return Err("event entered a successful wait result");
        };
        Ok(error.to_string())
    }

    #[test]
    fn driver_distinguishes_disconnection_from_timeout() -> Result<(), &'static str> {
        let error = wait_error(ApplicationEvent::HubDisconnected)?;
        assert!(error.contains("Hub worker disconnected"));
        assert!(!error.contains("timed out"));
        Ok(())
    }

    #[test]
    fn stage_rejects_an_unexpected_valid_application_event() -> Result<(), &'static str> {
        let failure = ApplicationFailure::new(ApplicationFailureKind::Hub, "resolution failed");
        let error = wait_error(ApplicationEvent::ModelResolutionFailed { failure })?;
        assert!(error.contains("unexpected application event"));
        assert!(error.contains("driver regression stage"));
        Ok(())
    }
}
