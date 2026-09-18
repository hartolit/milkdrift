use super::*;
use crate::{BearerCredential, ClientConfig};
use serde_json::{Value, json};
use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    thread,
    time::Duration,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn response(value: Value) -> TestResult<(ControlClient, thread::JoinHandle<std::io::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 8192);
            stream.read_exact(&mut byte)?;
            request.extend_from_slice(&byte);
        }
        let body = json!({"protocol":milkdrift_control_protocol::ProtocolVersion::CURRENT,"request_id":"test-response","value":value}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        Ok(())
    });
    let mut config = ClientConfig::new(endpoint);
    config.request_timeout = Duration::from_secs(5);
    config.safe_query_retries = 0;
    Ok((
        ControlClient::new(config, BearerCredential::new("fixture")?)?,
        worker,
    ))
}

#[tokio::test]
async fn observation_pages_require_the_exact_execution_cursor_and_status() -> TestResult {
    let expected = PeerExecutionId::new("execution:expected")?;
    let page = json!({"execution":expected,"after_sequence":2,"next_sequence":2,"observations":[],"status":"running","terminal":false,"closed":false,"history":{"type":"hot"}});
    for alteration in [None, Some("execution"), Some("cursor"), Some("closure")] {
        let mut value = page.clone();
        match alteration {
            Some("execution") => value["execution"] = json!("execution:other"),
            Some("cursor") => {
                value["after_sequence"] = json!(3);
                value["next_sequence"] = json!(3);
            }
            Some("closure") => value["closed"] = json!(true),
            _ => {}
        }
        let (client, worker) = response(value)?;
        let result = client.invocation_observations(&expected, 2, 1).await;
        worker.join().map_err(|_| "fixture panicked")??;
        if alteration.is_none() {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(ClientError::Protocol(_))),
                "{result:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn request_lookup_refuses_a_valid_response_for_another_key() -> TestResult {
    for key in ["request:expected", "request:other"] {
        let (client, worker) = response(json!({"type":"not_accepted","request_id":key}))?;
        let result = client
            .invocation_lookup(&PeerRequestId::new("request:expected")?)
            .await;
        worker.join().map_err(|_| "fixture panicked")??;
        assert_eq!(result.is_ok(), key == "request:expected");
    }
    Ok(())
}
