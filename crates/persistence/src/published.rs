//! Published definitions and the exact command association retained by an accepted invocation.
//!
//! The publication inventory owns selection for future calls. A local attempt keeps its association
//! in the run journal; a serving operation keeps it in its existing record. Neither association is a
//! second run journal. The referenced create/start receipts remain runtime-owned.

use milkdrift_authority::{ActorRef, AuthorityDecisionSnapshot, GrantDigest, GrantId};
use milkdrift_blueprint::RevisionId;
use milkdrift_capability::{CapabilityDescriptor, CapabilityId, InvocationId, InvocationRequest};
use milkdrift_workspace::{RunId, WorkspaceBudget};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{CommandId, PageSize, PersistenceError};

/// Current published-method and invocation-association format.
pub const PUBLISHED_METHOD_SCHEMA_VERSION: u32 = 1;

/// Explicit service identity, independent of the publisher and invoke-only caller.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedServiceIdentity {
    /// Actor executing the declared internal method.
    pub actor: ActorRef,
    /// Exact configured service grant.
    pub grant: GrantId,
    /// Immutable grant revision.
    pub grant_revision: u64,
    /// Immutable grant content digest.
    pub grant_digest: GrantDigest,
    /// Revocation epoch accepted by this publication.
    pub revocation_generation: u64,
}

/// A finite input contract prevents an invocation from becoming an administrative proxy.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublishedInput {
    /// One of the explicitly reviewed JSON values, mapped to the same workflow input name.
    Choice {
        /// Nonempty finite allowed values; objects are compared in canonical JSON order.
        values: Vec<milkdrift_capability::BoundedJson>,
    },
    /// Exact readable artifact, with content verified before child creation.
    Artifact {
        /// Required content type.
        media_type: String,
        /// Per-input byte ceiling.
        maximum_bytes: u64,
    },
}

/// Only these accepted node outputs may cross the public boundary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedOutput {
    /// Declared terminal workflow field. Only the run's accepted terminal references qualify.
    pub field: milkdrift_workspace::ValueKey,
    /// Required public content type; inline JSON uses application/json.
    pub media_type: String,
    /// Maximum bytes copied for this output.
    pub maximum_bytes: u64,
}

/// Immutable implementation identity. The descriptor's interface version is independent of its
/// generation: changing the starting revision or service policy always creates a new generation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedMethod {
    /// Exact reader version.
    pub schema_version: u32,
    /// Ordinary capability advertisement, including documentation and interface schemas.
    pub descriptor: CapabilityDescriptor,
    /// Human-readable purpose, input meaning and result limitations, shown in discovery.
    pub documentation: String,
    /// Exact starting definition; the revision owner retains the graph and adaptation policy.
    pub revision: RevisionId,
    /// Expected governing agreement; validated against that revision before publication.
    pub agreement: String,
    /// Narrow internal service grant, never the publisher's ambient grant.
    pub service: PublishedServiceIdentity,
    /// Every public input is required and maps to the identically named workflow input.
    pub inputs: BTreeMap<String, PublishedInput>,
    /// Public name to protected accepted output mapping.
    pub outputs: BTreeMap<String, PublishedOutput>,
    /// Internal workspace and artifact envelope.
    pub workspace_budget: WorkspaceBudget,
    /// Cumulative per-invocation allowance enforced on every internal descendant.
    pub allowance: crate::ControllerResourceBudget,
    /// Maximum accepted, unsettled calls to this generation.
    pub maximum_outstanding: u32,
    /// Inclusive ceiling on the complete publication chain; every ancestor's ceiling also applies.
    pub maximum_depth: u16,
    /// Elapsed invocation ceiling; timeout requests cancellation but does not prove stop.
    pub maximum_duration_ms: u64,
}

