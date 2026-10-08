use super::*;

fn binding() -> CursorBinding {
    CursorBinding {
        actor: "actor:one".into(),
        grant_id: "grant:one".into(),
        grant_revision: 1,
        grant_digest: format!("b3_{}", "a".repeat(64)),
        scope_digest: format!("b3_{}", "b".repeat(64)),
    }
}

#[test]
fn capability_windows_bind_every_authority_field_and_have_a_fixed_capacity()
-> Result<(), Box<dyn std::error::Error>> {
    let mut feeds = CapabilityFeeds::default();
    let original = binding();
    feeds
        .get(original.clone())
        .ok_or("initial feed absent")?
        .next_position = 9;
    let mut variants = vec![original.clone(); 5];
    variants.get_mut(0).ok_or("variant absent")?.actor = "actor:two".into();
    variants.get_mut(1).ok_or("variant absent")?.grant_id = "grant:two".into();
    variants.get_mut(2).ok_or("variant absent")?.grant_revision = 2;
    variants.get_mut(3).ok_or("variant absent")?.grant_digest = format!("b3_{}", "c".repeat(64));
    variants.get_mut(4).ok_or("variant absent")?.scope_digest = format!("b3_{}", "d".repeat(64));
    for variant in variants {
        assert_eq!(
            feeds
                .get(variant)
                .ok_or("distinct feed absent")?
                .next_position,
            1
        );
    }
    for revision in 3..=u64::try_from(crate::config::MAX_ACTOR_BINDINGS)? {
        let mut key = original.clone();
        key.grant_revision = revision;
        let _feed = feeds.get(key);
    }
    assert_eq!(feeds.0.len(), crate::config::MAX_ACTOR_BINDINGS);
    let mut overflow = original.clone();
    overflow.actor = "actor:overflow".into();
    assert!(feeds.get(overflow).is_none());
    assert_eq!(
        feeds
            .get(original)
            .ok_or("original feed evicted")?
            .next_position,
        9
    );
    Ok(())
}

#[test]
fn capability_window_bounds_both_items_and_encoded_bytes() -> Result<(), Box<dyn std::error::Error>>
{
    let mut feed = CapabilityFeed {
        next_position: 1,
        ..CapabilityFeed::default()
    };
    let mut value = CapabilityRead {
        capability_id: "capability:test".into(),
        generation: 1,
        descriptor_digest: "digest".into(),
        category: "tool".into(),
        operations: Vec::new(),
        operation_contracts: Vec::new(),
        provider_profile: None,
        locality: "local".into(),
        peer_id: None,
        trust_zones: Vec::new(),
        execution_trust: "trusted_host_process".into(),
        current: true,
        draining: false,
        health: "healthy".into(),
        available: Some(true),
        active_permits: 0,
        permit_limit: 1,
    };
    for generation in 1..=300 {
        value.generation = generation;
        record_capability_snapshot(&mut feed, &[value.clone()]).map_err(|()| "snapshot refused")?;
    }
    assert_eq!(feed.entries.len(), CAPABILITY_FEED_ITEMS);
    assert!(feed.after(1).is_none());
    assert_eq!(
        feed.after(0).ok_or("fresh snapshot absent")?,
        vec![(300, vec![value.clone()])]
    );
    assert_eq!(
        feed.after(299).ok_or("replay absent")?,
        vec![(300, vec![value.clone()])]
    );
    value.trust_zones = vec!["x".repeat(8_000); 16];
    for generation in 301..=320 {
        value.generation = generation;
        record_capability_snapshot(&mut feed, &[value.clone()]).map_err(|()| "snapshot refused")?;
        assert!(feed.retained_bytes <= MAX_DOCUMENT_BYTES);
    }
    assert!(feed.entries.len() < CAPABILITY_FEED_ITEMS);
    assert_eq!(feed.next_position, 321);
    assert!(feed.after(300).is_none());
    value.trust_zones = vec!["x".repeat(8_000); 200];
    assert!(record_capability_snapshot(&mut feed, &[value]).is_err());
    assert_eq!(
        feed.next_position, 321,
        "refused snapshots cannot move the cursor"
    );
    record_capability_snapshot(&mut feed, &[]).map_err(|()| "empty snapshot refused")?;
    assert_eq!(
        feed.after(320).ok_or("empty snapshot absent")?,
        vec![(321, vec![])]
    );
    assert_eq!(
        feed.retained_bytes,
        feed.entries.iter().map(|(_, _, size)| size).sum::<usize>()
    );
    Ok(())
}
