use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use milkdrift_capability::{BoundedJson, ExtensionKey, SchemaId};

use crate::{FieldId, NodeId, PathSelector, PortId, WorkflowId};

use super::ModelError;

const MAX_INTERFACE_FIELDS: usize = 256;
const MAX_METADATA_ENTRIES: usize = 64;

/// Exact schema identity/version used for compatibility checks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaRef {
    id: SchemaId,
    version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaRefWire {
    id: SchemaId,
    version: u32,
}

milkdrift_contracts::deserialize_via!(SchemaRef, SchemaRefWire, |wire| Self::new(
    wire.id,
    wire.version
));

impl SchemaRef {
    /// Creates a reference to a nonzero schema version.
    pub fn new(id: SchemaId, version: u32) -> Result<Self, ModelError> {
        if version == 0 {
            return Err(ModelError::new("schema.version", "must be nonzero"));
        }
        Ok(Self { id, version })
    }

    /// Schema identity.
    #[must_use]
    pub const fn id(&self) -> &SchemaId {
        &self.id
    }

    /// Schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    pub(crate) fn compatible_with(&self, other: &Self) -> bool {
        self == other
    }
}

/// One declared workflow interface field.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceField {
    schema: SchemaRef,
    required: bool,
}

impl InterfaceField {
    /// Creates a required interface field.
    #[must_use]
    pub const fn required(schema: SchemaRef) -> Self {
        Self {
            schema,
            required: true,
        }
    }

    /// Creates an optional interface field.
    #[must_use]
    pub const fn optional(schema: SchemaRef) -> Self {
        Self {
            schema,
            required: false,
        }
    }

    /// Field schema.
    #[must_use]
    pub const fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Whether a caller must supply the field.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.required
    }
}

/// Declared input and output contract of a workflow or subworkflow.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowInterface {
    inputs: BTreeMap<FieldId, InterfaceField>,
    outputs: BTreeMap<FieldId, InterfaceField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowInterfaceWire {
    inputs: BTreeMap<FieldId, InterfaceField>,
    outputs: BTreeMap<FieldId, InterfaceField>,
}

milkdrift_contracts::deserialize_via!(WorkflowInterface, WorkflowInterfaceWire, |wire| Self::new(
    wire.inputs,
    wire.outputs
));

impl WorkflowInterface {
    /// Constructs a bounded workflow interface.
    pub fn new(
        inputs: impl IntoIterator<Item = (FieldId, InterfaceField)>,
        outputs: impl IntoIterator<Item = (FieldId, InterfaceField)>,
    ) -> Result<Self, ModelError> {
        let inputs = collect_interface_fields("interface.inputs", inputs)?;
        let outputs = collect_interface_fields("interface.outputs", outputs)?;
        if inputs.len() > MAX_INTERFACE_FIELDS || outputs.len() > MAX_INTERFACE_FIELDS {
            return Err(ModelError::new(
                "interface",
                format!("at most {MAX_INTERFACE_FIELDS} inputs and outputs are allowed"),
            ));
        }
        Ok(Self { inputs, outputs })
    }

    /// Input fields.
    #[must_use]
    pub const fn inputs(&self) -> &BTreeMap<FieldId, InterfaceField> {
        &self.inputs
    }

    /// Output fields.
    #[must_use]
    pub const fn outputs(&self) -> &BTreeMap<FieldId, InterfaceField> {
        &self.outputs
    }
}

fn collect_interface_fields(
    location: &'static str,
    fields: impl IntoIterator<Item = (FieldId, InterfaceField)>,
) -> Result<BTreeMap<FieldId, InterfaceField>, ModelError> {
    let mut collected = BTreeMap::new();
    for (field, definition) in fields {
        if collected.insert(field.clone(), definition).is_some() {
            return Err(ModelError::new(
                location,
                format!("duplicate interface field `{field}`"),
            ));
        }
        if collected.len() > MAX_INTERFACE_FIELDS {
            return Err(ModelError::new(
                location,
                format!("at most {MAX_INTERFACE_FIELDS} fields are allowed"),
            ));
        }
    }
    Ok(collected)
}

