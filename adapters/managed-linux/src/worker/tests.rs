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
