use super::{Specification, Violation, checks, require};
use milkdrift_evidence::{EvidenceResult, application::run_command};
use reqwest::{Method, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path, process::Command, thread, time::Duration};

pub(super) struct Service {
    pub(super) spec: Specification,
    pub(super) token: String,
    pub(super) identity: Option<String>,
    base: String,
    client: Client,
}

pub(super) fn podman(args: &[String]) -> EvidenceResult<String> {
    let result = run_command(
        Command::new("/usr/bin/podman").args(args),
        None,
        Duration::from_secs(60),
    )?;
    if !result.status.success() {
        return Err("container observation unavailable".into());
    }
    require(
        result.stdout.len() <= 1_048_576,
        "container observation exceeds bound",
    )?;
    Ok(result.stdout)
}
pub(super) fn permissions(path: &Path, mode: u32) -> EvidenceResult {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err("Slotbook container verification requires Linux".into())
    }
}

pub(super) fn remove_cidfile(path: &Path) -> EvidenceResult {
    // Podman may remove its recorded cidfile with the container. Both observed absence and
    // successful unlink permit recreation; any other filesystem failure remains unknown.
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
impl Service {
    pub(super) fn prepare(spec: Specification) -> EvidenceResult<Self> {
        let token = fs::read_to_string(&spec.token_file)?.trim().to_owned();
        require(
            !token.is_empty() && token.len() <= 1024,
            "invalid verifier credential",
        )?;
        let config = spec.directory.join("config");
        fs::create_dir(&config)?;
        permissions(&config, 0o755)?;
        let data = spec.directory.join("data");
        fs::create_dir(&data)?;
        permissions(&data, 0o777)?;
        for (name, bytes) in [
            ("application.json", serde_json::to_vec(&spec.application)?),
            ("token", token.as_bytes().to_vec()),
            ("clock", b"2027-04-10T09:00:00Z".to_vec()),
        ] {
            fs::write(config.join(name), bytes)?;
            permissions(&config.join(name), 0o444)?;
        }
        Ok(Self {
            spec,
            token,
            identity: None,
            base: String::new(),
            client: Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(Policy::none())
                .no_proxy()
                .build()?,
        })
    }
    pub(super) fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        authorization: Option<&str>,
    ) -> EvidenceResult<(u16, Value)> {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(token) = authorization {
            request = request.bearer_auth(token);
        }
        let response = request.send()?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response.take(16_385).read_to_end(&mut bytes)?;
        require(bytes.len() <= 16_384, "candidate response exceeds bound")?;
        require(
            !bytes
                .windows(self.token.len())
                .any(|part| part == self.token.as_bytes()),
            "candidate disclosed service credential",
        )?;
        Ok((
            status,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes)?
            },
        ))
    }
    pub(super) fn launch(&mut self) -> EvidenceResult {
        let s = &self.spec;
        let lim = &s.limits;
        let integer = |name| {
            lim.get(name)
                .and_then(Value::as_u64)
                .ok_or("container limit absent")
        };
        let args = vec![
            "run".into(),
            "--detach".into(),
            "--pull=never".into(),
            "--replace=false".into(),
            format!("--name={}", s.container),
            format!("--cidfile={}", s.directory.join("container.cid").display()),
            format!("--label=org.milkdrift.verification={}", s.evaluation),
            format!("--label=org.milkdrift.platform={}", s.platform_owner),
            "--log-driver=none".into(),
            "--read-only".into(),
            "--read-only-tmpfs=false".into(),
            format!("--tmpfs=/tmp:rw,size={}", integer("temporary_bytes")?),
            "--cap-drop=all".into(),
            "--security-opt=no-new-privileges".into(),
            "--userns=auto:size=65536".into(),
            "--network=pasta:--no-map-gw".into(),
            "--publish=127.0.0.1::8080".into(),
            format!("--memory={}", integer("memory_bytes")?),
            format!("--memory-swap={}", integer("memory_bytes")?),
            format!(
                "--cpus={}.{:02}",
                integer("cpu_percent")? / 100,
                integer("cpu_percent")? % 100
            ),
            format!("--pids-limit={}", integer("pids")?),
            format!("--volume={}:/candidate/app:ro", s.candidate.display()),
            format!(
                "--volume={}:/config:ro",
                s.directory.join("config").display()
            ),
            format!("--volume={}:/data:U", s.directory.join("data").display()),
            "--entrypoint=/candidate/app".into(),
            s.image.clone(),
        ];
        podman(&args)?;
        let identity = fs::read_to_string(s.directory.join("container.cid"))?
            .trim()
            .to_owned();
        require(
            identity.len() == 64 && identity.bytes().all(|b| b.is_ascii_hexdigit()),
            "container identity invalid",
        )?;
        self.identity = Some(identity.clone());
        let port = podman(&["port".into(), identity, "8080/tcp".into()])?;
        let port: u16 = port
            .trim()
            .rsplit(':')
            .next()
            .ok_or("container port absent")?
            .parse()?;
        self.base = format!("http://127.0.0.1:{port}");
        Ok(())
    }
    pub(super) fn ready(&self) -> EvidenceResult {
        for _ in 0..100 {
            if self
                .request(Method::GET, "/health", None, None)
                .is_ok_and(|r| r.0 == 200)
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err("candidate readiness not observed".into())
    }
    pub(super) fn inspect(&self) -> EvidenceResult<Value> {
        let identity = self.identity.as_ref().ok_or("container not observed")?;
        let value: Value = serde_json::from_str(&podman(&["inspect".into(), identity.clone()])?)?;
        value
            .get(0)
            .cloned()
            .ok_or_else(|| "container inspection absent".into())
    }
    pub(super) fn clock(&self, value: &str) -> EvidenceResult {
        let path = self.spec.directory.join("config/clock");
        permissions(&path, 0o644)?;
        fs::write(&path, value)?;
        permissions(&path, 0o444)
    }
    pub(super) fn cleanup(&mut self) -> EvidenceResult {
        if self.identity.is_none() && self.spec.directory.join("container.cid").exists() {
            self.identity = Some(
                fs::read_to_string(self.spec.directory.join("container.cid"))?
                    .trim()
                    .to_owned(),
            );
        }
        if let Some(identity) = self.identity.as_ref() {
            let state = self.inspect()?;
            require(
                state
                    .pointer("/Config/Labels/org.milkdrift.platform")
                    .and_then(Value::as_str)
                    == Some(&self.spec.platform_owner)
                    && state
                        .pointer("/Config/Labels/org.milkdrift.verification")
                        .and_then(Value::as_str)
                        == Some(&self.spec.evaluation),
                "verifier cleanup ownership differs",
            )?;
            podman(&[
                "rm".into(),
                "--force".into(),
                "--time=0".into(),
                identity.clone(),
            ])?;
            self.identity = None;
            remove_cidfile(&self.spec.directory.join("container.cid"))?;
        }
        Ok(())
    }
    pub(super) fn observe(&mut self) -> EvidenceResult<Vec<Value>> {
        self.launch()?;
        self.ready()?;
        let mut observations = Vec::new();
        for name in self.spec.required_checks.clone() {
            let result = checks::observe(self, &name);
            let (passed, diagnostic) = match result {
                Ok(()) => (Some(true), "observed declared check".to_owned()),
                Err(error) if error.downcast_ref::<Violation>().is_some() => {
                    (Some(false), error.to_string())
                }
                Err(_) => (None, "declared check could not be observed".to_owned()),
            };
            observations.push(json!({"name":name,"passed":passed,"diagnostic":diagnostic}));
        }
        Ok(observations)
    }
}
