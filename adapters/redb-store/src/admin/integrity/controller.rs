use milkdrift_persistence::{
    ArtifactPublicationId, ControllerAccountId, ControllerArtifactOwner, ControllerTransitionId,
    IntegrityDigest,
};
use milkdrift_workspace::RunId;

use super::super::{
    CONTROLLER_ACCOUNTS, CONTROLLER_ARTIFACT_CHARGES, CONTROLLER_RUN_BINDINGS,
    CONTROLLER_TRANSITIONS, PersistenceError, error,
};
use super::{ScanContext, phase};

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let accounts = read.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
    let bindings = read
        .open_table(CONTROLLER_RUN_BINDINGS)
        .map_err(error::redb)?;
    let transitions = read
        .open_table(CONTROLLER_TRANSITIONS)
        .map_err(error::redb)?;
    let charges = read
        .open_table(CONTROLLER_ARTIFACT_CHARGES)
        .map_err(error::redb)?;

    context.string_bytes(
        phase::CONTROLLER_ACCOUNTS,
        &accounts,
        "controller_accounts",
        |key, bytes| {
            let key = ControllerAccountId::new(key)?;
            let state = crate::controller_account::decode_account(bytes)?;
            if state.declaration().account() != &key {
                return Err(error::corruption(
                    "controller account key disagrees with its declaration",
                ));
            }
            let binding = bindings
                .get(state.declaration().controller_run().as_str())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("controller account has no originating run binding")
                })?;
            if binding.value() != key.as_str() {
                return Err(error::corruption(
                    "controller account originating run is bound elsewhere",
                ));
            }
            Ok(())
        },
    )?;
    context.string_string(
        phase::CONTROLLER_RUN_BINDINGS,
        &bindings,
        "controller_run_bindings",
        |key, value| {
            let _run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid controller-bound run identity: {cause}"))
            })?;
            let account = ControllerAccountId::new(value)?;
            let bytes = accounts
                .get(account.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("controller run binding has no account"))?;
            let state = crate::controller_account::decode_account(bytes.value())?;
            if state.declaration().account() != &account {
                return Err(error::corruption(
                    "controller run binding points to a mismatched declaration",
                ));
            }
            Ok(())
        },
    )?;
    context.string_bytes(
        phase::CONTROLLER_TRANSITIONS,
        &transitions,
        "controller_transitions",
        |key, bytes| {
            let _transition = ControllerTransitionId::new(key)?;
            let digest = std::str::from_utf8(bytes)
                .map_err(|_| error::corruption("controller transition fingerprint is not UTF-8"))?;
            let _digest = IntegrityDigest::new(digest.to_owned())?;
            Ok(())
        },
    )?;
    context.string_bytes(
        phase::CONTROLLER_ARTIFACT_CHARGES,
        &charges,
        "controller_artifact_charges",
        |key, bytes| {
            let publication = ArtifactPublicationId::new(key)?;
            let charge = crate::controller_account::decode_artifact_charge(bytes)?;
            if accounts
                .get(charge.account.as_str())
                .map_err(error::redb)?
                .is_none()
            {
                return Err(error::corruption(
                    "controller artifact charge has no account",
                ));
            }
            let owner = charge.reservation.clone().map_or(
                ControllerArtifactOwner::RunBinding,
                ControllerArtifactOwner::InvocationReservation,
            );
            let run = crate::artifact::validate_controller_charge_publication(
                read,
                &publication,
                &owner,
                charge.bytes,
            )?;
            let binding = bindings
                .get(run.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("controller artifact charge run has no binding")
                })?;
            if binding.value() != charge.account.as_str() {
                return Err(error::corruption(
                    "controller artifact charge disagrees with the publication run binding",
                ));
            }
            Ok(())
        },
    )
}
