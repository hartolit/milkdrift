//! Own a real daemon process for independent HTTP and CLI clients.
use super::{inputs::ModelFixture, support::*};
use std::{path::Path, process::Child};

pub(super) struct ChildOwner(pub(super) Option<Child>);
impl Drop for ChildOwner {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub(super) struct BinaryDaemon {
    _child: ChildOwner,
    pub(super) endpoint: Url,
    pub(super) client: ControlClient,
}
impl BinaryDaemon {
    pub(super) async fn configured(
        directory: &TempDir,
        model: &ModelFixture,
    ) -> TestResult<(Self, std::path::PathBuf)> {
        let mut config = super::authoring::model_configuration_document(directory, model.address)?;
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
        for _ in 0..100 {
            if owner
                .0
                .as_mut()
                .ok_or("child absent")?
                .try_wait()?
                .is_some()
            {
                return Err("daemon exited before readiness".into());
            }
            if client.readiness().await.is_ok() {
                return Ok(Self {
                    _child: owner,
                    endpoint,
                    client,
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err("daemon readiness deadline".into())
    }
}
