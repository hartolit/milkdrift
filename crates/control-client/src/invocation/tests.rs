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
        let fields = value.as_object_mut().ok_or("page is not an object")?;
        match alteration {
            Some("execution") => {
                fields.insert("execution".to_owned(), json!("execution:other"));
            }
            Some("cursor") => {
                fields.insert("after_sequence".to_owned(), json!(3));
                fields.insert("next_sequence".to_owned(), json!(3));
            }
            Some("closure") => {
                fields.insert("closed".to_owned(), json!(true));
            }
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

#[tokio::test]
async fn output_ranges_require_progress_but_allow_an_empty_complete_artifact() -> TestResult {
    let execution = PeerExecutionId::new("execution:output")?;
    for (size, bytes, complete, valid) in [
        (1, vec![], false, false),
        (1, vec![42], true, true),
        (0, vec![], true, true),
    ] {
        let chunk = json!({
            "execution":execution,"offset":0,"bytes":bytes,"complete":complete,
            "metadata": {
                "reference":{"artifact":"output","digest":"0".repeat(64),"media_type":"text/plain","size_bytes":size},
                "sensitivity":"restricted","retention":{"type":"while_referenced"},
                "provenance":{"producer":{"type":"external","source":"fixture"},"causes":[]}
            }
        });
        // Establish that a failure tests the range contract, not malformed metadata.
        let decoded: milkdrift_peer_protocol::InvocationOutputChunk =
            serde_json::from_value(chunk.clone())?;
        assert_eq!(decoded.execution, execution);
        assert_eq!(decoded.metadata.reference().size_bytes(), size);
        let (client, worker) = response(chunk)?;
        let result = client.invocation_output(&execution, "output", 0, 1).await;
        worker.join().map_err(|_| "fixture panicked")??;
        assert_eq!(result.is_ok(), valid, "{result:?}");
    }
    Ok(())
}
