use domain_contracts::{MemoryBudget, MemoryFootprint, ModelLoader};

use crate::{MemoryKind, RuntimeError};

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
    Ok(MemoryBudget {
        host_bytes: limit.host_bytes.saturating_sub(used_host),
        device_bytes: limit.device_bytes.saturating_sub(used_device),
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
    Ok(MemoryFootprint {
        host_weight_bytes: left
            .host_weight_bytes
            .checked_add(right.host_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        device_weight_bytes: left
            .device_weight_bytes
            .checked_add(right.device_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        host_working_bytes: left
            .host_working_bytes
            .checked_add(right.host_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        device_working_bytes: left
            .device_working_bytes
            .checked_add(right.device_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        cache_bytes_per_token: left
            .cache_bytes_per_token
            .checked_add(right.cache_bytes_per_token)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
    })
}

pub(super) fn checked_sub_footprint(
    left: MemoryFootprint,
    right: MemoryFootprint,
) -> Result<MemoryFootprint, RuntimeError> {
    Ok(MemoryFootprint {
        host_weight_bytes: left
            .host_weight_bytes
            .checked_sub(right.host_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        device_weight_bytes: left
            .device_weight_bytes
            .checked_sub(right.device_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        host_working_bytes: left
            .host_working_bytes
            .checked_sub(right.host_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        device_working_bytes: left
            .device_working_bytes
            .checked_sub(right.device_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        cache_bytes_per_token: left
            .cache_bytes_per_token
            .checked_sub(right.cache_bytes_per_token)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
    })
}

pub(super) fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(super) fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
