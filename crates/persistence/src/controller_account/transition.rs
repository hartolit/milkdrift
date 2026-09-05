//! Account admission, terminal settlement and logical artifact charging.
use super::{
    ControllerAccountBlock, ControllerAccountState, ControllerAdmissionDenial,
    ControllerAdmissionOutcome, ControllerArtifactChargeOutcome, ControllerReservation,
    ControllerReservationDimension, ControllerReservationId, checked_add,
};
use crate::{AttemptId, PersistenceError};
use milkdrift_capability::{AdmissionBound, CapabilityCategory, InvocationAdmissionEnvelope};

impl ControllerAccountState {
    /// Applies one validated entry operation and returns the independently computed outcome.
    pub fn admit(
        &mut self,
        reservation: ControllerReservationId,
        attempt: AttemptId,
        category: CapabilityCategory,
        envelope: &InvocationAdmissionEnvelope,
    ) -> Result<ControllerAdmissionOutcome, PersistenceError> {
        if reservation
            != ControllerReservationId::for_attempt(self.declaration.account(), &attempt)?
        {
            return Err(PersistenceError::InvalidDocument(
                "controller reservation identity does not match its account and attempt".to_owned(),
            ));
        }
        if self.blocked.is_some() {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Blocked,
            });
        }
        if self.reservations.contains_key(&reservation) {
            return Err(PersistenceError::ImmutableConflict {
                entity: "controller reservation",
                identity: reservation.as_str().to_owned(),
            });
        }
        for (dimension, bound) in [
            ("input_units", envelope.input_units()),
            ("output_units", envelope.output_units()),
            ("artifact_bytes", envelope.artifact_bytes()),
        ] {
            if bound.is_unknown() {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Unknown {
                        dimension: dimension.to_owned(),
                    },
                });
            }
        }
        let input = admission_value(envelope.input_units());
        let output = admission_value(envelope.output_units());
        let artifact = admission_value(envelope.artifact_bytes());
        let cost = match envelope.monetary_cost() {
            AdmissionBound::Bounded(value) => {
                if value.currency() != self.declaration.budget.currency.as_str() {
                    return Ok(ControllerAdmissionOutcome::Denied {
                        account: self.declaration.account.clone(),
                        reason: ControllerAdmissionDenial::CurrencyMismatch,
                    });
                }
                Some(value.maximum_micros())
            }
            AdmissionBound::NotApplicable => None,
            AdmissionBound::Unknown => {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Unknown {
                        dimension: "monetary_cost".to_owned(),
                    },
                });
            }
        };
        for (dimension, value, settled, outstanding, limit) in [
            (
                "input_units",
                input,
                self.settled.input_units,
                self.outstanding.input_units,
                self.declaration.budget.input_units,
            ),
            (
                "output_units",
                output,
                self.settled.output_units,
                self.outstanding.output_units,
                self.declaration.budget.output_units,
            ),
            (
                "artifact_bytes",
                artifact,
                self.settled.artifact_bytes,
                self.outstanding.artifact_bytes,
                self.declaration.budget.artifact_bytes,
            ),
            (
                "monetary_cost",
                cost,
                self.settled.cost_micros,
                self.outstanding.cost_micros,
                self.declaration.budget.cost_micros,
            ),
        ] {
            let Some(value) = value else {
                continue;
            };
            let Some(candidate) = settled
                .checked_add(outstanding)
                .and_then(|committed| committed.checked_add(value))
            else {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Overflow {
                        dimension: dimension.to_owned(),
                    },
                });
            };
            let exceeds = candidate > limit;
            if exceeds {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Limit {
                        dimension: dimension.to_owned(),
                    },
                });
            }
        }
        let (process, model) = match category {
            CapabilityCategory::Process => (1, 0),
            CapabilityCategory::Model => (0, 1),
            CapabilityCategory::Tool
            | CapabilityCategory::Human
            | CapabilityCategory::Peer
            | CapabilityCategory::Custom(_) => (0, 0),
        };
        let Some(process_candidate) = self.settled.process_admissions.checked_add(process) else {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Overflow {
                    dimension: "process_admissions".to_owned(),
                },
            });
        };
        if process_candidate > self.declaration.budget.process_admissions {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Limit {
                    dimension: "process_admissions".to_owned(),
                },
            });
        }
        let Some(model_candidate) = self.settled.model_admissions.checked_add(model) else {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Overflow {
                    dimension: "model_admissions".to_owned(),
                },
            });
        };
        if model_candidate > self.declaration.budget.model_admissions {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Limit {
                    dimension: "model_admissions".to_owned(),
                },
            });
        }
        self.outstanding.input_units =
            checked_add(self.outstanding.input_units, input.unwrap_or(0))?;
        self.outstanding.output_units =
            checked_add(self.outstanding.output_units, output.unwrap_or(0))?;
        self.outstanding.artifact_bytes =
            checked_add(self.outstanding.artifact_bytes, artifact.unwrap_or(0))?;
        self.outstanding.cost_micros =
            checked_add(self.outstanding.cost_micros, cost.unwrap_or(0))?;
        self.settled.process_admissions = checked_add(self.settled.process_admissions, process)?;
        self.settled.model_admissions = checked_add(self.settled.model_admissions, model)?;
        self.reservations.insert(
            reservation.clone(),
            ControllerReservation {
                reservation: reservation.clone(),
                attempt,
                category,
                input: ControllerReservationDimension::from_admitted_value(input),
                output: ControllerReservationDimension::from_admitted_value(output),
                artifact: ControllerReservationDimension::from_admitted_value(artifact),
                cost: ControllerReservationDimension::from_admitted_value(cost),
            },
        );
        self.advance()?;
        Ok(ControllerAdmissionOutcome::Reserved {
            account: self.declaration.account.clone(),
            reservation,
        })
    }

    /// Settles exact terminal usage. `None` retains a bounded remainder and blocks admission.
    pub fn settle_terminal(
        &mut self,
        reservation: &ControllerReservationId,
        usage: Option<&crate::AttemptUsage>,
    ) -> Result<(), PersistenceError> {
        let mut record =
            self.reservations
                .remove(reservation)
                .ok_or_else(|| PersistenceError::NotFound {
                    entity: "controller reservation",
                    identity: reservation.as_str().to_owned(),
                })?;
        settle_dimension(
            "input_units",
            reservation,
            &mut record.input,
            usage.and_then(|value| value.input_units),
            &mut self.outstanding.input_units,
            &mut self.settled.input_units,
            &mut self.blocked,
        )?;
        settle_dimension(
            "output_units",
            reservation,
            &mut record.output,
            usage.and_then(|value| value.output_units),
            &mut self.outstanding.output_units,
            &mut self.settled.output_units,
            &mut self.blocked,
        )?;
        let observed_cost = usage
            .and_then(|value| value.cost.as_ref())
            .and_then(|cost| {
                (cost.currency == self.declaration.budget.currency).then_some(cost.micros)
            });
        if usage
            .and_then(|value| value.cost.as_ref())
            .is_some_and(|cost| cost.currency != self.declaration.budget.currency)
            && self.blocked.is_none()
        {
            self.blocked = Some(ControllerAccountBlock::Integrity {
                reason: "terminal cost currency differs from the admitted controller currency"
                    .to_owned(),
            });
        }
        settle_dimension(
            "monetary_cost",
            reservation,
            &mut record.cost,
            observed_cost,
            &mut self.outstanding.cost_micros,
            &mut self.settled.cost_micros,
            &mut self.blocked,
        )?;
        // Artifact bytes settle only at publication. A known terminal proves that no later
        // adapter publication for this synchronous invocation can begin.
        if let ControllerReservationDimension::Outstanding(remaining) = record.artifact {
            self.outstanding.artifact_bytes = self
                .outstanding
                .artifact_bytes
                .checked_sub(remaining)
                .ok_or_else(|| {
                    PersistenceError::InvalidDocument(
                        "controller artifact remainder underflow".to_owned(),
                    )
                })?;
            record.artifact = ControllerReservationDimension::Settled;
        }
        if record.input.remaining().is_some()
            || record.output.remaining().is_some()
            || record.cost.remaining().is_some()
        {
            self.reservations.insert(reservation.clone(), record);
        }
        self.advance()
    }

    /// Charges first logical artifact publication against a reservation or directly to the account.
    ///
    /// An invocation publication above its exact reservation is not charged. It instead records a
    /// durable contract-violation block so later admission cannot treat the failed publication as
    /// though the adapter had remained within its declared envelope.
    pub fn charge_artifact(
        &mut self,
        reservation: Option<&ControllerReservationId>,
        bytes: u64,
    ) -> Result<ControllerArtifactChargeOutcome, PersistenceError> {
        if reservation.is_none() && self.blocked.is_some() {
            return Err(PersistenceError::Bounds {
                location: "controller.artifact_budget",
                reason: "controller account is durably blocked".to_owned(),
            });
        }
        if let Some(reservation) = reservation {
            let artifact = self
                .reservations
                .get(reservation)
                .ok_or_else(|| PersistenceError::NotFound {
                    entity: "controller reservation",
                    identity: reservation.as_str().to_owned(),
                })?
                .artifact;
            let remaining = match artifact {
                ControllerReservationDimension::Outstanding(remaining) => Some(remaining),
                ControllerReservationDimension::NotApplicable
                | ControllerReservationDimension::Settled
                    if bytes == 0 =>
                {
                    None
                }
                ControllerReservationDimension::NotApplicable
                | ControllerReservationDimension::Settled => {
                    if self.blocked.is_none() {
                        self.blocked = Some(ControllerAccountBlock::ContractViolation {
                            dimension: "artifact_bytes".to_owned(),
                            reservation: reservation.clone(),
                            observed: bytes,
                            reserved: 0,
                        });
                        self.advance()?;
                    }
                    return Ok(ControllerArtifactChargeOutcome::ContractViolation);
                }
            };
            if let Some(remaining) = remaining
                && bytes > remaining
            {
                if self.blocked.is_none() {
                    self.blocked = Some(ControllerAccountBlock::ContractViolation {
                        dimension: "artifact_bytes".to_owned(),
                        reservation: reservation.clone(),
                        observed: bytes,
                        reserved: remaining,
                    });
                    self.advance()?;
                }
                return Ok(ControllerArtifactChargeOutcome::ContractViolation);
            }
            if let Some(remaining) = remaining {
                let record = self.reservations.get_mut(reservation).ok_or_else(|| {
                    PersistenceError::NotFound {
                        entity: "controller reservation",
                        identity: reservation.as_str().to_owned(),
                    }
                })?;
                record.artifact = ControllerReservationDimension::Outstanding(remaining - bytes);
                self.outstanding.artifact_bytes = self
                    .outstanding
                    .artifact_bytes
                    .checked_sub(bytes)
                    .ok_or_else(|| {
                        PersistenceError::InvalidDocument(
                            "controller artifact outstanding underflow".to_owned(),
                        )
                    })?;
            }
        } else {
            let committed = self.committed_totals()?;
            if checked_add(committed.artifact_bytes, bytes)?
                > self.declaration.budget.artifact_bytes
            {
                return Err(PersistenceError::Bounds {
                    location: "controller.artifact_budget",
                    reason: "logical artifact bytes exceed the controller account remainder"
                        .to_owned(),
                });
            }
        }
        self.settled.artifact_bytes = checked_add(self.settled.artifact_bytes, bytes)?;
        self.advance()?;
        Ok(ControllerArtifactChargeOutcome::Charged)
    }
}