impl PublishedMethod {
    /// Validate bounded shape before semantic validation by control and the revision owner.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.schema_version != PUBLISHED_METHOD_SCHEMA_VERSION
            || self.documentation.trim().is_empty()
            || self.documentation.len() > 4096
            || self.service.grant_revision == 0
            || self.maximum_outstanding == 0
            || self.maximum_outstanding > 1024
            || self.maximum_depth == 0
            || self.maximum_depth > milkdrift_capability::MAX_PUBLICATION_DEPTH
            || self.maximum_duration_ms == 0
            || self.descriptor.extensions().keys().any(|key| {
                key.as_str() == milkdrift_capability::PUBLISHED_ALLOWANCE_EXTENSION
                    || key.as_str() == "org.milkdrift/published-method.v1"
            })
            || self.descriptor.category() != &milkdrift_capability::CapabilityCategory::Tool
            || self.inputs.len() > 128
            || self.outputs.len() > 128
            || self.agreement.len() != 67
            || !self.agreement.starts_with("b3_")
        {
            return Err(invalid(
                "invalid published method version, agreement or bounds",
            ));
        }
        crate::ControllerResourceBudget::new(
            self.allowance.cost_micros(),
            self.allowance.currency().clone(),
            self.allowance.input_units(),
            self.allowance.output_units(),
            self.allowance.artifact_bytes(),
            self.allowance.process_admissions(),
            self.allowance.model_admissions(),
        )?;
        self.admission_envelope()?;
        if self.descriptor.locality() != milkdrift_capability::Locality::Local
            || self.descriptor.peer().is_some()
            || self.descriptor.provider_profile().is_some()
        {
            return Err(invalid(
                "a publication must describe its local workflow owner",
            ));
        }
        for (name, rule) in &self.inputs {
            milkdrift_workspace::ValueKey::new(name.clone())
                .map_err(|_| invalid("invalid public input name"))?;
            match rule {
                PublishedInput::Choice { values } if !values.is_empty() && values.len() <= 128 => {
                    for value in values {
                        let encoded = serde_json::to_vec(value)
                            .map_err(|error| invalid(&error.to_string()))?;
                        if encoded.len() > 32_768 {
                            return Err(invalid(
                                "public choice exceeds the canonical child-input bound",
                            ));
                        }
                    }
                }
                PublishedInput::Artifact {
                    media_type,
                    maximum_bytes,
                } if *maximum_bytes > 0
                    && milkdrift_workspace::MediaType::new(media_type).is_ok() => {}
                _ => return Err(invalid("invalid public input constraint")),
            }
        }
        for (name, output) in &self.outputs {
            if output.maximum_bytes == 0
                || milkdrift_workspace::MediaType::new(&output.media_type).is_err()
            {
                return Err(invalid("invalid public output content contract"));
            }
            milkdrift_capability::InputReference::new(
                name.clone(),
                milkdrift_capability::InvocationValueReference::Inline {
                    value: milkdrift_capability::BoundedJson::new(serde_json::Value::Null)
                        .map_err(|_| invalid("invalid output contract"))?,
                },
            )
            .map_err(|_| invalid("invalid public output name"))?;
        }
        // Derived discovery must fit the same bounded descriptor reader before inventory commit.
        self.capability_descriptor()?;
        Ok(())
    }

    /// Conservative internal work and public-copy envelope, derived from the immutable method.
    pub fn admission_envelope(
        &self,
    ) -> Result<milkdrift_capability::InvocationAdmissionEnvelope, PersistenceError> {
        use milkdrift_capability::{
            AdmissionBound, AdmissionMonetaryBound, AdmissionUnit, InvocationAdmissionEnvelope,
            InvocationCounts,
        };
        let budget = &self.allowance;
        let artifacts = self
            .outputs
            .values()
            .try_fold(budget.artifact_bytes(), |total, output| {
                total.checked_add(output.maximum_bytes)
            })
            .ok_or_else(|| invalid("published artifact bound overflow"))?;
        let money = match budget.currency() {
            Some(currency) => AdmissionBound::Bounded(
                AdmissionMonetaryBound::new(budget.cost_micros(), currency.as_str())
                    .map_err(|error| invalid(&error.to_string()))?,
            ),
            None => AdmissionBound::NotApplicable,
        };
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionUnit::ModelTokens,
            AdmissionBound::Bounded(budget.input_units()),
            AdmissionBound::Bounded(budget.output_units()),
            AdmissionBound::Bounded(artifacts),
            money,
        )
        .with_nested_invocations(InvocationCounts::new(
            budget.process_admissions(),
            budget.model_admissions(),
        )))
    }

    /// Ordinary catalog descriptor with the exact derived allowance for remote reservation.
    pub fn capability_descriptor(&self) -> Result<CapabilityDescriptor, PersistenceError> {
        let mut descriptor =
            serde_json::to_value(&self.descriptor).map_err(|error| invalid(&error.to_string()))?;
        descriptor["extensions"]["org.milkdrift/published-method.v1"] = serde_json::json!({
            "documentation": self.documentation, "inputs": self.inputs,
            "outputs": self.outputs.iter().map(|(name, output)| (name, serde_json::json!({
                "media_type": output.media_type, "maximum_bytes": output.maximum_bytes
            }))).collect::<BTreeMap<_, _>>()
        });
        descriptor["extensions"][milkdrift_capability::PUBLISHED_ALLOWANCE_EXTENSION] =
            serde_json::to_value(self.admission_envelope()?)
                .map_err(|error| invalid(&error.to_string()))?;
        serde_json::from_value(descriptor).map_err(|error| invalid(&error.to_string()))
    }

    /// Canonical implementation digest, independent of catalog health and retirement.
    pub fn digest(&self) -> Result<String, PersistenceError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| invalid(&e.to_string()))?;
        let mut hash = blake3::Hasher::new_derive_key("milkdrift.published-method.v1");
        hash.update(&bytes);
        Ok(format!("b3_{}", hash.finalize().to_hex()))
    }
}

