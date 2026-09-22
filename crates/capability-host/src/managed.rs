//! One semantic owner for persistent installations, shared by administration and invocations.
//!
//! A command first commits exact identities and closes affected admission. Platform calls then
//! advance a bounded transition. The immutable receipt acknowledges that intent; inspection reports
//! its later success or uncertainty. Recovery inspects the saved identities, never a matching name.

mod adapter;
pub use adapter::{ManagedLifecycleAdapter, managed_lifecycle_descriptor};

use milkdrift_authority::{
    AuthorityEvaluator, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
    CapabilityExecutionRequirements, DecisionId,
};
use milkdrift_capability::managed::{
    MANAGED_SCHEMA_VERSION, ManagedAction, ManagedName, ManagedRequest, ManagedResponse,
    RecipeReference,
};
use milkdrift_capability::{CapabilityId, OperationId, SideEffectClass};
use milkdrift_persistence::{
    PageSize, PersistenceError,
    managed::{
        ApprovedSetup, InstallationRecord, ManagedChange, ManagedChangePhase, ManagedObservation,
        ManagedResourceStore, ManagedStep, ManagedUse, QuiescenceEvidence,
    },
};
use milkdrift_runtime::BoundaryClock;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, Weak},
};
use thiserror::Error;

/// Lifecycle refusal or uncertain boundary, without mechanism-specific error types.
#[derive(Debug, Error)]
pub enum ManagedError {
    /// Invalid or unsupported configuration/request; no broader fallback is allowed.
    #[error("managed request rejected: {0}")]
    Rejected(String),
    /// Current authority does not permit the exact target operation.
    #[error("managed operation is not authorized")]
    Unauthorized,
    /// Guard conflict or unresolved resource use prevents a change.
    #[error("managed resource conflict: {0}")]
    Conflict(String),
    /// Durable state could not be read or committed.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Platform state may have changed; the recorded transition requires inspection/recovery.
    #[error("managed platform requires recovery: {0}")]
    Platform(String),
}

/// Adopted platform mechanism. Implementations validate bounded typed recipe configuration;
/// raw engine/unit administration never crosses this interface as a worker capability.
pub trait ManagedPlatform: Send + Sync {
    /// Compile one exact operator-approved recipe without changing platform resources.
    fn plan(
        &self,
        installation: &ManagedName,
        recipe: &RecipeReference,
        ownership: &str,
        generation: u64,
    ) -> Result<ApprovedSetup, ManagedError>;
    /// Inspect prerequisites and effective state without changing resources.
    fn diagnose(&self, setup: &ApprovedSetup) -> Result<Vec<String>, ManagedError>;
    /// Execute or recover one specifically replay-safe boundary, verifying ownership first.
    fn reconcile(
        &self,
        record: &InstallationRecord,
        step: ManagedStep,
    ) -> Result<ManagedObservation, ManagedError>;
    /// Observe a verified setup without restarting healthy services or rewriting working files.
    fn observe(&self, setup: &ApprovedSetup) -> Result<ManagedObservation, ManagedError>;
    /// Exact host resource permission facts derived from approved configuration, not request text.
    fn requirements(
        &self,
        setup: &ApprovedSetup,
    ) -> Result<CapabilityExecutionRequirements, ManagedError>;
    /// Physically fence one exact owned task; terminal execution outcome remains unchanged.
    fn fence(
        &self,
        setup: &ApprovedSetup,
        usage: &ManagedUse,
    ) -> Result<QuiescenceEvidence, ManagedError>;
}

/// Rebuild the host registry projection from verified durable generations. Publication has no
/// independent authority over resource state and is replayed after restart or a lost response.
pub trait ManagedGenerationPublisher: Send + Sync {
    /// Publish eligible descriptors and drain unavailable generations without discarding permits.
    fn synchronize(&self, record: &InstallationRecord) -> Result<(), ManagedError>;
}

/// Semantic owner. Stores/platforms are supplied by a production composition root.
pub struct ManagedResources {
    store: Arc<dyn ManagedResourceStore>,
    platform: Arc<dyn ManagedPlatform>,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<dyn BoundaryClock>,
    publisher: Option<Weak<dyn ManagedGenerationPublisher>>,
    transitions: Mutex<BTreeSet<ManagedName>>,
}

// Own a driver without holding a mutex across platform calls. Duplicate command acceptance alone
// cannot elect a driver: a concurrent caller can receive the same receipt from the store.
struct TransitionGuard<'a> {
    active: &'a Mutex<BTreeSet<ManagedName>>,
    installation: ManagedName,
}

