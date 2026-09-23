use super::private_id_mappings;

#[test]
fn realized_user_maps_exclude_the_manager_identity() {
    for offset in [1, 65537] {
        assert!(private_id_mappings(Some(&serde_json::json!({
            "UidMap":[format!("0:{offset}:65536")], "GidMap":[format!("0:{offset}:65536")]
        }))));
    }
    for invalid in [
        "0:0:65536",
        "0:1:65535",
        "1:1:65536",
        "0:1:0",
        "bad",
        "0:18446744073709551615:65536",
    ] {
        for key in ["UidMap", "GidMap"] {
            let mut maps = serde_json::json!({"UidMap":["0:1:65536"],"GidMap":["0:1:65536"]});
            maps[key] = serde_json::json!([invalid]);
            assert!(!private_id_mappings(Some(&maps)), "{key}: {invalid}");
        }
    }
    assert!(!private_id_mappings(None));
    assert!(!private_id_mappings(Some(&serde_json::json!({}))));
}

#[test]
fn encoded_output_bound_covers_escaping_replacement_and_extreme_exit_codes()
-> Result<(), Box<dyn std::error::Error>> {
    let stdout = [0, 1, b'"', b'\\', 255];
    let stderr = [254, 0];
    let document = super::result_document(
        &String::from_utf8_lossy(&stdout),
        &String::from_utf8_lossy(&stderr),
        false,
        Some(i32::MIN),
    );
    let bound = super::output_artifact_limit((stdout.len() + stderr.len()) as u64)?;
    assert!(serde_json::to_vec(&document)?.len() as u64 <= bound);
    assert!(super::output_artifact_limit(u64::MAX).is_err());
    Ok(())
}
