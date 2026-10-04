//! Own a real daemon process for independent HTTP and CLI clients.
use super::{inputs::ModelFixture, support::*};
use std::{io, path::Path, process::Child, time::Instant};

pub(super) struct ChildOwner(pub(super) Option<Child>);
impl ChildOwner {
    pub(super) fn finish(&mut self) -> io::Result<()> {
        let Some(child) = self.0.as_mut() else {
            return Ok(());
        };
        if child.try_wait()?.is_some() {
            self.0 = None;
            return Ok(());
        }
        // Always attempt to reap, including when kill reports an error racing with exit.
        let killed = child.kill();
        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                self.0 = None;
                return Ok(());
            }
            if started.elapsed() >= Duration::from_secs(5) {
                return Err(io::Error::other(format!(
                    "fixture child exit unconfirmed; kill: {killed:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            if std::thread::panicking() {
                #[expect(
                    clippy::print_stderr,
                    reason = "Report unconfirmed fixture cleanup without a second panic during test unwinding."
                )]
                {
                    eprintln!("fixture child cleanup: {error}");
                }
            } else {
                #[expect(
                    clippy::panic,
                    reason = "A fixture destructor must fail the test if ordinary cleanup did not prove the child exited; unwinding uses diagnostics instead."
                )]
                {
                    panic!("fixture child cleanup: {error}");
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn child_cleanup_reaps_a_running_child_and_can_be_repeated() -> TestResult {
    let mut owner = ChildOwner(Some(
        std::process::Command::new("/bin/sleep").arg("60").spawn()?,
    ));
    assert!(
        owner
            .0
            .as_mut()
            .ok_or("child absent")?
            .try_wait()?
            .is_none()
    );
    owner.finish()?;
    assert!(owner.0.is_none());
    owner.finish()?;
    Ok(())
}

pub(super) struct BinaryDaemon {
    child: ChildOwner,
    pub(super) endpoint: Url,
    pub(super) client: ControlClient,
}

#[tokio::test]
async fn daemon_selects_and_reports_its_own_listening_port() -> TestResult {
    let directory = TempDir::new()?;
    let config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
    assert_eq!(config.bind.port(), 0);
    let path = directory.path().join("daemon.toml");
    fs::write(&path, toml::to_string(&config)?)?;
    // The child owns port allocation. No probe listener is released between choosing
    // the address and binding it, even when other fixtures start concurrently.
    let daemon =
        BinaryDaemon::start(&path, Url::parse("http://127.0.0.1:0/")?, directory.path()).await?;
    assert!(daemon.endpoint.port().is_some_and(|port| port != 0));
    assert!(daemon.client.readiness().await?.ready);
    daemon.stop()?;
    Ok(())
}

#[tokio::test]
async fn failed_startup_retains_evidence_and_reaps_child() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("invalid.toml");
    fs::write(&path, "invalid = [")?;
    let started = Instant::now();
    let error = BinaryDaemon::start(&path, Url::parse("http://127.0.0.1:1/")?, directory.path())
        .await
        .err()
        .ok_or("invalid configuration unexpectedly started")?
        .to_string();
    assert!(error.contains("daemon exited before readiness"), "{error}");
    assert!(error.contains("diagnostics:"), "{error}");
    assert!(error.contains("cleanup: Ok(())"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(6));
    Ok(())
}
impl BinaryDaemon {
    pub(super) fn stop(mut self) -> TestResult {
        self.child.finish()?;
        Ok(())
    }
    pub(super) async fn configured(
        directory: &TempDir,
        model: &ModelFixture,
    ) -> TestResult<(Self, std::path::PathBuf)> {
        let mut config = super::authoring::model_configuration_document(directory, model.address)?;
        config
            .actors
            .first_mut()
            .ok_or("controller absent")?
            .authority
            .resources
            .workflow_run = milkdrift_authority::WorkflowRunScope::Workflows {
            workflows: milkdrift_authority::WorkflowSet::new([
                WorkflowId::new("release-notes")?,
                WorkflowId::new("independent-notes")?,
            ])?,
        };
        config.bind.set_port(0);
        let endpoint = Url::parse(&format!("http://{}/", config.bind))?;
        let path = directory.path().join("daemon.toml");
        fs::write(&path, toml::to_string(&config)?)?;
        let daemon = Self::start(&path, endpoint, directory.path()).await?;
        // Reopen the same endpoint after stopping this owned child. The initial port
        // was allocated by its actual listener, not by a released probe socket.
        config
            .bind
            .set_port(daemon.endpoint.port().ok_or("listener port absent")?);
        fs::write(&path, toml::to_string(&config)?)?;
        Ok((daemon, path))
    }
    pub(super) async fn start(
        config: &Path,
        mut endpoint: Url,
        directory: &Path,
    ) -> TestResult<Self> {
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_milkdrift-daemon"))
            .args(["--config"])
            .arg(config)
            .env("RUST_LOG", "info")
            .stdout(fs::File::create(directory.join("daemon.stdout"))?)
            .stderr(fs::File::create(directory.join("daemon.stderr"))?)
            .spawn()?;
        let mut owner = ChildOwner(Some(child));
        let mut control = if endpoint.port() == Some(0) {
            None
        } else {
            Some(client(&endpoint, CONTROLLER_TOKEN)?)
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if let Some(status) = owner.0.as_mut().ok_or("child absent")?.try_wait()? {
                let cleanup = owner.finish();
                return Err(format!(
                    "daemon exited before readiness ({status}); {}; cleanup: {cleanup:?}",
                    super::diagnostics::child_failure(directory)
                )
                .into());
            }
            if control.is_none() {
                match listening_endpoint(directory) {
                    Ok(Some(bound)) => {
                        endpoint = bound;
                        control = Some(client(&endpoint, CONTROLLER_TOKEN)?);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let cleanup = owner.finish();
                        return Err(format!(
                            "daemon listener discovery failed: {error}; {}; cleanup: {cleanup:?}",
                            super::diagnostics::child_failure(directory)
                        )
                        .into());
                    }
                }
            }
            if let Some(client) = &control
                && matches!(
                    tokio::time::timeout_at(deadline, client.readiness()).await,
                    Ok(Ok(_))
                )
            {
                return Ok(Self {
                    child: owner,
                    endpoint,
                    client: control.take().ok_or("ready client absent")?,
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let cleanup = owner.finish();
        Err(format!(
            "daemon readiness deadline; {}; cleanup: {cleanup:?}",
            super::diagnostics::child_failure(directory)
        )
        .into())
    }
}

fn listening_endpoint(directory: &Path) -> TestResult<Option<Url>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    fs::File::open(directory.join("daemon.stdout"))?
        .take(65_536)
        .read_to_end(&mut bytes)?;
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if value
            .pointer("/fields/phase")
            .and_then(serde_json::Value::as_str)
            != Some("listening")
        {
            continue;
        }
        let address: std::net::SocketAddr = value
            .pointer("/fields/address")
            .and_then(serde_json::Value::as_str)
            .ok_or("listener address absent")?
            .parse()?;
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err("invalid fixture listener address".into());
        }
        return Ok(Some(Url::parse(&format!("http://{address}/"))?));
    }
    Ok(None)
}
