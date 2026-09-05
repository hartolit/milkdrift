//! Independent validation of persisted account totals, identity and digest.
use super::{
    ControllerAccountBlock, ControllerAccountState, ControllerReservationId,
    ControllerResourceTotals, checked_add,
};
use crate::PersistenceError;

impl ControllerAccountState {
    /// Validates all redundant totals and the exact revision digest.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.declaration.validate()?;
        validate_account_block(self.blocked.as_ref())?;
        let mut summed = ControllerResourceTotals::default();
        for (identity, reservation) in &self.reservations {
            if identity != reservation.reservation() {
                return Err(PersistenceError::InvalidDocument(
                    "controller reservation map identity mismatch".to_owned(),
                ));
            }
            if identity
                != &ControllerReservationId::for_attempt(
                    self.declaration.account(),
                    reservation.attempt(),
                )?
            {
                return Err(PersistenceError::InvalidDocument(
                    "controller reservation identity does not match its account and attempt"
                        .to_owned(),
                ));
            }
            summed.input_units = checked_add(
                summed.input_units,
                reservation.input.remaining().unwrap_or(0),
            )?;
            summed.output_units = checked_add(
                summed.output_units,
                reservation.output.remaining().unwrap_or(0),
            )?;
            summed.artifact_bytes = checked_add(
                summed.artifact_bytes,
                reservation.artifact.remaining().unwrap_or(0),
            )?;
            summed.cost_micros = checked_add(
                summed.cost_micros,
                reservation.cost.remaining().unwrap_or(0),
            )?;
        }
        if summed.cost_micros != self.outstanding.cost_micros
            || summed.input_units != self.outstanding.input_units
            || summed.output_units != self.outstanding.output_units
            || summed.artifact_bytes != self.outstanding.artifact_bytes
            || self.outstanding.process_admissions != 0
            || self.outstanding.model_admissions != 0
        {
            return Err(PersistenceError::InvalidDocument(
                "controller outstanding totals disagree with exact reservations".to_owned(),
            ));
        }
        let committed = self.committed_totals()?;
        let budget = self.declaration.budget();
        if committed.cost_micros > budget.cost_micros
            || committed.input_units > budget.input_units
            || committed.output_units > budget.output_units
            || committed.artifact_bytes > budget.artifact_bytes
            || committed.process_admissions > budget.process_admissions
            || committed.model_admissions > budget.model_admissions
        {
            return Err(PersistenceError::InvalidDocument(
                "controller committed use exceeds its immutable budget".to_owned(),
            ));
        }
        if self.calculate_digest()? != self.revision_digest {
            return Err(PersistenceError::InvalidDocument(
                "controller account revision digest mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_account_block(block: Option<&ControllerAccountBlock>) -> Result<(), PersistenceError> {
    match block {
        Some(
            ControllerAccountBlock::UnknownUsage { dimension, .. }
            | ControllerAccountBlock::ContractViolation { dimension, .. },
        ) if !matches!(
            dimension.as_str(),
            "input_units" | "output_units" | "artifact_bytes" | "monetary_cost"
        ) =>
        {
            Err(PersistenceError::InvalidDocument(
                "controller account block has an unknown resource dimension".to_owned(),
            ))
        }
        Some(ControllerAccountBlock::ContractViolation {
            observed, reserved, ..
        }) if observed <= reserved => Err(PersistenceError::InvalidDocument(
            "controller contract violation must exceed its admitted reservation".to_owned(),
        )),
        Some(ControllerAccountBlock::Integrity { reason })
            if reason.is_empty() || reason.len() > 512 =>
        {
            Err(PersistenceError::InvalidDocument(
                "controller integrity block reason must contain 1..=512 bytes".to_owned(),
            ))
        }
        Some(
            ControllerAccountBlock::UnknownUsage { .. }
            | ControllerAccountBlock::ContractViolation { .. }
            | ControllerAccountBlock::Integrity { .. },
        )
        | None => Ok(()),
    }
}
