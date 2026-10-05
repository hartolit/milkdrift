//! Generated edge identities bind field roles without treating legal name characters as separators.

use super::BuildResult;
use milkdrift_blueprint::{Edge, EdgeId, EdgeKind};
use std::collections::BTreeMap;

pub(super) fn current(
    kind: EdgeKind,
    source: &str,
    port: &str,
    target: &str,
    input: &str,
) -> BuildResult<EdgeId> {
    let tag = match kind {
        EdgeKind::Control => "control",
        EdgeKind::Data => "data",
    };
    // The fixed string array is the encoding contract: domain, kind, source node/port, target
    // node/port. JSON string boundaries preserve colons and every other accepted name byte.
    let bytes =
        serde_json::to_vec(&["milkdrift.author.edge.v2", tag, source, port, target, input])?;
    Ok(EdgeId::new(format!("author.{}", blake3::hash(&bytes)))?)
}

fn legacy(edge: &Edge) -> BuildResult<EdgeId> {
    let tag = match edge.kind() {
        EdgeKind::Control => "Control",
        EdgeKind::Data => "Data",
    };
    Ok(EdgeId::new(format!(
        "author.{}",
        blake3::hash(
            format!(
                "{tag}:{}:{}:{}:{}",
                edge.source_node(),
                edge.source_port(),
                edge.target_node(),
                edge.target_port()
            )
            .as_bytes()
        )
    ))?)
}

pub(super) fn recognize<'a>(
    edges: impl Iterator<Item = &'a Edge>,
) -> BuildResult<BTreeMap<EdgeId, EdgeId>> {
    let mut retained = BTreeMap::new();
    for edge in edges {
        let canonical = current(
            edge.kind(),
            edge.source_node().as_str(),
            edge.source_port().as_str(),
            edge.target_node().as_str(),
            edge.target_port().as_str(),
        )?;
        if edge.id() != &canonical && edge.id() != &legacy(edge)? {
            return Err("unsupported editor edge identity".into());
        }
        if retained.insert(canonical, edge.id().clone()).is_some() {
            return Err("unsupported duplicate editor connection".into());
        }
    }
    // Reusing only recognized IDs does not admit arbitrary edges: the graph owner rebuilds
    // once and compares every semantic field, including these exact IDs and endpoints.
    Ok(retained)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_bind_explicit_field_boundaries_and_roles() -> BuildResult<()> {
        let cases = [
            (
                EdgeKind::Data,
                "draft",
                "final_text",
                "review",
                "brief",
                r#"["milkdrift.author.edge.v2","data","draft","final_text","review","brief"]"#,
            ),
            (
                EdgeKind::Data,
                "draft",
                "final_text",
                "review",
                "x:brief",
                r#"["milkdrift.author.edge.v2","data","draft","final_text","review","x:brief"]"#,
            ),
            (
                EdgeKind::Data,
                "draft",
                "final_text",
                "review:x",
                "brief",
                r#"["milkdrift.author.edge.v2","data","draft","final_text","review:x","brief"]"#,
            ),
            (
                EdgeKind::Control,
                "draft",
                "final_text",
                "review",
                "brief",
                r#"["milkdrift.author.edge.v2","control","draft","final_text","review","brief"]"#,
            ),
            (
                EdgeKind::Data,
                "draft:a",
                "b",
                "review",
                "brief",
                r#"["milkdrift.author.edge.v2","data","draft:a","b","review","brief"]"#,
            ),
            (
                EdgeKind::Data,
                "draft",
                "a:b",
                "review",
                "brief",
                r#"["milkdrift.author.edge.v2","data","draft","a:b","review","brief"]"#,
            ),
            (
                EdgeKind::Data,
                "review",
                "brief",
                "draft",
                "final_text",
                r#"["milkdrift.author.edge.v2","data","review","brief","draft","final_text"]"#,
            ),
            (
                EdgeKind::Data,
                "Draft",
                "final_text",
                "review",
                "brief",
                r#"["milkdrift.author.edge.v2","data","Draft","final_text","review","brief"]"#,
            ),
        ];
        let mut identities = std::collections::BTreeSet::new();
        for (kind, source, port, target, input, expected_encoding) in cases {
            let id = current(kind, source, port, target, input)?;
            assert_eq!(
                id.as_str(),
                format!("author.{}", blake3::hash(expected_encoding.as_bytes()))
            );
            assert_eq!(id, current(kind, source, port, target, input)?);
            assert!(identities.insert(id));
        }
        Ok(())
    }

    #[test]
    fn edge_builder_preserves_maximum_names_and_refuses_oversize_endpoints() -> BuildResult<()> {
        let node = format!("n{}", ":".repeat(127));
        let port = format!("p{}", ":".repeat(95));
        let mut edges = vec![];
        super::super::graph::add_edge(&mut edges, EdgeKind::Data, &node, &port, &node, &port)?;
        let edge = edges.first().ok_or("edge absent")?;
        assert_eq!(edge.id().as_str().len(), 71);
        assert_eq!(edge.source_node().as_str(), node);
        assert_eq!(edge.source_port().as_str(), port);
        assert_eq!(edge.target_node().as_str(), node);
        assert_eq!(edge.target_port().as_str(), port);
        assert!(
            super::super::graph::add_edge(
                &mut edges,
                EdgeKind::Data,
                &format!("{node}x"),
                &port,
                "target",
                "input"
            )
            .is_err()
        );
        assert!(
            super::super::graph::add_edge(
                &mut edges,
                EdgeKind::Data,
                "source",
                "output",
                "target",
                &format!("{port}x")
            )
            .is_err()
        );
        assert_eq!(edges.len(), 1);
        Ok(())
    }
}
