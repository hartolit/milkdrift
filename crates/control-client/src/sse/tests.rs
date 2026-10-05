use super::*;
use crate::{BearerCredential, ClientConfig, ControlClient};
use futures_util::StreamExt as _;
use milkdrift_control_protocol::{
    Cursor, Observation, ProtocolError, ProtocolVersion, TimelineCategory, TimelineEntry,
};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn observation(position: u64) -> TestResult<ObservationEnvelope> {
    Ok(ObservationEnvelope {
        protocol: ProtocolVersion::CURRENT,
        cursor: Cursor::new("run:test", position)?,
        observed_at_ms: position,
        feed: "run:test".into(),
        observation: Observation::Timeline(TimelineEntry {
            sequence: position,
            timestamp_ms: position,
            category: TimelineCategory::Lifecycle,
            actor: "human:test".into(),
            run_id: "test".into(),
            node_id: None,
            attempt_id: None,
            revision_id: None,
            summary: "Ændring 日本語 🦀".into(),
            detail: Value::Null,
        }),
    })
}

fn frame(observation: &ObservationEnvelope, ending: &str) -> TestResult<String> {
    Ok(format!(
        "data: {}{ending}{ending}",
        serde_json::to_string(observation)?
    ))
}

fn decode(chunks: &[&[u8]]) -> Result<Vec<ObservationEnvelope>, ClientError> {
    let mut decoder = Decoder::default();
    let mut events = vec![];
    for chunk in chunks {
        for byte in *chunk {
            if let Some(event) = decoder.push(*byte)? {
                events.push(event);
            }
        }
    }
    Ok(events)
}

#[test]
fn earliest_event_wins_across_all_line_endings_and_every_byte_split() -> TestResult {
    let first = observation(1)?;
    let second = observation(2)?;
    for a in ["\r\n", "\n", "\r"] {
        for b in ["\r\n", "\n", "\r"] {
            let wire = format!("{}{}", frame(&first, a)?, frame(&second, b)?);
            let expected = vec![first.clone(), second.clone()];
            assert_eq!(decode(&[wire.as_bytes()])?, expected);
            for boundary in 0..=wire.len() {
                let (prefix, suffix) = wire.as_bytes().split_at(boundary);
                assert_eq!(
                    decode(&[prefix, suffix])?,
                    expected,
                    "{a:?} {b:?} split {boundary}"
                );
            }
            assert_eq!(
                decode(&wire.as_bytes().chunks(1).collect::<Vec<_>>())?,
                expected
            );
        }
    }
    Ok(())
}

#[test]
fn comments_bom_multiline_data_and_empty_blocks_follow_line_semantics() -> TestResult {
    let expected = observation(1)?;
    let mut wire =
        "\u{feff}: heartbeat\r\n\r\n\nunknown\r\nid: forged-cursor\rretry: 0\ndata\r\n".to_owned();
    for (index, line) in serde_json::to_string_pretty(&expected)?.lines().enumerate() {
        let ending = ["\r\n", "\n", "\r"].get(index % 3).ok_or("ending")?;
        wire.push_str(&format!("data: {line}{ending}"));
    }
    wire.push_str("data:\r\n\r\n: more comments\r\r");
    for boundary in 0..=wire.len() {
        let (a, b) = wire.as_bytes().split_at(boundary);
        assert_eq!(decode(&[a, b])?, vec![expected.clone()]);
    }
    assert!(decode(&[b": heartbeat\r\n\r\n\r\n\nunknown\n\nid: 99\n\n"])?.is_empty());
    // Only the first BOM is stripped. A later BOM is part of an unknown field name.
    assert!(decode(&[format!("\n\u{feff}{}", frame(&expected, "\n")?).as_bytes()])?.is_empty());
    // SSE dispatches an empty data event; the application refuses its absent JSON.
    for empty in [b"data\n\n".as_slice(), b"data:\r\r"] {
        assert!(matches!(decode(&[empty]), Err(ClientError::Protocol(_))));
    }
    Ok(())
}

#[test]
fn incomplete_eof_never_dispatches_and_refusals_keep_the_valid_prefix() -> TestResult {
    let first = observation(1)?;
    let second = frame(&observation(2)?, "\n")?;
    let complete = frame(&first, "\r\n")?;
    for end in 0..second.len() {
        assert_eq!(
            decode(&[
                complete.as_bytes(),
                second.as_bytes().get(..end).ok_or("prefix")?
            ])?,
            vec![first.clone()]
        );
    }
    for invalid in [
        b"data: {bad}\n\n".as_slice(),
        b"data: \xff\r\r",
        b": \xff\n\n",
    ] {
        let mut decoder = Decoder::default();
        let mut accepted = vec![];
        for byte in complete.bytes() {
            if let Some(event) = decoder.push(byte)? {
                accepted.push(event);
            }
        }
        assert!(
            invalid
                .iter()
                .try_for_each(|byte| decoder.push(*byte).map(|_| ()))
                .is_err()
        );
        assert_eq!(accepted, vec![first.clone()]);
    }
    for minor in [
        0,
        ProtocolVersion::CURRENT.minor - 1,
        ProtocolVersion::CURRENT.minor + 1,
    ] {
        let mut unsupported = observation(2)?;
        unsupported.protocol.minor = minor;
        assert!(matches!(
            decode(&[frame(&unsupported, "\n")?.as_bytes()]),
            Err(ClientError::Protocol(
                ProtocolError::UnsupportedMinor { .. }
            ))
        ));
    }
    Ok(())
}

