//! Immutable structural boundaries for prospective method adaptation.
//!
//! An agreement identifies the enclosing program independently of the tasks an author may
//! replace. The protected fingerprint includes interfaces, metadata, every enclosing node,
//! and every edge crossing that boundary. Task arguments inside the region remain untrusted;
//! structural preservation alone never establishes result acceptance or effect permission.

use milkdrift_capability::CapabilityRequirement;
use serde::{Deserialize, Serialize};

use crate::{BlueprintRevision, ModelError, NodeId, NodeKind, SemanticBlueprint};

/// Current independently identified agreement definition format.
pub const AGREEMENT_SCHEMA_VERSION: u32 = 1;

/// The region in which a method may add investigation and replace future task work.
///
/// Node identities use a reserved prefix, ending in `.`. Enclosing nodes, including terminals,
/// branches, verifiers and effects, cannot use that prefix. Every editable task must select
/// one of the exact declared requirement envelopes; authority still checks the actual capability.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AdaptationScope {
    node_prefix: String,
    maximum_nodes: u16,
    maximum_revisions: u16,
    requirements: Vec<CapabilityRequirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWire {
    node_prefix: String,
    maximum_nodes: u16,
    maximum_revisions: u16,
    requirements: Vec<CapabilityRequirement>,
}

milkdrift_contracts::deserialize_via!(AdaptationScope, ScopeWire, |wire| {
    Self::new(wire.node_prefix, wire.maximum_nodes, wire.requirements)
        .and_then(|scope| scope.with_maximum_revisions(wire.maximum_revisions))
});

impl AdaptationScope {
    /// Read an author-supplied scope with the same duplicate and size refusals as a revision.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ModelError> {
        if bytes.len() > 65_536 {
            return Err(invalid("adaptation scope exceeds 65536 bytes"));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)
            .map_err(|error| invalid(&error.to_string()))?;
        serde_json::from_value(value).map_err(|error| invalid(&error.to_string()))
    }

    /// Establish an explicit bounded task region. This does not grant invocation authority.
    pub fn new(
        node_prefix: String,
        maximum_nodes: u16,
        requirements: Vec<CapabilityRequirement>,
    ) -> Result<Self, ModelError> {
        if node_prefix.len() > 96
            || !node_prefix.ends_with('.')
            || NodeId::new(&node_prefix).is_err()
            || maximum_nodes == 0
            || usize::from(maximum_nodes) > crate::model::MAX_NODES
            || requirements.is_empty()
            || requirements.len() > 32
            || requirements
                .iter()
                .enumerate()
                .any(|(index, value)| requirements[..index].contains(value))
        {
            return Err(invalid("invalid bounded adaptation scope"));
        }
        Ok(Self {
            node_prefix,
            maximum_nodes,
            maximum_revisions: 32,
            requirements,
        })
    }

    /// Set the cumulative revision ceiling for the accepted run. It never resets on restart.
    pub fn with_maximum_revisions(mut self, maximum: u16) -> Result<Self, ModelError> {
        if maximum == 0 || maximum > 1024 {
            return Err(invalid("adaptation revisions must be between 1 and 1024"));
        }
        self.maximum_revisions = maximum;
        Ok(self)
    }

    /// Exact capability envelopes permitted for future editable tasks. Publication validates
    /// their supported implementations before advertising a governed service.
    #[must_use]
    pub fn requirements(&self) -> &[CapabilityRequirement] {
        &self.requirements
    }

    /// Maximum prospective adoptions over the entire accepted governed run.
    #[must_use]
    pub const fn maximum_revisions(&self) -> u16 {
        self.maximum_revisions
    }

    /// Whether this identity belongs to the editable task region.
    #[must_use]
    pub fn contains(&self, node: &NodeId) -> bool {
        node.as_str().starts_with(&self.node_prefix)
    }
}

/// An immutable agreement's structural identity, distinct from its editable method revision.
///
/// Construct this against a complete ungoverned blueprint, then attach it with
/// [`crate::Mutation::SetAgreement`]. A different requirement needs a newly sealed agreement
/// and a new accepted run; adoption cannot change the agreement of an existing run.
/// `effect_policy` names the exact separately owned evidence/target policy. It is a commitment,
/// not an acceptance result. The host must independently enforce that policy at effect entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GoverningAgreement {
    schema_version: u32,
    name: NodeId,
    scope: AdaptationScope,
    protected_digest: String,
    effect_policy: String,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgreementWire {
    schema_version: u32,
    name: NodeId,
    scope: AdaptationScope,
    protected_digest: String,
    effect_policy: String,
    digest: String,
}

milkdrift_contracts::deserialize_via!(GoverningAgreement, AgreementWire, |wire| {
    let agreement = Self {
        schema_version: wire.schema_version,
        name: wire.name,
        scope: wire.scope,
        protected_digest: wire.protected_digest,
        effect_policy: wire.effect_policy,
        digest: wire.digest,
    };
    agreement.validate_identity().map(|()| agreement)
});

