//! A bounded Rust helper holds the directory inside Podman's rootless user namespace.
use crate::{command, platform_error, rejected};
use milkdrift_capability_host::managed::ManagedError;
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    pid: u32,
    fd: i32,
    identity: String,
}

/// Internal daemon helper role. It opens only the supplied directory, proves its native identity,
/// writes one bounded descriptor handshake and waits for parent EOF or its finite deadline.
///
/// # Errors
/// Refuses replaced roots, unsupported platforms, invalid lifetime or failed protocol I/O.
pub fn run_project_pin_helper(
    folder: &Path,
    identity: &str,
    lifetime_ms: u64,
) -> Result<(), ManagedError> {
    crate::recipe::validate_timeout(lifetime_ms, "project descriptor lifetime")?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (folder, identity);
        Err(rejected("project descriptor helper requires Linux"))
    }
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, fcntl_getfl, fcntl_setfl};
        use std::{
            fs::File,
            io::{Read, Write},
            os::{fd::AsRawFd, unix::fs::MetadataExt},
            time::Instant,
        };
        let directory = File::from(
            rustix::fs::open(
                folder,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(rejected)?,
        );
        let metadata = directory.metadata().map_err(rejected)?;
        if format!("unix:{}:{}", metadata.dev(), metadata.ino()) != identity {
            return Err(rejected(
                "project folder changed before rootless descriptor capture",
            ));
        }
        let ready = Ready {
            pid: std::process::id(),
            fd: directory.as_raw_fd(),
            identity: identity.into(),
        };
        let mut stdout = std::io::stdout().lock();
        serde_json::to_writer(&mut stdout, &ready).map_err(platform_error)?;
        stdout.write_all(b"\n").map_err(platform_error)?;
        stdout.flush().map_err(platform_error)?;
        drop(stdout);
        let mut stdin = std::io::stdin().lock();
        fcntl_setfl(
            &stdin,
            fcntl_getfl(&stdin).map_err(platform_error)? | OFlags::NONBLOCK,
        )
        .map_err(platform_error)?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(lifetime_ms))
            .ok_or_else(|| rejected("project descriptor deadline overflow"))?;
        let mut byte = [0];
        while Instant::now() < deadline {
            match stdin.read(&mut byte) {
                Ok(_) => return Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(platform_error(error)),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(platform_error(
            "project descriptor parent exceeded its finite lifetime",
        ))
    }
}

#[cfg(target_os = "linux")]
pub(super) struct RootlessPin {
    owned: command::OwnedChild,
    pub(super) source: String,
}

#[cfg(target_os = "linux")]
impl RootlessPin {
    pub(super) fn start(
        helper: &Path,
        folder: &Path,
        identity: &str,
        lifetime_ms: u64,
    ) -> Result<Self, ManagedError> {
        use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
        use std::os::unix::fs::MetadataExt;
        use std::{io::Read, process::Stdio, time::Instant};
        let metadata = std::fs::symlink_metadata(helper).map_err(rejected)?;
        if !helper.is_absolute()
            || !metadata.is_file()
            || helper.canonicalize().map_err(rejected)? != helper
            || metadata.len() > 1_073_741_824
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
            || (metadata.uid() != 0 && metadata.uid() != rustix::process::getuid().as_raw())
        {
            return Err(rejected(
                "project pin helper must be a canonical trusted executable without group or other write access",
            ));
        }
        let mut command = command::configured(Path::new("/usr/bin/podman"))?;
        command
            .arg("unshare")
            .arg(helper)
            .arg("--internal-project-pin")
            .arg(folder)
            .arg(identity)
            .arg(lifetime_ms.to_string())
            .stdin(Stdio::piped());
        let child = command.spawn().map_err(platform_error)?;
        let mut pin = Self {
            owned: command::OwnedChild {
                child,
                reaped: false,
            },
            source: String::new(),
        };
        let mut output = pin
            .owned
            .child
            .stdout
            .take()
            .ok_or_else(|| platform_error("project descriptor handshake missing"))?;
        let mut errors = pin
            .owned
            .child
            .stderr
            .take()
            .ok_or_else(|| platform_error("project descriptor diagnostic missing"))?;
        fcntl_setfl(
            &output,
            fcntl_getfl(&output).map_err(platform_error)? | OFlags::NONBLOCK,
        )
        .map_err(platform_error)?;
        fcntl_setfl(
            &errors,
            fcntl_getfl(&errors).map_err(platform_error)? | OFlags::NONBLOCK,
        )
        .map_err(platform_error)?;
        let deadline = Instant::now() + command::HELPER_TIMEOUT;
        let mut bytes = Vec::new();
        let mut diagnostics = Vec::new();
        loop {
            let read =
                |reader: &mut dyn Read, destination: &mut Vec<u8>| -> Result<(), ManagedError> {
                    let mut chunk = [0; 256];
                    match reader.read(&mut chunk) {
                        Ok(length) if destination.len().saturating_add(length) <= 512 => {
                            destination.extend_from_slice(
                                chunk
                                    .get(..length)
                                    .ok_or_else(|| platform_error("project pin output overflow"))?,
                            );
                        }
                        Ok(_) => {
                            return Err(platform_error(
                                "project pin output exceeds bounded handshake",
                            ));
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) => {}
                        Err(error) => return Err(platform_error(error)),
                    }
                    Ok(())
                };
            read(&mut output, &mut bytes)?;
            read(&mut errors, &mut diagnostics)?;
            if bytes.ends_with(b"\n") {
                let ready: Ready = serde_json::from_slice(&bytes).map_err(platform_error)?;
                if ready.pid == 0 || ready.fd < 3 || ready.identity != identity {
                    return Err(platform_error(
                        "project descriptor handshake differs from the accepted root",
                    ));
                }
                pin.source = format!("/proc/{}/fd/{}", ready.pid, ready.fd);
                return Ok(pin);
            }
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
            let exited = waitid(
                WaitId::Pid(Pid::from_child(&pin.owned.child)),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .map_err(platform_error)?
            .is_some();
            if exited || Instant::now() >= deadline {
                return Err(platform_error(format!(
                    "Podman cannot establish the pinned project attachment; configure the daemon's supported rootless helper and mapping: {}",
                    milkdrift_contracts::truncate_utf8(&String::from_utf8_lossy(&diagnostics), 256)
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub(super) fn stop(&mut self) -> Result<(), ManagedError> {
        self.owned.terminate()
    }
}

#[cfg(target_os = "linux")]
impl Drop for RootlessPin {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::error!(%error, "project descriptor helper cleanup remains unconfirmed");
        }
    }
}
