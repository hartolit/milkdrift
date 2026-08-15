use super::*;

pub(crate) struct FaultSequence {
    pub(crate) id: SequenceId,
    pub(crate) state: SequenceState,
    pub(crate) position: usize,
    pub(crate) token_capacity: usize,
    pub(crate) plan: SequencePlan,
    pub(crate) faults: Faults,
}

impl BackendSequence for FaultSequence {
    fn id(&self) -> SequenceId {
        self.id
    }

    fn state(&self) -> SequenceState {
        self.state
    }

    fn position(&self) -> usize {
        self.position
    }

    fn token_capacity(&self) -> usize {
        self.token_capacity
    }

    fn reported_plan(&self) -> SequencePlan {
        self.plan
    }
}