impl GoverningAgreement {
    /// Freeze the immutable enclosing program while leaving the declared task region editable.
    /// The supplied policy must be the digest of an operator-owned effect policy, never prose.
    pub fn seal(
        name: NodeId,
        baseline: &BlueprintRevision,
        scope: AdaptationScope,
        effect_policy: String,
    ) -> Result<Self, ModelError> {
        if baseline.semantic().agreement().is_some() {
            return Err(invalid(
                "seal a distinct agreement from an ungoverned definition",
            ));
        }
        let mut agreement = Self {
            schema_version: AGREEMENT_SCHEMA_VERSION,
            name,
            protected_digest: protected_digest(baseline.semantic(), &scope)?,
            scope,
            effect_policy,
            digest: String::new(),
        };
        agreement.digest = agreement.compute_digest()?;
        agreement.validate_identity()?;
        agreement.validate_method(baseline.semantic())?;
        Ok(agreement)
    }

    /// Content-derived identity retained independently of prospective method revisions.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Exact evidence/target policy required by the consequential owner.
    #[must_use]
    pub fn effect_policy(&self) -> &str {
        &self.effect_policy
    }

    /// Declared editable region and requirement choices.
    #[must_use]
    pub const fn scope(&self) -> &AdaptationScope {
        &self.scope
    }

    fn compute_digest(&self) -> Result<String, ModelError> {
        digest(
            "milkdrift.agreement.v1",
            &(
                self.schema_version,
                &self.name,
                &self.scope,
                &self.protected_digest,
                &self.effect_policy,
            ),
        )
    }

    fn validate_identity(&self) -> Result<(), ModelError> {
        if self.schema_version != AGREEMENT_SCHEMA_VERSION
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.effect_policy)
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.protected_digest)
            || self.digest != self.compute_digest()?
        {
            return Err(invalid(
                "unsupported agreement version or inconsistent agreement identity",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_method(&self, method: &SemanticBlueprint) -> Result<(), ModelError> {
        if self.protected_digest != protected_digest(method, &self.scope)? {
            return Err(invalid(
                "protected interface, metadata, node or boundary edge changed",
            ));
        }
        let mut count = 0;
        for node in method
            .nodes()
            .values()
            .filter(|node| self.scope.contains(node.id()))
        {
            count += 1;
            let NodeKind::Task { config } = node.kind() else {
                return Err(invalid(
                    "editable work must be an ordinary task; structured and terminal work is protected",
                ));
            };
            if !self.scope.requirements.contains(config.requirement()) {
                return Err(invalid(
                    "editable task uses an undeclared capability requirement",
                ));
            }
        }
        if count == 0 || count > usize::from(self.scope.maximum_nodes) {
            return Err(invalid("editable task count is outside the accepted scope"));
        }
        Ok(())
    }
}

/// Check preservation below proposal classification, including direct runtime adoption.
///
/// An agreement cannot be introduced, removed or replaced in an already accepted run. A new
/// agreement requires a separate run so earlier failures cannot acquire new compliance meaning.
pub fn validate_agreement_adoption(
    old: &BlueprintRevision,
    new: &BlueprintRevision,
) -> Result<(), ModelError> {
    if old.semantic().agreement() != new.semantic().agreement() {
        return Err(invalid(
            "an accepted run cannot change its governing agreement",
        ));
    }
    if let Some(agreement) = old.semantic().agreement() {
        agreement.validate_method(new.semantic())?;
    }
    Ok(())
}

fn protected_digest(
    method: &SemanticBlueprint,
    scope: &AdaptationScope,
) -> Result<String, ModelError> {
    let nodes: Vec<_> = method
        .nodes()
        .values()
        .filter(|n| !scope.contains(n.id()))
        .collect();
    let edges: Vec<_> = method
        .edges()
        .values()
        .filter(|e| !scope.contains(e.source_node()) || !scope.contains(e.target_node()))
        .collect();
    if nodes.is_empty() {
        return Err(invalid(
            "agreement requires an immutable enclosing boundary",
        ));
    }
    digest(
        "milkdrift.agreement.protected.v1",
        &(
            method.workflow(),
            method.blueprint(),
            method.metadata(),
            method.interface(),
            nodes,
            edges,
        ),
    )
}

fn digest(value_domain: &str, value: &impl Serialize) -> Result<String, ModelError> {
    let bytes =
        crate::document::canonical_value_bytes(value).map_err(|e| invalid(&e.to_string()))?;
    let mut hash = blake3::Hasher::new();
    hash.update(value_domain.as_bytes());
    hash.update(&[0]);
    hash.update(&bytes);
    Ok(format!("b3_{}", hash.finalize()))
}

fn invalid(message: &str) -> ModelError {
    ModelError::new("agreement", message)
}
