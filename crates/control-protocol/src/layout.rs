use super::*;

/// Presentation-only layout coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPoint {
    /// Horizontal canvas coordinate.
    pub x: f64,
    /// Vertical canvas coordinate.
    pub y: f64,
    /// Optional width.
    pub width: Option<f64>,
    /// Optional height.
    pub height: Option<f64>,
}

/// Optional viewport preference.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutViewport {
    /// Horizontal pan.
    pub x: f64,
    /// Vertical pan.
    pub y: f64,
    /// Positive zoom factor.
    pub zoom: f64,
}

/// Independent versioned presentation layout.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutDocument {
    /// Layout schema version, independent from the control protocol.
    pub schema_version: u32,
    /// Workflow association.
    pub workflow_id: String,
    /// Exact revision association.
    pub revision_id: String,
    /// Optimistic update generation.
    pub generation: u64,
    /// Bounded author reference supplied from authenticated context on write.
    pub author: String,
    /// Digest over the complete document with an empty digest field.
    pub digest: String,
    /// Node positions/dimensions keyed by semantic node identity.
    pub nodes: BTreeMap<String, LayoutPoint>,
    /// Collapsed presentation group identities.
    pub collapsed_groups: BTreeSet<String>,
    /// Short non-executable annotations keyed by stable presentation identity.
    pub annotations: BTreeMap<String, String>,
    /// Optional canvas viewport preference.
    pub viewport: Option<LayoutViewport>,
}

impl LayoutDocument {
    /// Validates associations, finite coordinates, counts, byte size, and digest.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != LAYOUT_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedMajor {
                found: u16::try_from(self.schema_version).unwrap_or(u16::MAX),
                supported: u16::try_from(LAYOUT_SCHEMA_VERSION).unwrap_or(u16::MAX),
            });
        }
        validate_identifier("layout.workflow_id", &self.workflow_id, 192)?;
        validate_identifier("layout.revision_id", &self.revision_id, 192)?;
        validate_identifier("layout.author", &self.author, 192)?;
        if self.generation == 0
            || self.nodes.len() > 4_096
            || self.collapsed_groups.len() > 1_024
            || self.annotations.len() > 1_024
        {
            return Err(ProtocolError::Bounds(
                "layout generation/count bounds are invalid".to_owned(),
            ));
        }
        for (node, point) in &self.nodes {
            validate_identifier("layout.node", node, 192)?;
            if !point.x.is_finite()
                || !point.y.is_finite()
                || point
                    .width
                    .is_some_and(|value| !value.is_finite() || value <= 0.0)
                || point
                    .height
                    .is_some_and(|value| !value.is_finite() || value <= 0.0)
            {
                return Err(ProtocolError::InvalidContract(
                    "layout coordinates must be finite and dimensions positive".to_owned(),
                ));
            }
        }
        for (identity, annotation) in &self.annotations {
            validate_identifier("layout.annotation", identity, 192)?;
            if annotation.len() > 4_096 {
                return Err(ProtocolError::Bounds(
                    "layout annotation exceeds 4096 bytes".to_owned(),
                ));
            }
        }
        if let Some(viewport) = self.viewport
            && (!viewport.x.is_finite()
                || !viewport.y.is_finite()
                || !viewport.zoom.is_finite()
                || !(0.01..=100.0).contains(&viewport.zoom))
        {
            return Err(ProtocolError::InvalidContract(
                "layout viewport is invalid".to_owned(),
            ));
        }
        let encoded = encode_json(self)?;
        if encoded.len() > MAX_LAYOUT_BYTES {
            return Err(ProtocolError::Bounds(format!(
                "layout exceeds {MAX_LAYOUT_BYTES} bytes"
            )));
        }
        let expected = self.computed_digest()?;
        if self.digest != expected {
            return Err(ProtocolError::InvalidContract(
                "layout digest does not match its content".to_owned(),
            ));
        }
        Ok(())
    }

    /// Computes the domain-separated content digest without semantic blueprint data.
    pub fn computed_digest(&self) -> Result<String, ProtocolError> {
        let mut unsigned = self.clone();
        unsigned.digest.clear();
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.layout.v1\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }

    /// Replaces the digest with the value computed from current content.
    pub fn seal(mut self) -> Result<Self, ProtocolError> {
        self.digest = self.computed_digest()?;
        self.validate()?;
        Ok(self)
    }
}
