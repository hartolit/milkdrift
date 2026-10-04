use super::{InvocationMode, Qualify, prepare, publication};
use milkdrift_evidence::{
    EvidenceResult,
    application::{OwnedChild, ensure, run_command, write_private},
};
use serde_json::{Value, json};
use std::io::Write as _;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[derive(Clone, Copy)]
pub(super) enum Caller {
    Operator,
    Consumer,
    Origin,
    Learner,
    Evaluator,
    SourceAuthor,
}
#[derive(Clone, Copy)]
pub(super) enum Expected {
    Success,
    Refused,
    PreAcceptanceRetry,
    InvocationObservation,
}
pub(super) fn text(value: &Value) -> EvidenceResult<&str> {
    value
        .as_str()
        .ok_or_else(|| "expected string in product response".into())
}
pub(super) fn number(value: &Value) -> EvidenceResult<u64> {
    value
        .as_u64()
        .ok_or_else(|| "expected integer in product response".into())
}
pub(super) fn load(path: impl AsRef<Path>) -> EvidenceResult<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub(super) struct Session {
    pub(super) root: PathBuf,
    pub(super) cli: PathBuf,
    pub(super) logs: PathBuf,
    pub(super) protected: Value,
    pub(super) worker: Value,
    pub(super) requests: BTreeMap<String, PathBuf>,
    daemon: PathBuf,
    config: PathBuf,
    port: u16,
    origin: Option<(PathBuf, u16)>,
    children: Vec<OwnedChild>,
    stop_failure: Option<String>,
    starts: u32,
    calls: std::cell::Cell<u64>,
    token: PathBuf,
}
impl Session {
    pub(super) fn resume(
        root: &Path,
        cli: &Path,
        daemon: &Path,
        port: u16,
        evidence: &Path,
        token: &Path,
    ) -> EvidenceResult<Self> {
        let root = root.canonicalize()?;
        let logs = evidence;
        fs::create_dir_all(logs)?;
        let logs = logs.join(format!(
            "session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
        ));
        prepare::private_directory(&logs)?;
        Ok(Self {
            token: token.canonicalize()?,
            config: root.join("host/daemon.toml"),
            root,
            cli: cli.canonicalize()?,
            daemon: daemon.canonicalize()?,
            port,
            logs,
            protected: Value::Null,
            worker: Value::Null,
            requests: BTreeMap::new(),
            origin: None,
            children: Vec::new(),
            stop_failure: None,
            starts: 0,
            calls: std::cell::Cell::new(0),
        })
    }
    pub(super) fn prepare(args: &Qualify) -> EvidenceResult<Self> {
        let root = args.root.canonicalize()?;
        let cli = args.cli.canonicalize()?;
        let daemon = args.daemon.canonicalize()?;
        let logs = root.join("evidence");
        prepare::private_directory(&logs)?;
        let host = root.join("host");
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME absent")?);
        let quadlet = home.join(".config/containers/systemd").join(format!(
            "milkdrift-{}",
            root.file_name()
                .ok_or("root name absent")?
                .to_string_lossy()
        ));
        let bootstrap = |recipe: &str, preview: bool| -> EvidenceResult<Value> {
            let mut command = Command::new(&daemon);
            command.args(args![
                "managed-bootstrap",
                "--root",
                host.display(),
                "--quadlet-directory",
                quadlet.display(),
                "--systemd-directory",
                home.join(".config/systemd/user").display(),
                "--recipe",
                root.join(recipe).display()
            ]);
            if preview {
                command.arg("--preview");
            }
            let output = run_command(&mut command, None, Duration::from_secs(30))?;
            ensure(
                output.status.success(),
                &format!("bootstrap refused: {}", output.stderr),
            )?;
            write_private(
                &logs.join(if preview {
                    "worker-preview.json"
                } else {
                    "bootstrap.json"
                }),
                output.stdout.as_bytes(),
            )?;
            Ok(serde_json::from_str::<Value>(&output.stdout)?
                .pointer("/recipe")
                .ok_or("missing /recipe")?
                .clone())
        };
        let protected = bootstrap("protected-recipe.json", false)?;
        let worker = bootstrap("worker-recipe.json", true)?;
        let config = host.join("daemon.toml");
        let mut document: Value = toml::from_str(&fs::read_to_string(&config)?)?;
        let document_fields = document.as_object_mut().ok_or("expected JSON object")?;
        document_fields.insert("role".into(), json!("workflow_enabled"));
        document_fields.insert("bind".into(), json!(format!("127.0.0.1:{}", args.port)));
        document
            .pointer_mut("/actors/0")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /actors/0")?
            .insert("actor".into(), json!("agent:repair"));
        document
            .pointer_mut("/actors/0/authority/budget")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /actors/0/authority/budget")?
            .insert(
                "artifact_bytes".into(),
                json!(prepare::INTERNAL_ARTIFACT_BYTES),
            );
        document
            .pointer_mut("/adapters/managed_linux")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /adapters/managed_linux")?
            .insert(
                "recipes".into(),
                json!([host.join("recipe.json"), root.join("worker-recipe.json")]),
            );
        document
            .pointer_mut("/actors/0/authority/resources/capability/identities/values")
            .ok_or("missing /actors/0/authority/resources/capability/identities/values")?
            .as_array_mut()
            .ok_or("bootstrap capability scope absent")?
            .extend([
                json!("managed.slotbook-build"),
                json!("managed.slotbook-build.worker"),
            ]);
        document
            .pointer_mut("/serving/clients/execution_limits")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /serving/clients/execution_limits")?
            .insert(
                "artifact_bytes".into(),
                json!(prepare::INTERNAL_ARTIFACT_BYTES),
            );
        if args.published {
            publication::configure(&mut document, &root)?;
        }
        let origin = if args.invocation_mode == InvocationMode::Peer {
            Some(publication::peer_configuration(
                &mut document,
                &root,
                args.port,
            )?)
        } else {
            None
        };
        fs::write(&config, toml::to_string_pretty(&document)?.as_bytes())?;
        for config in std::iter::once(&config).chain(origin.as_ref().map(|(p, _)| p)) {
            let result = run_command(
                Command::new(&daemon)
                    .arg("--config")
                    .arg(config)
                    .arg("--check-config"),
                None,
                Duration::from_secs(30),
            )?;
            ensure(
                result.status.success(),
                &format!("qualification config refused: {}", result.stderr),
            )?;
        }
        Ok(Self {
            token: root.join("host/operator.token"),
            root,
            cli,
            logs,
            protected,
            worker,
            requests: BTreeMap::new(),
            daemon,
            config,
            port: args.port,
            origin,
            children: Vec::new(),
            stop_failure: None,
            starts: 0,
            calls: std::cell::Cell::new(0),
        })
    }
    fn command(&self, caller: Caller) -> Command {
        let mut command = Command::new(&self.cli);
        let port = if matches!(caller, Caller::Origin) {
            self.origin.as_ref().map_or(self.port, |(_, p)| *p)
        } else {
            self.port
        };
        command.env("MILKDRIFT_ENDPOINT", format!("http://127.0.0.1:{port}"));
        command.env(
            "MILKDRIFT_TOKEN_FILE",
            match caller {
                Caller::Consumer => self.root.join("consumer.token"),
                Caller::Learner => self.root.join("learning-05/learner.token"),
                Caller::Evaluator => self.root.join("learning-05/evaluator.token"),
                Caller::SourceAuthor => self.root.join("source-author.token"),
                _ => self.token.clone(),
            },
        );
        command
    }
    pub(super) fn call(
        &self,
        label: &str,
        arguments: Vec<String>,
        expected: Expected,
        caller: Caller,
    ) -> EvidenceResult<Value> {
        let timeout = if arguments.windows(2).any(|a| a == ["run", "wait"]) {
            600
        } else {
            240
        };
        let output = run_command(
            self.command(caller)
                .args(args![
                    "--json",
                    "--yes",
                    "--timeout-secs",
                    timeout,
                    "--command-id",
                    label
                ])
                .args(arguments),
            None,
            Duration::from_secs(timeout + 10),
        )?;
        self.calls.set(self.calls.get() + 1);
        let log_label = if self.logs.join(format!("{label}.json")).exists() {
            format!("{label}-replay-{}", self.calls.get())
        } else {
            label.to_owned()
        };
        write_private(
            &self.logs.join(format!("{log_label}.json")),
            output.stdout.as_bytes(),
        )?;
        write_private(
            &self.logs.join(format!("{log_label}.stderr")),
            output.stderr.as_bytes(),
        )?;
        let pages = output
            .stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()?;
        let mut final_page = pages.last().cloned().ok_or("CLI produced no response")?;
        let retryable_preparation = final_page
            .pointer("/status")
            .and_then(serde_json::Value::as_str)
            == Some("failure")
            && final_page
                .pointer("/value/type")
                .and_then(serde_json::Value::as_str)
                == Some("rejected")
            && matches!(
                final_page
                    .pointer("/value/code")
                    .ok_or("missing /value/code")?
                    .as_str(),
                Some("catalog_stale" | "deadline")
            )
            && final_page
                .pointer("/value/known_execution")
                .is_some_and(serde_json::Value::is_null);
        let accepted = match expected {
            Expected::Success => output.status.success(),
            Expected::Refused => !output.status.success(),
            Expected::PreAcceptanceRetry => output.status.success() || retryable_preparation,
            Expected::InvocationObservation => {
                output.status.success()
                    || (final_page
                        .pointer("/type")
                        .and_then(serde_json::Value::as_str)
                        == Some("invocation.wait")
                        && final_page
                            .pointer("/error/code")
                            .and_then(serde_json::Value::as_str)
                            == Some("invocation_failed"))
            }
        };
        ensure(
            accepted,
            &format!(
                "{label} returned unexpected disposition: {} {}",
                output.stdout, output.stderr
            ),
        )?;
        writeln!(std::io::stdout().lock(), "{label}")?;
        if final_page
            .pointer("/type")
            .and_then(serde_json::Value::as_str)
            == Some("invocation.wait")
        {
            let mut observations = BTreeMap::new();
            for page in &pages {
                for observation in page
                    .pointer("/value/observations")
                    .ok_or("missing /value/observations")?
                    .as_array()
                    .ok_or("observations absent")?
                {
                    observations.insert(number(&observation["sequence"])?, observation.clone());
                }
            }
            let history = final_page
                .pointer("/value/history")
                .ok_or("missing /value/history")?;
            if history["type"] == "archived" {
                for observation in history
                    .pointer("/summary/output_observations")
                    .ok_or("missing /summary/output_observations")?
                    .as_array()
                    .ok_or("archived outputs absent")?
                {
                    observations.insert(number(&observation["sequence"])?, observation.clone());
                }
                let terminal = history
                    .pointer("/summary/final_observation")
                    .ok_or("missing /summary/final_observation")?;
                if !terminal.is_null() {
                    observations.insert(number(&terminal["sequence"])?, terminal.clone());
                }
            }
            final_page
                .pointer_mut("/value")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("missing object /value")?
                .insert(
                    "observations".into(),
                    json!(observations.into_values().collect::<Vec<_>>()),
                );
        }
        Ok(final_page)
    }
    pub(super) fn ok(&self, label: &str, args: Vec<String>) -> EvidenceResult<Value> {
        self.call(label, args, Expected::Success, Caller::Operator)
    }
    pub(super) fn resource(
        &self,
        label: &str,
        action: &str,
        version: u64,
        extra: Vec<String>,
        target: &str,
        expected: Expected,
    ) -> EvidenceResult<Value> {
        let mut arguments = args![
            "resource",
            "--installation",
            target,
            "--expected-version",
            version,
            action
        ];
        arguments.extend(extra);
        Ok(self
            .call(label, arguments, expected, Caller::Operator)?
            .pointer("/value")
            .ok_or("missing /value")?
            .clone())
    }
    pub(super) fn write(&self, name: &str, value: &Value) -> EvidenceResult<PathBuf> {
        prepare::write(&self.root, name, value)
    }
    pub(super) fn invoke(
        &mut self,
        label: &str,
        capability: &str,
        operation: &str,
        inputs: &Path,
        caller: Caller,
    ) -> EvidenceResult<String> {
        // On recovery prefer the newest retained preparation: an earlier stale document may
        // share a request key with later accepted bytes, and must not be mistaken for a replay.
        let path_for = |attempt| {
            self.root.join(if attempt == 0 {
                format!("{label}-request.json")
            } else {
                format!("{label}-{attempt}-request.json")
            })
        };
        let attempts = (0..6)
            .rev()
            .filter(|i| path_for(*i).exists())
            .chain((0..6).filter(|i| !path_for(*i).exists()))
            .collect::<Vec<_>>();
        for attempt in attempts {
            let identity = if attempt == 0 {
                label.to_owned()
            } else {
                format!("{label}-{attempt}")
            };
            let request = self.root.join(format!("{identity}-request.json"));
            if !request.exists() {
                self.call(
                    &format!("{identity}-prepare"),
                    args![
                        "invocation",
                        "prepare",
                        capability,
                        operation,
                        "--host",
                        "host:slotbook-test",
                        "--request-id",
                        label,
                        "--inputs",
                        inputs.display(),
                        "--output",
                        request.display()
                    ],
                    Expected::Success,
                    caller,
                )?;
            }
            let accepted = self.call(
                &format!("{identity}-submit"),
                args!["invocation", "submit", request.display()],
                Expected::PreAcceptanceRetry,
                caller,
            )?;
            if accepted
                .pointer("/status")
                .and_then(serde_json::Value::as_str)
                == Some("success")
            {
                self.requests.insert(label.into(), request);
                return Ok(text(
                    accepted
                        .pointer("/value/execution")
                        .ok_or("missing /value/execution")?,
                )?
                .into());
            }
        }
        Err("catalog changed across six explicit refusals without acceptance".into())
    }
    pub(super) fn start(&mut self) -> EvidenceResult {
        ensure(
            self.stop_failure.is_none(),
            "session retains failed shutdown evidence",
        )?;
        ensure(
            self.children.is_empty(),
            "qualification daemons already running",
        )?;
        match self.start_children() {
            Ok(()) => Ok(()),
            Err(error) => self.finish(Err(error)),
        }
    }

