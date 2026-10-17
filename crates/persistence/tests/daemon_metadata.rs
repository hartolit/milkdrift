//! Shared daemon names normalize once and refuse malformed display text.
use milkdrift_persistence::DaemonDisplayName;

#[test]
fn daemon_names_normalize_surrounding_spaces_and_count_unicode_scalars()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(DaemonDisplayName::default().as_str(), "Milkdrift");
    assert_eq!(DaemonDisplayName::new("  Studio A　")?.as_str(), "Studio A");
    let name = "🪻".repeat(80);
    let accepted = DaemonDisplayName::new(&name)?;
    assert_eq!(accepted.as_str(), name);
    assert_eq!(
        serde_json::to_string(&accepted)?,
        serde_json::to_string(&name)?
    );
    assert_eq!(
        serde_json::from_str::<DaemonDisplayName>(&serde_json::to_string(&name)?)?,
        accepted
    );
    assert!(DaemonDisplayName::new("🪻".repeat(81)).is_err());
    Ok(())
}

#[test]
fn daemon_names_refuse_empty_and_control_characters_including_trimmed_edges() {
    for name in [
        "",
        "  　",
        "Studio\nA",
        "\tStudio",
        "Studio\n",
        "Studio\0",
        "Studio\u{7f}",
        "Studio\u{85}",
    ] {
        assert!(DaemonDisplayName::new(name).is_err(), "accepted {name:?}");
        let encoded = serde_json::json!(name).to_string();
        assert!(serde_json::from_str::<DaemonDisplayName>(&encoded).is_err());
    }
}
