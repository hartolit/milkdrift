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
    if !limit.host_bytes().contains(used_host) {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: used_host,
            available_bytes: limit.host_bytes(),
        });
    }
    if !limit.device_bytes().contains(used_device) {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: used_device,
            available_bytes: limit.device_bytes(),
        });
    }
    Ok(MemoryBudget::ZERO
        .with_host_bytes(
            limit
                .host_bytes()
                .checked_sub(used_host)
                .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        )
        .with_device_bytes(
            limit
                .device_bytes()
                .checked_sub(used_device)
                .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        ))
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
    if !budget.host_bytes().contains(required_host) {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: required_host,
            available_bytes: budget.host_bytes(),
        });
    }
    let required_device = next
        .checked_device_bytes()
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    if !budget.device_bytes().contains(required_device) {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: budget.device_bytes(),
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
    validated_conservative_footprint(accepted_loading_peak.component_max(reported_footprint))
}

/// Extends retained unverified evidence without losing an earlier, larger report.
///
/// A backend whose report changes more than once is never allowed to shrink the
/// conservative evidence already published by E0. Once checked host/device
/// arithmetic overflows, later reports cannot restore a representable claim.
pub(super) const fn extend_conservative_footprint(
    current: ConservativeFootprint,
    reported_footprint: MemoryFootprint,
) -> ConservativeFootprint {
    match current {
        ConservativeFootprint::Known(current) => {
            validated_conservative_footprint(current.component_max(reported_footprint))
        }
        ConservativeFootprint::Overflow => ConservativeFootprint::Overflow,
    }
}

const fn validated_conservative_footprint(footprint: MemoryFootprint) -> ConservativeFootprint {
    if footprint.checked_host_bytes().is_some() && footprint.checked_device_bytes().is_some() {
        ConservativeFootprint::Known(footprint)
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
    use domain_contracts::ByteCount;

    const fn bytes(value: u64) -> ByteCount {
        ByteCount::from_u64(value)
    }

    const fn footprint(
        host_weights: u64,
        device_weights: u64,
        host_working: u64,
        device_working: u64,
    ) -> MemoryFootprint {
        MemoryFootprint::ZERO
            .with_host_weight_bytes(bytes(host_weights))
            .with_device_weight_bytes(bytes(device_weights))
            .with_host_working_bytes(bytes(host_working))
            .with_device_working_bytes(bytes(device_working))
    }

    const COMPONENTS: [MemoryFootprint; 4] = [
        MemoryFootprint::host_weights(bytes(1)),
        MemoryFootprint::device_weights(bytes(1)),
        MemoryFootprint::host_working(bytes(1)),
        MemoryFootprint::device_working(bytes(1)),
    ];

    #[test]
    fn component_addition_and_subtraction_are_exact_inverses() {
        let base = footprint(7, 11, 13, 17);
        for component in COMPONENTS {
            let round_trip = checked_add_footprint(base, component)
                .and_then(|added| checked_sub_footprint(added, component));
            assert_eq!(round_trip, Ok(base));
        }
    }

    #[test]
    fn every_component_overflow_and_underflow_is_rejected() {
        for component in COMPONENTS {
            let maximum = MemoryFootprint::ZERO
                .with_host_weight_bytes(if component.host_weight_bytes().is_zero() {
                    ByteCount::ZERO
                } else {
                    ByteCount::MAX
                })
                .with_device_weight_bytes(if component.device_weight_bytes().is_zero() {
                    ByteCount::ZERO
                } else {
                    ByteCount::MAX
                })
                .with_host_working_bytes(if component.host_working_bytes().is_zero() {
                    ByteCount::ZERO
                } else {
                    ByteCount::MAX
                })
                .with_device_working_bytes(if component.device_working_bytes().is_zero() {
                    ByteCount::ZERO
                } else {
                    ByteCount::MAX
                });
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
        let host = footprint(u64::MAX, 0, 1, 0);
        assert_eq!(
            admit_footprint(MemoryFootprint::default(), host, MemoryBudget::UNLIMITED,),
            Err(RuntimeError::MemoryArithmeticOverflow)
        );
        assert_eq!(
            conservative_footprint(MemoryFootprint::default(), host),
            ConservativeFootprint::Overflow
        );
    }

    #[test]
    fn conservative_evidence_preserves_reclassified_components_without_exactness() {
        let planned = footprint(64, 0, 0, 0);
        let reported = footprint(0, 96, 0, 0);
        assert_eq!(
            conservative_footprint(planned, reported),
            ConservativeFootprint::Known(footprint(64, 96, 0, 0))
        );
    }

    #[test]
    fn conservative_extension_is_monotonic_across_changing_reports() {
        let initial = ConservativeFootprint::Known(footprint(64, 0, 32, 0));
        let larger_device_report = footprint(0, 128, 0, 16);
        let extended = extend_conservative_footprint(initial, larger_device_report);
        assert_eq!(
            extended,
            ConservativeFootprint::Known(footprint(64, 128, 32, 16))
        );

        let smaller_report = footprint(1, 2, 3, 4);
        assert_eq!(
            extend_conservative_footprint(extended, smaller_report),
            extended
        );
    }

    #[test]
    fn conservative_overflow_is_sticky() {
        assert_eq!(
            extend_conservative_footprint(
                ConservativeFootprint::Overflow,
                MemoryFootprint::default(),
            ),
            ConservativeFootprint::Overflow
        );
    }
}
