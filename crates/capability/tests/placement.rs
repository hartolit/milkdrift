//! Task constraints and immutable host facts use the same matching boundary.
use std::collections::BTreeSet;

use milkdrift_capability::{
    CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityRequirement, DescriptorBuilder,
    Locality, OperationId, PeerId, PlacementRequirement, ResolvedCapabilitySnapshot,
    ResolvedCapabilitySnapshotDocument,
};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn descriptor(
    locality: Locality,
    peer: Option<PeerId>,
) -> Result<CapabilityDescriptor, Box<dyn std::error::Error>> {
    let base =
        CapabilityDescriptorDocument::from_json(include_bytes!("fixtures/descriptor-v1.json"))?;
    let base = base.body();
    Ok(DescriptorBuilder::new(
        base.identity().clone(),
        base.descriptor_revision(),
        base.category().clone(),
        base.admission().clone(),
        locality,
    )
    .peer(peer)
    .operations(base.operations().clone())
    .build()?)
}

#[test]
fn absent_empty_exact_and_intersecting_placement_have_distinct_meanings() -> TestResult {
    let operation = OperationId::new("model.generate")?;
    let base = CapabilityRequirement::new(operation.clone());
    let a = PeerId::new("peer-a")?;
    let b = PeerId::new("peer-b")?;
    let local = descriptor(Locality::Local, None)?;
    let remote = descriptor(Locality::Remote, None)?;
    let peer_a = descriptor(Locality::Peer, Some(a.clone()))?;
    let peer_b = descriptor(Locality::Peer, Some(b.clone()))?;
    for candidate in [&local, &remote, &peer_a, &peer_b] {
        assert!(candidate.matches(&base).is_match());
    }
    let pinned = base
        .clone()
        .with_placement(PlacementRequirement::new(None, Some(BTreeSet::from([a])))?);
    assert!(peer_a.matches(&pinned).is_match());
    for wrong in [&local, &remote, &peer_b] {
        assert_eq!(wrong.matches(&pinned).mismatch_reasons(), ["placement"]);
    }
    for deny in [
        PlacementRequirement::new(Some(BTreeSet::new()), None)?,
        PlacementRequirement::new(None, Some(BTreeSet::new()))?,
    ] {
        assert!(deny.denies_all());
        let denied = base.clone().with_placement(deny);
        for candidate in [&local, &remote, &peer_a, &peer_b] {
            assert!(!candidate.matches(&denied).is_match());
        }
    }
    assert!(
        PlacementRequirement::new(
            Some(BTreeSet::from([Locality::Local])),
            Some(BTreeSet::from([b]))
        )
        .is_err()
    );
    let encoded = serde_json::to_value(&base)?;
    assert!(encoded.get("placement").is_none());
    assert_eq!(
        serde_json::from_value::<CapabilityRequirement>(encoded)?,
        base
    );
    assert_eq!(
        serde_json::from_value::<CapabilityRequirement>(serde_json::to_value(&pinned)?)?,
        pinned
    );
    Ok(())
}

#[test]
fn placement_reader_refuses_ambiguous_and_unbounded_sets() -> TestResult {
    for bad in [
        json!({"localities":["local"], "peers":["peer-a"]}),
        json!({"peers":["peer-a", "peer-a"]}),
        json!({"peers":["peer-b", "peer-a"]}),
        json!({"peers":["*"]}),
        json!({"localities":["unknown"]}),
        json!({"localities":["peer","local"]}),
        json!({"tags":["trusted"]}),
        json!({"peers": (0..129).map(|n| format!("peer-{n:03}")).collect::<Vec<_>>()}),
    ] {
        assert!(
            serde_json::from_value::<PlacementRequirement>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    assert!(
        serde_json::from_str::<PlacementRequirement>(r#"{"peers":[],"peers":["peer-a"]}"#).is_err()
    );
    assert_eq!(
        serde_json::from_value::<PlacementRequirement>(json!({"peers":null}))?,
        PlacementRequirement::new(None, None)?
    );
    Ok(())
}

#[test]
fn snapshots_require_current_schema_and_bind_host_facts() -> TestResult {
    let peer = PeerId::new("peer-a")?;
    let descriptor = descriptor(Locality::Peer, Some(peer.clone()))?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let placement = PlacementRequirement::new(None, Some(BTreeSet::from([peer.clone()])))?;
    snapshot.validate_placement(&placement)?;
    assert_eq!(snapshot.locality(), Locality::Peer);
    assert_eq!(snapshot.peer(), Some(&peer));
    let document = ResolvedCapabilitySnapshotDocument::new(snapshot.clone());
    assert_eq!(document.schema_version(), 3);
    assert_eq!(
        ResolvedCapabilitySnapshotDocument::from_json(&document.to_canonical_json()?)?,
        document
    );
    for (key, value) in [
        ("locality", json!("local")),
        ("peer", json!("peer-b")),
        ("trust_zones", json!(["trusted"])),
    ] {
        let mut bad = serde_json::to_value(&snapshot)?;
        bad[key] = value;
        assert!(serde_json::from_value::<ResolvedCapabilitySnapshot>(bad).is_err());
    }
    // Envelope and nested readers both reject obsolete formats; no missing host fact is inferred.
    for version in [0, 1, 2, 4, u32::MAX] {
        let mut bad = serde_json::to_value(&document)?;
        bad["schema_version"] = json!(version);
        assert!(ResolvedCapabilitySnapshotDocument::from_json(&serde_json::to_vec(&bad)?).is_err());
        assert!(serde_json::from_value::<ResolvedCapabilitySnapshotDocument>(bad).is_err());
    }
    for field in ["category", "locality", "trust_zones"] {
        for absent in [true, false] {
            let mut bad = serde_json::to_value(&snapshot)?;
            if absent {
                bad.as_object_mut()
                    .ok_or("snapshot must be an object")?
                    .remove(field);
            } else {
                bad[field] = json!(null);
            }
            assert!(serde_json::from_value::<ResolvedCapabilitySnapshot>(bad).is_err());
        }
    }
    Ok(())
}
