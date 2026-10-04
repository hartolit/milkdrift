//! Bounded streaming observation through an owned CLI child.

use milkdrift_evidence::{
    EvidenceResult,
    application::{CliRunner, OwnedChild, ensure},
};
use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Observations {
    page: bool,
    update: bool,
}

pub(super) struct TimelineFollower {
    child: OwnedChild,
    reader: Option<thread::JoinHandle<io::Result<Observations>>>,
    finished: Option<Result<(), String>>,
}

impl TimelineFollower {
    pub(super) fn start(runner: &CliRunner, run: &str, timeout_secs: u64) -> EvidenceResult<Self> {
        let mut child = OwnedChild::spawn(
            Command::new(&runner.executable)
                .arg("--endpoint")
                .arg(&runner.endpoint)
                .arg("--token-file")
                .arg(&runner.token_file)
                .arg("--json")
                .arg("--timeout-secs")
                .arg(timeout_secs.to_string())
                .args(["run", "timeline", run, "--limit", "100", "--follow"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )?;
        let stdout = child
            .take_stdout()
            .ok_or("timeline stdout pipe is absent")?;
        let (send, ready) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || observe(stdout, send));
        let mut follower = Self {
            child,
            reader: Some(reader),
            finished: None,
        };
        if let Err(error) = ready.recv_timeout(Duration::from_secs(10)) {
            return match follower.finish() {
                Ok(()) => Err(error.into()),
                Err(cleanup) => {
                    Err(format!("timeline startup failed: {error}; finish: {cleanup}").into())
                }
            };
        }
        Ok(follower)
    }

    pub(super) fn finish(&mut self) -> EvidenceResult {
        if let Some(result) = &self.finished {
            return result.clone().map_err(Into::into);
        }
        let outcome = (|| -> EvidenceResult {
            self.child.terminate()?;
            let started = Instant::now();
            while self
                .reader
                .as_ref()
                .is_some_and(|reader| !reader.is_finished())
            {
                ensure(
                    started.elapsed() < Duration::from_secs(2),
                    "timeline reader stop unconfirmed",
                )?;
                thread::sleep(Duration::from_millis(5));
            }
            if let Some(reader) = self.reader.take() {
                let observed = reader.join().map_err(|_| "timeline reader panicked")??;
                ensure(
                    observed.page && observed.update,
                    "timeline follow did not expose both its bounded page and resumed observations",
                )?;
            }
            Ok(())
        })();
        self.finished = Some(outcome.as_ref().map(|()| ()).map_err(ToString::to_string));
        outcome
    }
}

impl Drop for TimelineFollower {
    fn drop(&mut self) {
        if self.finished.is_none()
            && let Err(error) = self.finish()
        {
            #[expect(
                clippy::print_stderr,
                reason = "Cancellation/unwind cannot return follower cleanup failure; completed scenarios must call finish explicitly."
            )]
            {
                eprintln!("timeline follower finish failed: {error}");
            }
        }
    }
}

fn observe(input: impl Read, ready: mpsc::SyncSender<()>) -> io::Result<Observations> {
    let mut input = BufReader::new(input);
    let mut observed = Observations::default();
    // Consume every bounded line as it arrives. Retain only the required structural witnesses;
    // the CLI deadline bounds stream lifetime independently of the number of output fragments.
    loop {
        let mut line = String::new();
        let read = input.by_ref().take(131_073).read_line(&mut line)?;
        if read == 0 {
            return Ok(observed);
        }
        if read > 131_072 {
            return Err(io::Error::other("timeline line exceeds bound"));
        }
        let value: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        if value["type"] == "run.timeline"
            && value["status"] == "reconnecting"
            && value["final"] == false
            && value["error"]["retryable"] == true
        {
            observed.update = false;
            continue;
        }
        if value["status"] != "success" || !value["error"].is_null() {
            return Err(io::Error::other(format!(
                "timeline CLI reported {}: {}",
                value["status"].as_str().unwrap_or("missing status"),
                value["error"]["classification"]
                    .as_str()
                    .unwrap_or("missing classification"),
            )));
        }
        match value["type"].as_str() {
            Some("run.timeline") if !observed.page => {
                observed.page = true;
                ready.send(()).map_err(io::Error::other)?;
            }
            Some("run.observation") => match value["value"]["observation"]["type"].as_str() {
                Some("timeline" | "run_status") => observed.update = true,
                _ => {
                    return Err(io::Error::other(
                        "run observation requires resynchronization or closed",
                    ));
                }
            },
            _ => return Err(io::Error::other("unexpected timeline document")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn reader_failure_remains_failed_after_child_reaping() -> EvidenceResult {
        let child = OwnedChild::spawn(Command::new("/bin/sleep").arg("30"))?;
        let mut follower = TimelineFollower {
            child,
            reader: Some(thread::spawn(|| {
                Err(io::Error::other("injected reader failure"))
            })),
            finished: None,
        };
        let error = follower
            .finish()
            .err()
            .ok_or("reader failure was discarded")?;
        assert!(error.to_string().contains("injected reader failure"));
        assert!(follower.child.try_wait()?.is_some());
        let repeated = follower
            .finish()
            .err()
            .ok_or("repeated finish erased reader failure")?;
        assert_eq!(repeated.to_string(), error.to_string());
        Ok(())
    }

    #[test]
    fn long_stream_is_consumed_through_its_last_document() -> io::Result<()> {
        let mut stream = String::from("{\"status\":\"success\",\"type\":\"run.timeline\"}\n");
        stream.push_str(&"{\"status\":\"success\",\"type\":\"run.observation\",\"value\":{\"observation\":{\"type\":\"timeline\"}}}\n".repeat(200));
        let (send, ready) = mpsc::sync_channel(1);
        let observed = observe(stream.as_bytes(), send)?;
        ready.try_recv().map_err(io::Error::other)?;
        assert!(observed.page && observed.update);
        stream.push_str("{\"status\":\"error\",\"type\":\"run.observation\"}\n");
        let (send, _ready) = mpsc::sync_channel(1);
        assert!(observe(stream.as_bytes(), send).is_err());
        Ok(())
    }

    #[test]
    fn oversized_line_is_refused() {
        let (send, _ready) = mpsc::sync_channel(1);
        assert!(observe("x".repeat(131_073).as_bytes(), send).is_err());
    }

    #[test]
    fn reconnect_requires_a_subsequent_observation() -> io::Result<()> {
        let mut stream = String::from(
            "{\"status\":\"success\",\"type\":\"run.timeline\"}\n\
             {\"status\":\"success\",\"type\":\"run.observation\",\"value\":{\"observation\":{\"type\":\"timeline\"}}}\n\
             {\"status\":\"reconnecting\",\"type\":\"run.timeline\",\"final\":false,\"error\":{\"retryable\":true}}\n",
        );
        let (send, _ready) = mpsc::sync_channel(1);
        assert!(!observe(stream.as_bytes(), send)?.update);
        stream.push_str("{\"status\":\"success\",\"type\":\"run.observation\",\"value\":{\"observation\":{\"type\":\"timeline\"}}}\n");
        let (send, _ready) = mpsc::sync_channel(1);
        assert!(observe(stream.as_bytes(), send)?.update);
        Ok(())
    }
}
