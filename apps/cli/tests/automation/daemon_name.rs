//! The CLI sends exact name commands through the public client and pins their daemon identity.
use super::{Server, TestResult, http, negotiation, response};
use milkdrift_control_protocol::ProtocolVersion;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn metadata() -> String {
    response(
        json!({"daemon_id":"host:studio-a","name":"Current name","generation":51,"can_rename":true}),
    )
}

#[test]
fn daemon_rename_preserves_exact_stale_guard_and_binds_the_bootstrapped_identity() -> TestResult {
    let receipt = json!({"request_id":"rename-exact","daemon_id":"host:studio-a","name":"Studio Δ","generation":43});
    let expected = receipt.clone();
    let server = Server::with_request_hook(
        vec![negotiation(), metadata(), response(receipt)],
        |index, request| {
            let text = std::str::from_utf8(request).map_err(std::io::Error::other)?;
            match index {
                0 => assert!(text.starts_with("POST /v1/version HTTP/1.1\r\n")),
                1 => assert!(text.starts_with("GET /v1/daemon/metadata HTTP/1.1\r\n")),
                2 => {
                    assert!(text.starts_with("POST /v1/daemon/rename HTTP/1.1\r\n"));
                    assert!(
                        text.to_ascii_lowercase()
                            .contains("\r\nx-milkdrift-daemon: host:studio-a\r\n")
                    );
                    let (_, body) = text
                        .split_once("\r\n\r\n")
                        .ok_or_else(|| std::io::Error::other("request body absent"))?;
                    let actual: serde_json::Value = serde_json::from_str(body)?;
                    assert_eq!(
                        actual,
                        json!({"request_id":"rename-exact","expected_generation":42,"name":"  Studio Δ  "})
                    );
                }
                _ => return Err(std::io::Error::other("rename must not add requests")),
            }
            Ok(())
        },
    )?;
    let (exit, records, errors) = server.invoke(
        &[
            "daemon",
            "rename",
            "  Studio Δ  ",
            "--expected-generation",
            "42",
            "--request-id",
            "rename-exact",
        ],
        false,
    )?;
    assert_eq!(exit, 0, "{records:?}; {errors}");
    assert_eq!(
        records.first().and_then(|record| record.get("value")),
        Some(&expected)
    );
    Ok(())
}

#[test]
fn rename_failure_is_never_automatically_resubmitted() -> TestResult {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let error = json!({"protocol":ProtocolVersion::CURRENT,"request_id":null,"code":"unavailable","message":"result unavailable","retryable":true,"details":{}});
    let server = Server::with_response_hook(
        vec![negotiation(), metadata(), http(503, error.to_string())],
        move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )?;
    let (exit, records, errors) = server.invoke(
        &[
            "daemon",
            "rename",
            "Studio",
            "--expected-generation",
            "42",
            "--request-id",
            "rename-exact",
        ],
        false,
    )?;
    assert_eq!(exit, 5, "{records:?}; {errors}");
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    Ok(())
}

#[test]
fn rename_receipt_lookup_is_actor_authenticated_and_daemon_bound() -> TestResult {
    let receipt = json!({"request_id":"rename-exact","daemon_id":"host:studio-a","name":"Studio","generation":43});
    let expected = receipt.clone();
    let server = Server::with_request_hook(
        vec![negotiation(), metadata(), response(receipt)],
        |index, request| {
            if index == 2 {
                let text = std::str::from_utf8(request).map_err(std::io::Error::other)?;
                assert!(
                    text.starts_with("GET /v1/daemon/rename-requests/rename-exact HTTP/1.1\r\n")
                );
                assert!(
                    text.to_ascii_lowercase()
                        .contains("\r\nx-milkdrift-daemon: host:studio-a\r\n")
                );
                assert!(
                    text.to_ascii_lowercase()
                        .contains("\r\nauthorization: bearer ")
                );
            }
            Ok(())
        },
    )?;
    let (exit, records, errors) =
        server.invoke(&["daemon", "rename-receipt", "rename-exact"], false)?;
    assert_eq!(exit, 0, "{records:?}; {errors}");
    assert_eq!(
        records.first().and_then(|record| record.get("value")),
        Some(&expected)
    );
    Ok(())
}

#[test]
fn daemon_name_reads_server_owned_metadata_without_a_client_label() -> TestResult {
    let server = Server::new(vec![negotiation(), metadata()])?;
    let (exit, records, errors) = server.invoke(&["daemon", "name"], false)?;
    assert_eq!(exit, 0, "{records:?}; {errors}");
    assert_eq!(
        records
            .first()
            .and_then(|record| record.pointer("/value/name")),
        Some(&json!("Current name"))
    );
    Ok(())
}
