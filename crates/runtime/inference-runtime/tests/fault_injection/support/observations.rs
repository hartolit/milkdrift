use super::*;

#[derive(Default)]
pub(crate) struct CleanupCounts {
    pub(crate) preparations: Cell<u32>,
    pub(crate) model_loads: Cell<u32>,
    pub(crate) model_cleanups: Cell<u32>,
    pub(crate) failed_load_cleanups: Cell<u32>,
    pub(crate) successful_failed_load_cleanups: Cell<u32>,
    pub(crate) retained_partial_load_bytes: Cell<u64>,
    pub(crate) sequence_creations: Cell<u32>,
    pub(crate) sequence_destructions: Cell<u32>,
    pub(crate) plan_reads: Cell<u32>,
    pub(crate) prepared_drops: Cell<u32>,
    pub(crate) retained_prepared_drops: Cell<u32>,
    pub(crate) successful_model_cleanups: Cell<u32>,
    pub(crate) model_drops_while_owned: Cell<u32>,
}
