//! Host-owned continuation port. Control supplies workflow meaning; the host never constructs a
//! workflow engine. The daemon retains the implementation and installs a weak reference to avoid
//! a host/runtime/control ownership cycle.
use milkdrift_capability::InvocationEvent;
use milkdrift_persistence::published::PublishedInvocationPlan;
use milkdrift_runtime::ExecutorError;

/// Advance an already accepted workflow-backed operation through its original runtime owner.
pub trait PublishedWorkflowContinuation: Send + Sync {
    /// Arrange/recover the exact child and observe one terminal result without waiting for work.
    /// Errors retain the association and must not authorize a replacement child.
    fn continue_invocation(
        &self,
        plan: &PublishedInvocationPlan,
        cancel: bool,
        next_sequence: u64,
    ) -> Result<Option<InvocationEvent>, ExecutorError>;
}