fn admission_value(bound: &AdmissionBound<u64>) -> Option<u64> {
    match bound {
        AdmissionBound::Bounded(value) => Some(*value),
        AdmissionBound::NotApplicable | AdmissionBound::Unknown => None,
    }
}

fn settle_dimension(
    dimension: &str,
    reservation: &ControllerReservationId,
    state: &mut ControllerReservationDimension,
    observed: Option<u64>,
    outstanding: &mut u64,
    settled: &mut u64,
    blocked: &mut Option<ControllerAccountBlock>,
) -> Result<(), PersistenceError> {
    let reserved = match *state {
        ControllerReservationDimension::NotApplicable => {
            if let Some(observed) = observed.filter(|observed| *observed > 0)
                && blocked.is_none()
            {
                *blocked = Some(ControllerAccountBlock::ContractViolation {
                    dimension: dimension.to_owned(),
                    reservation: reservation.clone(),
                    observed,
                    reserved: 0,
                });
            }
            return Ok(());
        }
        ControllerReservationDimension::Outstanding(reserved) => reserved,
        ControllerReservationDimension::Settled => return Ok(()),
    };
    let Some(observed) = observed else {
        if blocked.is_none() {
            *blocked = Some(ControllerAccountBlock::UnknownUsage {
                dimension: dimension.to_owned(),
                reservation: reservation.clone(),
            });
        }
        return Ok(());
    };
    *outstanding = outstanding.checked_sub(reserved).ok_or_else(|| {
        PersistenceError::InvalidDocument("controller outstanding settlement underflow".to_owned())
    })?;
    let charged = observed.min(reserved);
    *settled = checked_add(*settled, charged)?;
    *state = ControllerReservationDimension::Settled;
    if observed > reserved && blocked.is_none() {
        *blocked = Some(ControllerAccountBlock::ContractViolation {
            dimension: dimension.to_owned(),
            reservation: reservation.clone(),
            observed,
            reserved,
        });
    }
    Ok(())
}