    fn start_children(&mut self) -> EvidenceResult {
        self.starts = self
            .starts
            .checked_add(1)
            .ok_or("session start count overflow")?;
        for (name, config) in self
            .origin
            .as_ref()
            .map(|(path, _)| ("origin", path))
            .into_iter()
            .chain(std::iter::once(("daemon", &self.config)))
        {
            let log = fs::File::create(self.logs.join(format!("{name}-{}.log", self.starts)))?;
            self.children.push(OwnedChild::spawn(
                Command::new(&self.daemon)
                    .arg("--config")
                    .arg(config)
                    .stdout(log.try_clone()?)
                    .stderr(log)
                    .stdin(Stdio::null()),
            )?);
        }
        for _ in 0..200 {
            for child in &mut self.children {
                ensure(
                    child.try_wait()?.is_none(),
                    "daemon exited before readiness",
                )?;
            }
            let ready = |caller| -> EvidenceResult<bool> {
                Ok(run_command(
                    self.command(caller).args([
                        "--json",
                        "--timeout-secs",
                        "2",
                        "daemon",
                        "readiness",
                    ]),
                    None,
                    Duration::from_secs(5),
                )?
                .status
                .success())
            };
            if ready(Caller::Operator)? && (self.origin.is_none() || ready(Caller::Origin)?) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("qualification readiness deadline".into())
    }
    pub(super) fn stop(&mut self) -> EvidenceResult {
        if let Some(failure) = &self.stop_failure {
            return Err(failure.clone().into());
        }
        let mut failures = Vec::new();
        for (index, child) in self.children.iter_mut().enumerate() {
            if let Err(error) = child.shutdown() {
                let cleanup = match child.terminate() {
                    Ok(()) => "forced cleanup=reaped".to_owned(),
                    Err(error) => format!("forced cleanup unconfirmed: {error}"),
                };
                failures.push(format!("child {index} shutdown failed: {error}; {cleanup}"));
            }
        }
        if !failures.is_empty() {
            let failure = failures.join("; ");
            self.stop_failure = Some(failure.clone());
            return Err(failure.into());
        }
        self.children.clear();
        Ok(())
    }

    pub(super) fn finish<T>(&mut self, outcome: EvidenceResult<T>) -> EvidenceResult<T> {
        match (outcome, self.stop()) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(format!("{error}; {cleanup}").into()),
        }
    }
}

