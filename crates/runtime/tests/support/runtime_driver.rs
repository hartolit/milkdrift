use milkdrift_persistence::{MAX_PAGE_SIZE, PageSize};
use milkdrift_runtime::{EffectExecutionResult, RuntimeError, RuntimeService, SchedulerTickResult};

pub(crate) fn runtime_tick(runtime: &RuntimeService) -> Result<SchedulerTickResult, RuntimeError> {
    let mut scheduled = runtime.scheduler_tick()?;
    let actions = runtime.claim_effects(PageSize::new(MAX_PAGE_SIZE)?)?;
    for action in actions {
        match runtime.execute_effect(action)? {
            EffectExecutionResult::Completed { .. } => {
                scheduled.completed = scheduled.completed.saturating_add(1);
            }
            EffectExecutionResult::Uncertain { .. } => {
                scheduled.uncertain = scheduled.uncertain.saturating_add(1);
            }
            EffectExecutionResult::CancellationAcknowledged
            | EffectExecutionResult::CancellationDeferred => {}
        }
    }
    Ok(scheduled)
}
