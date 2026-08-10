use domain_contracts::{MemoryBudget, MemoryFootprint, ModelLoader};

use crate::{ConservativeFootprint, MemoryKind, RuntimeError};

use super::InferenceRuntime;

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Releases one generation task's host workspace after terminal output publication.
    pub(crate) fn release_generation_workspace(
        &mut self,
        footprint: MemoryFootprint,
    ) -> Result<(), RuntimeError> {
        let next_generation_workspaces = self
            .generation_workspaces
            .checked_sub(1)
            .ok_or(RuntimeError::BackendContractViolation)?;
        let next_reserved_generation_workspace =
            checked_sub_footprint(self.reserved_generation_workspace, footprint)?;
        let next_reserved_footprint = checked_sub_footprint(self.reserved_footprint, footprint)?;
        self.generation_workspaces = next_generation_workspaces;
        self.reserved_generation_workspace = next_reserved_generation_workspace;
        self.reserved_footprint = next_reserved_footprint;
        Ok(())
    }
}

pub(super) fn remaining_budget(
    limit: MemoryBudget,
    used: MemoryFootprint,
) -> Result<MemoryBudget, RuntimeError> {
    let used_host = used
        .checked_host_bytes()
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    let used_device = used
        .checked_device_bytes()
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    if used_host > limit.host_bytes {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: used_host,
            available_bytes: limit.host_bytes,
        });
    }
    if used_device > limit.device_bytes {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: used_device,
            available_bytes: limit.device_bytes,
        });
    }
    Ok(MemoryBudget {
        host_bytes: limit.host_bytes - used_host,
        device_bytes: limit.device_bytes - used_device,
    })
}

pub(super) fn admit_footprint(
    current: MemoryFootprint,
    additional: MemoryFootprint,
    budget: MemoryBudget,
) -> Result<MemoryFootprint, RuntimeError> {
    let next = checked_add_footprint(current, additional)?;
    let required_host = next
        .checked_host_bytes()
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    if required_host > budget.host_bytes {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: required_host,
            available_bytes: budget.host_bytes,
        });
    }
    let required_device = next
        .checked_device_bytes()
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    if required_device > budget.device_bytes {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: budget.device_bytes,
        });
    }
    Ok(next)
}

pub(super) fn checked_add_footprint(
    left: MemoryFootprint,
    right: MemoryFootprint,
) -> Result<MemoryFootprint, RuntimeError> {
    left.checked_add(right)
        .ok_or(RuntimeError::MemoryArithmeticOverflow)
}

pub(super) fn checked_sub_footprint(
    left: MemoryFootprint,
    right: MemoryFootprint,
) -> Result<MemoryFootprint, RuntimeError> {
    left.checked_sub(right)
        .ok_or(RuntimeError::MemoryArithmeticUnderflow)
}

pub(super) const fn conservative_footprint(
    accepted_loading_peak: MemoryFootprint,
    reported_footprint: MemoryFootprint,
) -> ConservativeFootprint {
    let conservative = accepted_loading_peak.component_max(reported_footprint);
    if conservative.checked_host_bytes().is_some() && conservative.checked_device_bytes().is_some()
    {
        ConservativeFootprint::Known(conservative)
    } else {
        ConservativeFootprint::Overflow
    }
}

pub(super) fn add_conservative_footprint(
    current: ConservativeFootprint,
    additional: ConservativeFootprint,
) -> ConservativeFootprint {
    let (ConservativeFootprint::Known(current), ConservativeFootprint::Known(additional)) =
        (current, additional)
    else {
        return ConservativeFootprint::Overflow;
    };
    let Some(next) = current.checked_add(additional) else {
        return ConservativeFootprint::Overflow;
    };
    if next.checked_host_bytes().is_some() && next.checked_device_bytes().is_some() {
        ConservativeFootprint::Known(next)
    } else {
        ConservativeFootprint::Overflow
    }
}

pub(super) fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(super) fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPONENTS: [MemoryFootprint; 4] = [
        MemoryFootprint {
            host_weight_bytes: 1,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
        },
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 1,
            host_working_bytes: 0,
            device_working_bytes: 0,
        },
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 1,
            device_working_bytes: 0,
        },
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 1,
        },
    ];

    #[test]
    fn component_addition_and_subtraction_are_exact_inverses() {
        let base = MemoryFootprint {
            host_weight_bytes: 7,
            device_weight_bytes: 11,
            host_working_bytes: 13,
            device_working_bytes: 17,
        };
        for component in COMPONENTS {
            let round_trip = checked_add_footprint(base, component)
                .and_then(|added| checked_sub_footprint(added, component));
            assert_eq!(round_trip, Ok(base));
        }
    }

    #[test]
    fn every_component_overflow_and_underflow_is_rejected() {
        for component in COMPONENTS {
            let maximum = MemoryFootprint {
                host_weight_bytes: if component.host_weight_bytes == 1 {
                    u64::MAX
                } else {
                    0
                },
                device_weight_bytes: if component.device_weight_bytes == 1 {
                    u64::MAX
                } else {
                    0
                },
                host_working_bytes: if component.host_working_bytes == 1 {
                    u64::MAX
                } else {
                    0
                },
                device_working_bytes: if component.device_working_bytes == 1 {
                    u64::MAX
                } else {
                    0
                },
            };
            assert_eq!(
                checked_add_footprint(maximum, component),
                Err(RuntimeError::MemoryArithmeticOverflow)
            );
            assert_eq!(
                checked_sub_footprint(MemoryFootprint::default(), component),
                Err(RuntimeError::MemoryArithmeticUnderflow)
            );
        }
    }

    #[test]
    fn domain_total_overflow_is_rejected_even_when_components_fit() {
        let host = MemoryFootprint {
            host_weight_bytes: u64::MAX,
            host_working_bytes: 1,
            ..MemoryFootprint::default()
        };
        assert_eq!(
            admit_footprint(
                MemoryFootprint::default(),
                host,
                MemoryBudget {
                    host_bytes: u64::MAX,
                    device_bytes: u64::MAX,
                },
            ),
            Err(RuntimeError::MemoryArithmeticOverflow)
        );
        assert_eq!(
            conservative_footprint(MemoryFootprint::default(), host),
            ConservativeFootprint::Overflow
        );
    }

    #[test]
    fn conservative_evidence_preserves_reclassified_components_without_exactness() {
        let planned = MemoryFootprint {
            host_weight_bytes: 64,
            ..MemoryFootprint::default()
        };
        let reported = MemoryFootprint {
            device_weight_bytes: 96,
            ..MemoryFootprint::default()
        };
        assert_eq!(
            conservative_footprint(planned, reported),
            ConservativeFootprint::Known(MemoryFootprint {
                host_weight_bytes: 64,
                device_weight_bytes: 96,
                ..MemoryFootprint::default()
            })
        );
    }
}