#[cfg(all(test, unix))]
mod lifecycle_tests {
    use super::*;

    fn failed_child_session(root: &Path) -> EvidenceResult<Session> {
        let token = root.join("token");
        fs::write(&token, b"fixture")?;
        let mut session = Session::resume(
            root,
            Path::new("/bin/true"),
            Path::new("/bin/true"),
            1,
            root,
            &token,
        )?;
        let mut exited = OwnedChild::spawn(Command::new("/bin/false").stdin(Stdio::null()))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while exited.try_wait()?.is_none() {
            ensure(
                std::time::Instant::now() < deadline,
                "fixture child did not exit",
            )?;
            thread::yield_now();
        }
        session.children.push(exited);
        session
            .children
            .push(OwnedChild::spawn(Command::new("/bin/sleep").arg("30"))?);
        Ok(session)
    }

    #[test]
    fn failed_shutdown_still_reaps_other_children_and_retains_failure() -> EvidenceResult {
        let root = tempfile::tempdir()?;
        let mut session = failed_child_session(root.path())?;
        let first = session
            .stop()
            .err()
            .ok_or("failed daemon shutdown accepted")?;
        for child in &mut session.children {
            assert!(
                child.try_wait()?.is_some(),
                "later owned child left running"
            );
        }
        let repeated = session.stop().err().ok_or("repeated stop erased failure")?;
        assert_eq!(first.to_string(), repeated.to_string());
        Ok(())
    }

    #[test]
    fn failed_scenario_retains_shutdown_evidence() -> EvidenceResult {
        let root = tempfile::tempdir()?;
        let mut session = failed_child_session(root.path())?;
        let error = session
            .finish::<()>(Err("injected scenario failure".into()))
            .err()
            .ok_or("failed scenario accepted")?;
        let detail = error.to_string();
        assert!(detail.contains("injected scenario failure"));
        assert!(
            detail.contains("shutdown"),
            "cleanup evidence lost: {detail}"
        );
        Ok(())
    }
}
