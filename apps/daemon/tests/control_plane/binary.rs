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
        let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
        config.bind = socket.local_addr()?;
        drop(socket);
        let endpoint = Url::parse(&format!("http://{}/", config.bind))?;
        let path = directory.path().join("daemon.toml");
        fs::write(&path, toml::to_string(&config)?)?;
        Ok((Self::start(&path, endpoint, directory.path()).await?, path))
    }
    pub(super) async fn start(config: &Path, endpoint: Url, directory: &Path) -> TestResult<Self> {
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_milkdrift-daemon"))
            .args(["--config"])
            .arg(config)
            .stdout(fs::File::create(directory.join("daemon.stdout"))?)
            .stderr(fs::File::create(directory.join("daemon.stderr"))?)
            .spawn()?;
        let mut owner = ChildOwner(Some(child));
        let client = client(&endpoint, CONTROLLER_TOKEN)?;
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
            if matches!(
                tokio::time::timeout_at(deadline, client.readiness()).await,
                Ok(Ok(_))
            ) {
                return Ok(Self {
                    child: owner,
                    endpoint,
                    client,
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
