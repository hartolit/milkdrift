use milkdrift_evidence::{
    EvidenceResult,
    application::ensure,
    http_fixture::{LoopbackServer, read_request},
};
use serde_json::json;
use std::{
    fs,
    io::Write as _,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) const HARBOR_BRIEF: &str = "Harbor Host 1.4 now shows queued jobs and lets operators download completed job logs. This release fixes duplicate status rows after reconnecting. Running jobs keep their current settings during an upgrade. Automatic failover is not included. Write release notes for the operators who will upgrade.";
pub(super) const LANTERN_BRIEF: &str = "Lantern Rally 0.8 adds private lobby codes and reconnecting to a match for up to 30 seconds after a dropped connection. It fixes duplicate lap notifications. Hosts and players must use the same version. Cross-region matchmaking is not included. Write release notes for players.";

pub(super) struct Model {
    pub(super) server: LoopbackServer,
    calls: Arc<AtomicUsize>,
}

impl Model {
    pub(super) fn start(root: &Path) -> EvidenceResult<Self> {
        let root = root.to_owned();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let server = LoopbackServer::start(move |stream| {
            let request = read_request(stream)?;
            let index = counted.fetch_add(1, Ordering::SeqCst);
            fs::write(root.join(format!("provider-request-{index}.txt")), &request)?;
            let (brief, excluded, text, finish) = match index {
                0 => (
                    "Harbor Host 1.4",
                    "Lantern Rally",
                    "Harbor draft: queued jobs, downloadable logs, duplicate status fix; running settings retained; no automatic failover.",
                    "stop",
                ),
                1 => (
                    "Harbor Host 1.4",
                    "Lantern Rally",
                    "Harbor revised notes: queued jobs and downloadable logs; duplicate status rows fixed. Running settings remain unchanged during upgrades. Automatic failover is not included.",
                    "stop",
                ),
                2 => (
                    "Lantern Rally 0.8",
                    "Harbor Host",
                    "Lantern draft: private lobby codes, 30-second reconnect, duplicate lap fix; same version required, no cross-region matchmaking.",
                    "stop",
                ),
                3 => (
                    "Lantern Rally 0.8",
                    "Harbor Host",
                    "Lantern revised notes: private lobby codes and reconnect within 30 seconds; duplicate lap notifications fixed. Hosts and players need the same version. Cross-region matchmaking is not included.",
                    "stop",
                ),
                4 => (
                    "Harbor Host 1.4",
                    "Lantern Rally",
                    "PRIVATE EARLIER DRAFT",
                    "stop",
                ),
                5 => (
                    "Harbor Host 1.4",
                    "Lantern Rally",
                    "SELECTED INCOMPLETE RESULT",
                    "length",
                ),
                6 => (
                    "Harbor Host 1.4",
                    "PRIVATE EARLIER DRAFT",
                    "Repaired complete notes from the original Harbor brief and selected failed result.",
                    "stop",
                ),
                _ => return Err(std::io::Error::other("unexpected extra model request")),
            };
            if !request.contains(brief) || request.contains(excluded) {
                return Err(std::io::Error::other(
                    "brief isolation or repair context violated",
                ));
            }
            let expected = if matches!(index, 2 | 3) {
                LANTERN_BRIEF
            } else {
                HARBOR_BRIEF
            };
            if !request.contains(expected) {
                return Err(std::io::Error::other(
                    "complete assigned brief missing from provider request",
                ));
            }
            if matches!(index, 1 | 3)
                && !request.contains(if index == 1 {
                    "Harbor draft:"
                } else {
                    "Lantern draft:"
                })
            {
                return Err(std::io::Error::other("selected draft missing"));
            }
            if index == 6 && !request.contains("SELECTED INCOMPLETE RESULT") {
                return Err(std::io::Error::other("selected failed response missing"));
            }
            let body = json!({"id":format!("fixture-{index}"),"model":"controlled-writer","choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":finish}],"usage":{"prompt_tokens":20,"completion_tokens":40,"total_tokens":60}}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
            Ok(())
        })?;
        Ok(Self { server, calls })
    }

    pub(super) fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn finish(&mut self) -> EvidenceResult {
        self.server.finish()?;
        ensure(self.count() <= 7, "fixture request budget exceeded")
    }
}