#[test]
fn limits_apply_to_each_event_independently_of_network_chunk_size() -> TestResult {
    let mut decoder = Decoder::default();
    for _ in 0..MAX_FRAME_BYTES {
        assert!(decoder.push(b'x')?.is_none());
    }
    assert!(matches!(decoder.push(b'x'), Err(ClientError::Stream(_))));
    // Many complete bounded comment blocks in one chunk must not look like one huge frame.
    let large_chunk = b": heartbeat\n\n".repeat(MAX_FRAME_BYTES / 12 + 1);
    assert!(large_chunk.len() > MAX_FRAME_BYTES);
    assert!(decode(&[&large_chunk])?.is_empty());
    let mut decoder = Decoder::default();
    for _ in 0..MAX_FRAME_BYTES / 3 {
        for byte in b":\r\n" {
            assert!(decoder.push(*byte)?.is_none());
        }
    }
    assert!(matches!(
        decoder
            .push(b'x')
            .and_then(|_| decoder.push(b'x'))
            .and_then(|_| decoder.push(b'x')),
        Err(ClientError::Stream(_))
    ));
    Ok(())
}

async fn fixture(
    responses: Vec<String>,
) -> TestResult<(
    ControlClient,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<Result<Vec<String>, String>>,
)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}/", listener.local_addr()?).parse()?;
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let task = tokio::spawn(async move {
        let serve = async {
            let mut paths = vec![];
            for body in responses {
                let (mut stream, _) = listener.accept().await?;
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    if request.len() >= 8192 {
                        return Err("fixture request headers too large".into());
                    }
                    request.push(stream.read_u8().await?);
                }
                let request = std::str::from_utf8(&request)?;
                paths.push(request.lines().next().ok_or("request line")?.to_owned());
                observed.fetch_add(1, Ordering::SeqCst);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).await?;
                stream.write_all(body.as_bytes()).await?;
                stream.shutdown().await?;
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(paths)
        };
        tokio::time::timeout(Duration::from_secs(15), serve)
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    });
    let mut config = ClientConfig::new(endpoint);
    config.retry_delay = Duration::from_millis(1);
    Ok((
        ControlClient::new(config, BearerCredential::new("sse-fixture")?)?,
        count,
        task,
    ))
}

#[tokio::test]
async fn subscription_reconnects_after_incomplete_eof_without_advancing_cursor() -> TestResult {
    let a = observation(1)?;
    let b = observation(2)?;
    let c = observation(3)?;
    let mut closing = observation(4)?;
    closing.observation = Observation::StreamClosing {
        reason: "fixture complete".into(),
    };
    let first = format!(
        ": comment\r\n\r\n{}{}data: {}\r\n",
        frame(&a, "\r\n")?,
        frame(&b, "\n")?,
        serde_json::to_string(&c)?
    );
    let second = format!(
        "{}{}{}",
        frame(&b, "\r")?,
        frame(&c, "\r\n")?,
        frame(&closing, "\n")?
    );
    let (client, count, task) = fixture(vec![first, second]).await?;
    let mut stream = client.subscribe("v1/runs/test/stream", None);
    for expected in [&a, &b] {
        assert_eq!(stream.next().await.ok_or("ended")??, *expected);
    }
    assert!(matches!(
        stream.next().await.ok_or("EOF error")?,
        Err(ClientError::Transport(_))
    ));
    assert_eq!(stream.next().await.ok_or("resumed")??, c);
    assert_eq!(stream.next().await.ok_or("closing")??, closing);
    assert!(stream.next().await.is_none());
    let paths = task.await?.map_err(|e| format!("fixture: {e}"))?;
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(
        paths,
        vec![
            "GET /v1/runs/test/stream HTTP/1.1".to_owned(),
            format!(
                "GET /v1/runs/test/stream?cursor={} HTTP/1.1",
                b.cursor.as_str()
            )
        ]
    );
    Ok(())
}

#[tokio::test]
async fn subscription_stops_on_malformed_or_foreign_events_without_reconnecting() -> TestResult {
    let accepted = observation(1)?;
    let mut foreign = observation(2)?;
    foreign.feed = "run:other".into();
    let mut bad_cursor = observation(2)?;
    bad_cursor.cursor = Cursor::new("run:other", 2)?;
    for bad in [
        "data: {bad}\n\n".to_owned(),
        frame(&foreign, "\r\n")?,
        frame(&bad_cursor, "\r")?,
    ] {
        let (client, count, task) =
            fixture(vec![format!("{}{}", frame(&accepted, "\r\n")?, bad)]).await?;
        let mut stream = client.subscribe("v1/runs/test/stream", None);
        assert_eq!(stream.next().await.ok_or("first")??.cursor, accepted.cursor);
        assert!(stream.next().await.ok_or("refusal")?.is_err());
        assert!(stream.next().await.is_none());
        task.await?.map_err(|e| format!("fixture: {e}"))?;
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    Ok(())
}