/// Current admission state for one immutable published generation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedMethodRecord {
    /// Exact canonical publication request, including its command identity and caller grant.
    pub publication_request: crate::IntegrityDigest,
    /// Original predecessor guard, retained across promotion and retirement.
    pub expected_previous_version: Option<u64>,
    /// Exact successful retirement request. The prior version is always one lower than `version`.
    pub retirement_request: Option<crate::IntegrityDigest>,
    /// Immutable method and service contract.
    pub method: PublishedMethod,
    /// Monotonic state revision for promotion/retirement guards.
    pub version: u64,
    /// Retired generations remain available to already accepted invocations.
    pub retired: bool,
    /// Exact authorized publication decision.
    pub authorization: AuthorityDecisionSnapshot,
}

/// Authoritative owner of a published invocation; transport does not change workflow origin.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublishedInvocationSource {
    /// A local attempt, retained only in its runtime journal.
    Local {
        /// Owning run.
        run: RunId,
        /// Exact immutable attempt.
        attempt: crate::AttemptId,
    },
    /// A direct or incoming peer operation in the ordinary serving store.
    Serving {
        /// Authenticated caller namespace.
        caller: milkdrift_peer_protocol::ServingCaller,
        /// Exact accepted serving execution.
        execution: milkdrift_peer_protocol::PeerExecutionId,
    },
}

/// A planned real run. This fact must be durable before the continuation port creates anything.
/// Commands contain canonical runtime documents, so reply loss replays the exact request bytes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedInvocationPlan {
    /// Existing authoritative acceptance owner.
    pub source: PublishedInvocationSource,
    /// Exact reader version.
    pub schema_version: u32,
    /// Host-scoped invocation identity.
    pub invocation: InvocationId,
    /// Exact public request, including supplied input bytes/references.
    pub request: InvocationRequest,
    /// Selected public capability.
    pub capability: CapabilityId,
    /// Exact selected generation.
    pub generation: u64,
    /// Immutable implementation digest.
    pub method_digest: String,
    /// Reserved child identity; never replaced during recovery.
    pub child_run: RunId,
    /// Immutable internal allowance, derived from this exact method and invocation.
    pub allowance: crate::ControllerAccountDeclaration,
    /// Canonical create command saved before child creation.
    pub create_command: String,
    /// Canonical start command saved before child creation.
    pub start_command: String,
    /// Stable cancellation command identity; its exact request is retained by runtime's receipt.
    pub cancel_command: CommandId,
    /// Service grant selected by publication.
    pub service: PublishedServiceIdentity,
    /// Original accepted caller relationship, retained for revocation and disclosure checks.
    pub caller: AuthorityDecisionSnapshot,
    /// Selected method's depth ceiling, retained for later descendant admission.
    pub maximum_depth: u16,
    /// Accepted publication ancestry, preserving recursion and depth restrictions across hosts.
    pub ancestry: Vec<milkdrift_capability::PublicationAncestor>,
    /// Public deadline; observation and cancellation may continue beyond it.
    pub deadline_unix_ms: u64,
}

impl PublishedInvocationPlan {
    /// The enclosing fact propagated when this invocation's internal work calls another method.
    pub fn ancestor(&self) -> Result<milkdrift_capability::PublicationAncestor, PersistenceError> {
        milkdrift_capability::PublicationAncestor::new(
            self.capability.clone(),
            self.generation,
            self.maximum_depth,
        )
        .map_err(|error| invalid(&error.to_string()))
    }