impl Drop for TransitionGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.installation);
        }
    }
}

impl ManagedResources {
    /// Compose the shared owner; startup verifies inventories before opening execution admission.
    #[must_use]
    pub fn new(
        store: Arc<dyn ManagedResourceStore>,
        platform: Arc<dyn ManagedPlatform>,
        authority: Arc<dyn AuthorityEvaluator>,
        clock: Arc<dyn BoundaryClock>,
    ) -> Self {
        Self {
            store,
            platform,
            authority,
            clock,
            publisher: None,
            transitions: Mutex::new(BTreeSet::new()),
        }
    }

    /// Install the production registry projection before startup recovery.
    #[must_use]
    pub fn with_publisher(mut self, publisher: Weak<dyn ManagedGenerationPublisher>) -> Self {
        self.publisher = Some(publisher);
        self
    }

    fn publish(&self, record: &InstallationRecord) -> Result<(), ManagedError> {
        match &self.publisher {
            None => Ok(()),
            Some(publisher) => publisher
                .upgrade()
                .ok_or_else(|| ManagedError::Conflict("registry owner is unavailable".to_owned()))?
                .synchronize(record),
        }
    }

    fn claim_transition(
        &self,
        installation: &ManagedName,
    ) -> Result<TransitionGuard<'_>, ManagedError> {
        let mut active = self.transitions.lock().map_err(|_| {
            ManagedError::Conflict("resource driver coordination is unavailable".to_owned())
        })?;
        if active.len() >= 32 || !active.insert(installation.clone()) {
            return Err(ManagedError::Conflict(
                "installation driver is busy or management capacity is exhausted".to_owned(),
            ));
        }
        Ok(TransitionGuard {
            active: &self.transitions,
            installation: installation.clone(),
        })
    }

    /// Inspect retained uses and recover only transitions whose original acceptance is still
    /// pending. Uncertain transitions remain closed until an authorized recovery request.
    pub fn recover_startup(&self) -> Result<(), ManagedError> {
        self.store.verify_managed_integrity()?;
        let mut after = None;
        loop {
            let page = self
                .store
                .managed_installations(after.as_ref(), PageSize::new(32)?)?;
            if page.is_empty() {
                break;
            }
            for record in &page {
                let _driver = self.claim_transition(&record.name)?;
                let record = self
                    .store
                    .managed_installation(&record.name)?
                    .ok_or_else(|| {
                        ManagedError::Conflict(
                            "installation disappeared during recovery".to_owned(),
                        )
                    })?;
                if record
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p.phase, ManagedChangePhase::Pending {}))
                {
                    // A verified completion can be recorded after response loss; all effects are
                    // constrained by the previously accepted plan and exact platform owner.
                    self.drive(record.clone())?;
                } else {
                    self.publish(&record)?;
                }
            }
            after = page.last().map(|r| r.name.clone());
        }
        Ok(())
    }

    /// Execute one request under a trusted authenticated caller's exact grant claim. Authorization
    /// is re-evaluated here for both HTTP and in-process adapters; transports cannot widen it.
    pub fn execute(
        &self,
        caller: &AuthorityRequest,
        request: &ManagedRequest,
    ) -> Result<ManagedResponse, ManagedError> {
        request.validate().map_err(rejected)?;
        if let Some(response) = self.replay(caller, request)? {
            return Ok(response);
        }
        let _driver = if matches!(
            request.action,
            ManagedAction::Inspect {} | ManagedAction::Prepare { .. }
        ) {
            None
        } else {
            Some(self.claim_transition(&request.installation)?)
        };
        // A driver may have finished between the first lookup and our claim. Its receipt remains
        // a replay even if another transition has since changed the inventory.
        if let Some(response) = self.replay(caller, request)? {
            return Ok(response);
        }
        self.execute_new(caller, request)
    }

    fn replay(
        &self,
        caller: &AuthorityRequest,
        request: &ManagedRequest,
    ) -> Result<Option<ManagedResponse>, ManagedError> {
        if let Some(receipt) = self
            .store
            .managed_receipt(caller.actor.as_str(), &request.command)?
        {
            let prior = receipt.authorization.request();
            if receipt.request != *request
                || prior.grant != caller.grant
                || prior.grant_revision != caller.grant_revision
                || prior.grant_digest != caller.grant_digest
                || prior.revocation_generation != caller.revocation_generation
                || prior.resources.workflow != caller.resources.workflow
                || prior.resources.run != caller.resources.run
                || prior.resources.workspace_scope != caller.resources.workspace_scope
            {
                return Err(ManagedError::Conflict(
                    "command key is permanently bound to another request, grant or originating scope".to_owned(),
                ));
            }
            let mut replay = prior.clone();
            replay.evaluated_at =
                BoundaryTimeMillis::new(self.clock.now().map_err(rejected)?.get());
            if !self
                .authority
                .evaluate(&replay)
                .map_err(rejected)?
                .is_allowed()
            {
                return Err(ManagedError::Unauthorized);
            }
            return Ok(Some(receipt.response));
        }
        Ok(None)
    }

    fn execute_new(
        &self,
        caller: &AuthorityRequest,
        request: &ManagedRequest,
    ) -> Result<ManagedResponse, ManagedError> {
        let record = self.store.managed_installation(&request.installation)?;
        let existing = record.as_ref().and_then(|r| {
            r.current
                .as_ref()
                .or_else(|| r.pending.as_ref().map(|p| &p.change.candidate))
        });
        let ownership = existing.map(|s| s.ownership.clone()).unwrap_or_else(|| {
            digest(&format!(
                "{}:{}:{}:{}",
                caller.actor, caller.grant_digest, request.installation, request.command
            ))
        });
        let generation = record.as_ref().map_or(1, |r| {
            r.generation.saturating_add(u64::from(matches!(
                request.action,
                ManagedAction::Update { .. }
            )))
        });
        let mut candidate = match &request.action {
            ManagedAction::Prepare { recipe }
            | ManagedAction::Apply { recipe }
            | ManagedAction::Update { recipe, .. } => {
                self.platform
                    .plan(&request.installation, recipe, &ownership, generation.max(1))?
            }
            ManagedAction::Recover {} => record
                .as_ref()
                .and_then(|r| r.pending.as_ref())
                .map(|pending| pending.change.candidate.clone())
                .ok_or_else(|| ManagedError::Conflict("no pending transition".to_owned()))?,
            _ => existing
                .cloned()
                .ok_or_else(|| ManagedError::Rejected("installation does not exist".to_owned()))?,
        };
        let authorization = self.authorize(caller, request, &candidate)?;
        if matches!(request.action, ManagedAction::Inspect {}) {
            let record = record
                .ok_or_else(|| ManagedError::Rejected("installation does not exist".to_owned()))?;
            let mut view = record.view();
            if record.removed {
                return Ok(view);
            }
            match self.platform.observe(&candidate) {
                Ok(observation) => {
                    view.observed_running = Some(observation.running);
                    if record.pending.is_none() && observation.running != record.desired_running {
                        view.state = "drift".to_owned();
                    }
                    view.diagnostics.push(observation.summary);
                }
                Err(error) => {
                    view.state = "drift".to_owned();
                    view.diagnostics.push(bounded(&error.to_string()));
                }
            }
            return Ok(view);
        }
        if matches!(request.action, ManagedAction::Prepare { .. }) {
            let diagnostics = self.platform.diagnose(&candidate)?;
            return Ok(ManagedResponse {
                schema_version: MANAGED_SCHEMA_VERSION,
                installation: request.installation.clone(),
                version: record.as_ref().map_or(0, |r| r.version),
                generation,
                state: "prepared".to_owned(),
                desired_running: None,
                observed_running: None,
                recipe: Some(candidate.recipe),
                resources: candidate.resources,
                blockers: record
                    .as_ref()
                    .map(|r| r.view().blockers)
                    .unwrap_or_default(),
                pending: None,
                capabilities: Vec::new(),
                diagnostics,
            });
        }
        if record.as_ref().map_or(0, |r| r.version) != request.expected_version {
            return Err(ManagedError::Conflict(
                "expected installation version differs".to_owned(),
            ));
        }
        if matches!(
            request.action,
            ManagedAction::Handoff { .. } | ManagedAction::Return { .. }
        ) {
            return Ok(self
                .store
                .transfer_managed_editing(request, &authorization)?
                .response);
        }
        if let ManagedAction::Resolve {
            use_id,
            expected_claim,
        } = &request.action
        {
            let usage = self
                .store
                .managed_use(use_id)?
                .ok_or_else(|| ManagedError::Conflict("use does not exist".to_owned()))?;
            if usage.binding.installation != request.installation || usage.claim != *expected_claim
            {
                return Err(ManagedError::Conflict(
                    "resolution target or claim differs".to_owned(),
                ));
            }
            let receipt = self
                .store
                .begin_managed_resolution(request, &authorization)?;
            let fenced = self
                .store
                .managed_use(use_id)?
                .ok_or_else(|| ManagedError::Conflict("resolution use disappeared".to_owned()))?;
            let evidence = self.platform.fence(&candidate, &fenced)?;
            self.store
                .quiesce_managed_use(use_id, fenced.claim, &evidence)?;
            if fenced.parent.is_none() {
                self.store.release_managed_use(use_id, fenced.claim)?;
            }
            return Ok(receipt.response);
        }
        let previous = record.as_ref().and_then(|r| r.current.as_ref());
        let running = record.as_ref().is_some_and(|r| r.desired_running);
        let owns_service = candidate.resources.iter().any(|r| {
            r.kind == milkdrift_capability::managed::ManagedResourceKind::Service
                && r.ownership == milkdrift_capability::managed::ResourceOwnership::Owned
        });
        let (steps, running) = match &request.action {
            ManagedAction::Apply { .. } if previous.is_some() => {
                if previous.is_none_or(|p| p.recipe != candidate.recipe) {
                    return Err(ManagedError::Conflict(
                        "changed setup requires an explicit update".to_owned(),
                    ));
                }
                candidate = previous
                    .cloned()
                    .ok_or_else(|| ManagedError::Conflict("setup disappeared".to_owned()))?;
                (vec![ManagedStep::Verify], running)
            }
            ManagedAction::Apply { .. } => (
                vec![
                    ManagedStep::Prerequisites,
                    ManagedStep::PrepareStorage,
                    ManagedStep::Configure,
                    ManagedStep::StartService,
                    ManagedStep::Verify,
                ],
                owns_service,
            ),
            ManagedAction::Update {
                allow_interruption, ..
            } => {
                if !allow_interruption {
                    return Err(ManagedError::Rejected(
                        "the maintained setup requires explicit drained interruption".to_owned(),
                    ));
                }
                (
                    vec![
                        ManagedStep::Prerequisites,
                        ManagedStep::StopService,
                        ManagedStep::RemoveConfiguration,
                        ManagedStep::Configure,
                        ManagedStep::StartService,
                        ManagedStep::Verify,
                    ],
                    owns_service,
                )
            }
            ManagedAction::Start {} => (
                vec![
                    ManagedStep::Prerequisites,
                    ManagedStep::Configure,
                    ManagedStep::StartService,
                    ManagedStep::Verify,
                ],
                owns_service,
            ),
            ManagedAction::Stop {} => (
                vec![
                    ManagedStep::StopService,
                    ManagedStep::Configure,
                    ManagedStep::Verify,
                ],
                false,
            ),
            ManagedAction::Remove {} => (
                vec![
                    ManagedStep::StopService,
                    ManagedStep::RemoveConfiguration,
                    ManagedStep::RemoveStorage,
                ],
                false,
            ),
            ManagedAction::Preserve { disposition } => {
                for resource in &mut candidate.resources {
                    resource.disposition = *disposition;
                }
                (vec![ManagedStep::Verify], running)
            }
            ManagedAction::Recover {} => {
                let change = record
                    .as_ref()
                    .and_then(|r| r.pending.as_ref())
                    .map(|p| p.change.clone())
                    .ok_or_else(|| ManagedError::Conflict("no pending transition".to_owned()))?;
                let receipt = self
                    .store
                    .begin_managed_change(request, &authorization, &change)?;
                if let Some(record) = self.store.managed_installation(&request.installation)? {
                    self.drive(record)?;
                }
                return Ok(receipt.response);
            }
            _ => {
                return Err(ManagedError::Rejected(
                    "unsupported lifecycle action".to_owned(),
                ));
            }
        };
        if let Some(previous) = previous {
            // Preservation is a resource decision, not a default re-read from a new recipe.
            if !matches!(request.action, ManagedAction::Preserve { .. }) {
                for resource in &mut candidate.resources {
                    if let Some(prior) = previous.resources.iter().find(|r| r.name == resource.name)
                    {
                        resource.disposition = prior.disposition;
                    }
                }
            }
        }
        let change = ManagedChange {
            identity: digest(
                &serde_json::to_string(&(caller.actor.clone(), request)).map_err(rejected)?,
            ),
            candidate,
            generation: generation.max(1),
            steps,
            removing: matches!(request.action, ManagedAction::Remove {}),
            running,
        };
        let receipt = self
            .store
            .begin_managed_change(request, &authorization, &change)?;
        if let Some(record) = self.store.managed_installation(&request.installation)? {
            self.drive(record)?;
        }
        Ok(receipt.response)
    }

    fn authorize(
        &self,
        caller: &AuthorityRequest,
        request: &ManagedRequest,
        candidate: &ApprovedSetup,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, ManagedError> {
        let mut authority = caller.clone();
        authority.operation = if matches!(
            request.action,
            ManagedAction::Prepare { .. } | ManagedAction::Inspect {}
        ) {
            AuthorityOperation::InspectCapabilityHealth
        } else {
            AuthorityOperation::AdministerCapabilities
        };
        authority.resources.category = Some(milkdrift_capability::CapabilityCategory::Tool);
        authority.resources.provider_profile = None;
        authority.resources.capability_envelope = None;
        authority.resources.execution_trust_class =
            Some(milkdrift_capability::ExecutionTrustClass::Unspecified);
        authority.resources.locality = Some(milkdrift_capability::Locality::Local);
        authority.resources.trust_zone = None;
        authority.resources.trust_zones.clear();
        authority.resources.capability =
            Some(CapabilityId::new(format!("managed.{}", request.installation)).map_err(rejected)?);
        authority.resources.capability_operation =
            Some(OperationId::new(request.operation()).map_err(rejected)?);
        authority.resources.side_effect = if matches!(
            request.action,
            ManagedAction::Prepare { .. } | ManagedAction::Inspect {}
        ) {
            SideEffectClass::ReadOnly
        } else {
            SideEffectClass::NonIdempotentWrite
        };
        let requirements = self.platform.requirements(candidate)?;
        authority.resources.filesystem = requirements.filesystem;
        if matches!(
            request.action,
            ManagedAction::Prepare { .. } | ManagedAction::Inspect {}
        ) {
            authority.resources.filesystem = authority
                .resources
                .filesystem
                .iter()
                .map(|scope| {
                    milkdrift_authority::FilesystemScope::new(
                        scope.root(),
                        std::collections::BTreeSet::from([milkdrift_authority::AccessMode::Read]),
                    )
                })
                .collect::<Result<_, _>>()
                .map_err(rejected)?;
        }
        authority.resources.network_profiles = requirements.network_profiles;
        authority.resources.network_destinations = requirements.network_destinations;
        authority.resources.secrets = requirements.secrets;
        authority.evaluated_at = BoundaryTimeMillis::new(self.clock.now().map_err(rejected)?.get());
        authority.decision = DecisionId::new(format!(
            "decision:{}",
            digest(&serde_json::to_string(&(request, &authority)).map_err(rejected)?)
        ))
        .map_err(rejected)?;
        let decision = self.authority.evaluate(&authority).map_err(rejected)?;
        if !decision.is_allowed() {
            return Err(ManagedError::Unauthorized);
        }
        Ok(decision)
    }

    fn drive(&self, mut record: InstallationRecord) -> Result<(), ManagedError> {
        for _ in 0..16 {
            let Some(pending) = &record.pending else {
                return self.publish(&record);
            };
            if !matches!(pending.phase, ManagedChangePhase::Pending {}) {
                return self.publish(&record);
            }
            let identity = pending.change.identity.clone();
            let index = pending.next_step;
            let step = *pending
                .change
                .steps
                .get(index as usize)
                .ok_or_else(|| ManagedError::Conflict("invalid transition step".to_owned()))?;
            match self.platform.reconcile(&record, step) {
                Ok(observation) => {
                    record = self.store.advance_managed_change(
                        &record.name,
                        &identity,
                        index,
                        observation,
                    )?;
                }
                Err(error) => {
                    self.store.fail_managed_change(
                        &record.name,
                        &identity,
                        index,
                        &bounded(&error.to_string()),
                    )?;
                    // Accepted command replay remains stable. Inspection exposes the partial effect
                    // and recovery requires a separately authorized command.
                    return Ok(());
                }
            }
        }
        if record.pending.is_some() {
            return Err(ManagedError::Conflict(
                "transition exceeded finite step bound".to_owned(),
            ));
        }
        Ok(())
    }
}

fn rejected(error: impl std::fmt::Display) -> ManagedError {
    ManagedError::Rejected(bounded(&error.to_string()))
}
fn bounded(value: &str) -> String {
    milkdrift_contracts::truncate_utf8(value, 512).to_owned()
}
fn digest(value: &str) -> String {
    format!("b3_{}", blake3::hash(value.as_bytes()))
}
