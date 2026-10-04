//! Resolve ordinary run inputs through authenticated artifact reads before run creation.

use super::super::{ActorSession, Owner, PublicFailure, invalid};
use milkdrift_control_protocol::RunInput;
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, ContentDigest, MediaType, ValueKey, WorkspaceBudget,
    WorkspaceScope, WorkspaceValue, WorkspaceValueEntry,
};

pub(super) fn resolve(
    owner: &Owner,
    session: &ActorSession,
    revision: &str,
    root: &WorkspaceScope,
    budget: &WorkspaceBudget,
    inputs: &[RunInput],
) -> Result<Vec<WorkspaceValueEntry>, PublicFailure> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    // Use the normal revision read so input inspection cannot disclose a hidden definition.
    let revision = owner.revision(session, revision)?;
    let (_, definition) = milkdrift_blueprint::BlueprintRevisionDocument::from_json(
        &serde_json::to_vec(&revision.document).map_err(|_| invalid("invalid definition"))?,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    let mut names = std::collections::BTreeSet::new();
    let mut usage = milkdrift_workspace::WorkspaceUsage::default();
    let mut artifacts = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for input in inputs {
        let key = ValueKey::new(&input.name).map_err(|error| invalid(&error.to_string()))?;
        if !names.insert(&input.name) {
            return Err(invalid("run input names must be distinct"));
        }
        let field = definition
            .semantic()
            .interface()
            .inputs()
            .iter()
            .find(|(name, _)| name.as_str() == input.name)
            .map(|(_, field)| field)
            .ok_or_else(|| invalid("run input is not declared by the workflow"))?;
        if field.schema().id().as_str() != "milkdrift.artifact-reference"
            || field.schema().version() != 1
        {
            return Err(invalid(
                "ordinary supplied inputs require an artifact-reference v1 interface field",
            ));
        }
        let metadata = owner.artifact_metadata(session, &input.artifact_id)?;
        let reference = ArtifactReference::new(
            ArtifactId::new(&metadata.artifact_id).map_err(|error| invalid(&error.to_string()))?,
            ContentDigest::from_hex(&metadata.digest)
                .map_err(|error| invalid(&error.to_string()))?,
            MediaType::new(&metadata.content_type).map_err(|error| invalid(&error.to_string()))?,
            metadata.size,
        );
        if artifacts.insert(reference.clone()) {
            usage = budget
                .admit_artifact_reference(&usage, &reference)
                .map_err(|error| invalid(&error.to_string()))?;
        }
        let value = WorkspaceValue::Artifact(reference);
        usage = budget
            .admit_value(&usage, &value)
            .map_err(|error| invalid(&error.to_string()))?;
        let mut hash = blake3::Hasher::new();
        let mut offset = 0;
        loop {
            let chunk =
                owner.artifact_range(session, &input.artifact_id, offset, 65_536, "run-input")?;
            if chunk.metadata != metadata || chunk.offset != offset {
                return Err(invalid("run input metadata changed during admission"));
            }
            hash.update(&chunk.bytes);
            offset += chunk.bytes.len() as u64;
            if chunk.end {
                break;
            }
            if chunk.bytes.is_empty() || offset > metadata.size {
                return Err(invalid("run input content is incomplete"));
            }
        }
        if offset != metadata.size || hash.finalize().to_hex().as_str() != metadata.digest {
            return Err(invalid(
                "run input content contradicts its immutable reference",
            ));
        }
        result.push(WorkspaceValueEntry::initial(
            root.reference().clone(),
            key,
            value,
        ));
    }
    Ok(result)
}