/// Bounded descriptive metadata excluded from execution state but included in semantic identity.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlueprintMetadata {
    name: String,
    description: String,
    labels: BTreeSet<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintMetadataWire {
    name: String,
    description: String,
    labels: BTreeSet<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

milkdrift_contracts::deserialize_via!(BlueprintMetadata, BlueprintMetadataWire, |wire| Self::new(
    wire.name,
    wire.description,
    wire.labels,
    wire.extensions
));

impl BlueprintMetadata {
    /// Creates bounded package metadata.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        labels: BTreeSet<String>,
        extensions: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        let description = description.into();
        if name.is_empty() || name.len() > 160 || description.len() > 4_096 {
            return Err(ModelError::new(
                "metadata",
                "name must contain 1..=160 bytes and description at most 4096 bytes",
            ));
        }
        if labels.len() > MAX_METADATA_ENTRIES
            || labels
                .iter()
                .any(|label| label.is_empty() || label.len() > 96)
            || extensions.len() > MAX_METADATA_ENTRIES
        {
            return Err(ModelError::new(
                "metadata",
                format!("labels/extensions are bounded to {MAX_METADATA_ENTRIES} entries"),
            ));
        }
        let extension_bytes = serde_json::to_vec(&extensions)
            .map_err(|error| ModelError::new("metadata.extensions", error.to_string()))?;
        if extension_bytes.len() > 65_536 {
            return Err(ModelError::new(
                "metadata.extensions",
                "serialized extensions exceed 65536 bytes",
            ));
        }
        Ok(Self {
            name,
            description,
            labels,
            extensions,
        })
    }

    /// Human-facing blueprint name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Bounded human-facing description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Deterministically ordered labels.
    #[must_use]
    pub const fn labels(&self) -> &BTreeSet<String> {
        &self.labels
    }

    /// Bounded namespaced semantic extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
    }

    pub(crate) fn default_for(workflow: &WorkflowId) -> Result<Self, ModelError> {
        Self::new(workflow.as_str(), "", BTreeSet::new(), BTreeMap::new())
    }
}

/// Source bound to a node data input or safe condition operand.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum BindingSource {
    /// Small literal structured value.
    Literal {
        /// Bounded structured literal.
        value: BoundedJson,
    },
    /// Declared workflow input.
    WorkflowInput {
        /// Interface input identity.
        field: FieldId,
    },
    /// Output of a causally upstream node with an optional safe path.
    NodeOutput {
        /// Source node.
        node: NodeId,
        /// Source data port.
        port: PortId,
        /// Safe selector within the output value.
        path: PathSelector,
    },
    /// Durable branch-local workspace value contract.
    WorkspaceValue {
        /// Opaque durable value reference.
        reference: String,
        /// Exact declared value schema.
        contract: SchemaRef,
    },
    /// Durable artifact contract reference.
    Artifact {
        /// Opaque durable artifact reference.
        reference: String,
        /// Exact declared artifact value schema.
        contract: SchemaRef,
    },
    /// Parameter supplied to an explicitly pinned subworkflow.
    SubworkflowParameter {
        /// Pinned subworkflow parameter identity.
        field: FieldId,
    },
}

impl BindingSource {
    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::WorkspaceValue { reference, .. } | Self::Artifact { reference, .. }
                if reference.is_empty()
                    || reference.len() > milkdrift_capability::MAX_DURABLE_REFERENCE_BYTES =>
            {
                Err(ModelError::new(
                    "binding.reference",
                    format!(
                        "must contain 1..={} bytes",
                        milkdrift_capability::MAX_DURABLE_REFERENCE_BYTES
                    ),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(super) enum PortDirection {
    Input,
    Output,
}

/// Declared data port; constructors prevent input/output contradictions.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataPort {
    schema: SchemaRef,
    required: bool,
    binding: Option<BindingSource>,
    direction: PortDirection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DataPortWire {
    schema: SchemaRef,
    required: bool,
    binding: Option<BindingSource>,
    direction: PortDirection,
}

milkdrift_contracts::deserialize_via!(DataPort, DataPortWire, |wire| {
    match wire.direction {
        PortDirection::Input => Self::input(wire.schema, wire.required, wire.binding),
        PortDirection::Output if !wire.required && wire.binding.is_none() => {
            Ok(Self::output(wire.schema))
        }
        PortDirection::Output => Err(ModelError::new(
            "port.direction",
            "an output cannot be required or carry an input binding",
        )),
    }
});

impl DataPort {
    /// Creates an input port with an optional explicit binding.
    pub fn input(
        schema: SchemaRef,
        required: bool,
        binding: Option<BindingSource>,
    ) -> Result<Self, ModelError> {
        if let Some(binding) = &binding {
            binding.validate()?;
        }
        Ok(Self {
            schema,
            required,
            binding,
            direction: PortDirection::Input,
        })
    }

    /// Creates an output port. Outputs never carry input bindings.
    #[must_use]
    pub const fn output(schema: SchemaRef) -> Self {
        Self {
            schema,
            required: false,
            binding: None,
            direction: PortDirection::Output,
        }
    }

    /// Port schema.
    #[must_use]
    pub const fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Input binding, when this is an input.
    #[must_use]
    pub const fn binding(&self) -> Option<&BindingSource> {
        self.binding.as_ref()
    }

    /// Whether this input must resolve before its owning node may execute.
    ///
    /// Output ports always return `false` because their constructor cannot carry
    /// an input requirement.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.required
    }

    pub(super) fn ensure_direction(&self, expected: PortDirection) -> Result<(), ModelError> {
        if self.direction != expected {
            return Err(ModelError::new(
                "port.direction",
                "input and output port constructors cannot be interchanged",
            ));
        }
        Ok(())
    }
}