    /// Stable resource handoff association; neither a guessed child name nor a new journal.
    #[must_use]
    pub fn association_id(&self) -> String {
        format!("published:{}:{}", self.method_digest, self.invocation)
    }

    /// Checks bounded linkage shape; runtime validates commands before accepting the child.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.allowance.validate()?;
        if self.allowance.controller_run() != &self.child_run
            || self.allowance.published_invocation() != Some(&self.invocation)
            || self.allowance.policy_digest() != self.method_digest
            || self.schema_version != PUBLISHED_METHOD_SCHEMA_VERSION
            || self.generation == 0
            || self.request.invocation() != &self.invocation
            || self.request.capability() != &self.capability
            || self.create_command.is_empty()
            || self.start_command.is_empty()
            || self.create_command.len() > 65_536
            || self.start_command.len() > 65_536
            || self.service.grant_revision == 0
            || self.caller.request().resources.capability.as_ref() != Some(&self.capability)
            || self
                .caller
                .request()
                .resources
                .capability_operation
                .as_ref()
                != Some(self.request.operation())
            || self.caller.request().provenance.descriptor_revision != Some(self.generation)
            || self.request.operation().as_str() != "method.invoke"
            || self.maximum_depth == 0
            || self.maximum_depth > milkdrift_capability::MAX_PUBLICATION_DEPTH
            || self.ancestry.len() >= usize::from(self.maximum_depth)
            || self.ancestry.iter().any(|ancestor| {
                ancestor.capability() == &self.capability
                    || self.ancestry.len() >= usize::from(ancestor.maximum_depth())
            })
            || !self.caller.is_allowed()
            || self.deadline_unix_ms <= self.caller.request().evaluated_at.get()
        {
            return Err(invalid("invalid published invocation association"));
        }
        Ok(())
    }
}

/// Read the caller-owned invocation association and its derived pending index.
pub trait PublishedInvocationStore: Send + Sync {
    /// Verify pending membership during runtime recovery; a missing derived row must not hide work.
    fn published_local_pending(
        &self,
        source: &PublishedInvocationSource,
    ) -> Result<bool, PersistenceError>;
    /// Page pending local associations by their derived index. The index references journal facts;
    /// it never replaces the attempt's history. The cursor must be a local source returned here.
    fn published_local_page(
        &self,
        after: Option<&PublishedInvocationSource>,
        limit: PageSize,
    ) -> Result<
        (
            Vec<PublishedInvocationPlan>,
            Option<PublishedInvocationSource>,
        ),
        PersistenceError,
    >;

    /// Retrieve an association from its authoritative caller record, never by an untrusted child name.
    fn published_invocation(
        &self,
        source: &PublishedInvocationSource,
    ) -> Result<Option<PublishedInvocationPlan>, PersistenceError>;
}

/// Publication inventory, without a competing workflow or invocation ledger.
pub trait PublishedMethodStore: PublishedInvocationStore {
    /// Insert an exact validated generation, comparing the previous generation's state revision.
    fn publish_method(
        &self,
        method: &PublishedMethod,
        expected_previous_version: Option<u64>,
        authorization: &AuthorityDecisionSnapshot,
        request: &crate::IntegrityDigest,
    ) -> Result<PublishedMethodRecord, PersistenceError>;
    /// Read an exact generation, including retired generations.
    fn published_method(
        &self,
        capability: &CapabilityId,
        generation: u64,
    ) -> Result<Option<PublishedMethodRecord>, PersistenceError>;
    /// Page by exact capability/generation key; no lifetime-sized inventory load.
    fn published_methods(
        &self,
        after: Option<(&CapabilityId, u64)>,
        limit: PageSize,
    ) -> Result<Vec<PublishedMethodRecord>, PersistenceError>;
    /// Close new admission, preserving immutable implementation and accepted work.
    fn retire_method(
        &self,
        capability: &CapabilityId,
        generation: u64,
        expected_version: u64,
        authorization: &AuthorityDecisionSnapshot,
        request: &crate::IntegrityDigest,
    ) -> Result<PublishedMethodRecord, PersistenceError>;
}

fn invalid(message: &str) -> PersistenceError {
    PersistenceError::InvalidDocument(message.to_owned())
}
