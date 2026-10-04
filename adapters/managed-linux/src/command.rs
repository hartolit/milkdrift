//! Bounded helper ownership: one child/group, nonblocking pipes, one deadline and no detached readers.
use crate::{platform_error, rejected};
use milkdrift_capability_host::managed::ManagedError;
use std::{path::Path, sync::atomic::AtomicBool, time::Duration};
#[cfg(target_os = "linux")]
use std::{
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::Instant,
};

// Administrative metadata/cleanup helpers have a separate bounded wait from configured task and
// service lifetimes. Calls that wait for service shutdown pass that grace explicitly.
pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(45);
// Engine inspection and fixed protection probes are bounded control-plane documents.
pub(crate) const HELPER_OUTPUT_BYTES: usize = 1_048_576;
// A failed helper gets one short force/reap allowance; it cannot turn a request timeout
// into an unbounded wait. Failure to observe exit remains an explicit platform error.
#[cfg(target_os = "linux")]
const HELPER_REAP_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) fn run(
    program: &Path,
    args: &[String],
    extra_env: &[(&str, &str)],
    timeout: Duration,
    limit: usize,
    cancel: Option<&AtomicBool>,
) -> Result<CommandOutput, ManagedError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (program, args, extra_env, timeout, limit, cancel);
        Err(rejected(
            "managed setup requires Linux with rootless Podman and user systemd",
        ))
    }
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
        use std::{io::Read, os::unix::process::CommandExt};
        let home = std::env::var("HOME").map_err(rejected)?;
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .map_err(|_| rejected("an active systemd user session is required"))?;
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={runtime}/bus"),
            )
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let child = command.spawn().map_err(|e| {
            rejected(format!(
                "required helper {} unavailable: {:?}",
                program.display(),
                e.kind()
            ))
        })?;
        let mut owned = OwnedChild {
            child,
            reaped: false,
        };
        // Contain unwinding inside the process owner so every post-spawn exit reaches
        // the same fallible cleanup path, including partial pipe setup.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut stdout = owned
                .child
                .stdout
                .take()
                .ok_or_else(|| platform_error("helper stdout missing"))?;
            let mut stderr = owned
                .child
                .stderr
                .take()
                .ok_or_else(|| platform_error("helper stderr missing"))?;
            fcntl_setfl(
                &stdout,
                fcntl_getfl(&stdout).map_err(platform_error)? | OFlags::NONBLOCK,
            )
            .map_err(platform_error)?;
            fcntl_setfl(
                &stderr,
                fcntl_getfl(&stderr).map_err(platform_error)? | OFlags::NONBLOCK,
            )
            .map_err(platform_error)?;
            let deadline = Instant::now()
                .checked_add(timeout)
                .ok_or_else(|| rejected("helper timeout cannot be represented"))?;
            let mut out = Vec::new();
            let mut err = Vec::new();
            let mut out_eof = false;
            let mut err_eof = false;
            let mut exited = false;
            loop {
                if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
                    return Err(platform_error(
                        "helper cancelled; external resource stop still requires inspection",
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(platform_error(
                        "helper deadline exceeded; external outcome remains uncertain",
                    ));
                }
                let read_pipe = |pipe: &mut dyn Read,
                                 bytes: &mut Vec<u8>,
                                 eof: &mut bool,
                                 other_bytes: usize|
                 -> Result<(), ManagedError> {
                    let mut chunk = [0; 8192];
                    // Finite reads per pass keep cancellation/deadline checks responsive under output flood.
                    for _ in 0..16 {
                        let remaining =
                            limit.saturating_sub(other_bytes.saturating_add(bytes.len()));
                        let read_bound = chunk.len().min(remaining.saturating_add(1));
                        match pipe.read(
                            chunk
                                .get_mut(..read_bound)
                                .ok_or_else(|| platform_error("helper read bound exceeded"))?,
                        ) {
                            Ok(0) => {
                                *eof = true;
                                break;
                            }
                            Ok(n) => {
                                if n > remaining {
                                    return Err(platform_error("helper output limit exceeded"));
                                }
                                bytes.extend_from_slice(chunk.get(..n).ok_or_else(|| {
                                    platform_error("helper read exceeded its buffer")
                                })?);
                            }
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::Interrupted
                                ) =>
                            {
                                break;
                            }
                            Err(e) => return Err(platform_error(e)),
                        }
                    }
                    Ok(())
                };
                read_pipe(&mut stdout, &mut out, &mut out_eof, err.len())?;
                read_pipe(&mut stderr, &mut err, &mut err_eof, out.len())?;
                if !exited {
                    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
                    // Keep the leader unreaped until pipes close. Its reserved PID prevents a timeout
                    // cleanup from targeting a recycled process group after the helper exits first.
                    exited = waitid(
                        WaitId::Pid(Pid::from_child(&owned.child)),
                        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
                    )
                    .map_err(platform_error)?
                    .is_some();
                }
                if exited && out_eof && err_eof {
                    let status = owned.child.wait().map_err(platform_error)?;
                    owned.reaped = true;
                    return Ok(CommandOutput {
                        success: status.success(),
                        exit_code: status.code(),
                        stdout: out,
                        stderr: err,
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }))
        .unwrap_or_else(|_| {
            Err(platform_error(
                "helper observation panicked; external outcome remains uncertain",
            ))
        });
        owned.finish(outcome)
    }
}

#[cfg(target_os = "linux")]
struct OwnedChild {
    child: std::process::Child,
    reaped: bool,
}
#[cfg(target_os = "linux")]
impl OwnedChild {
    fn finish(
        &mut self,
        outcome: Result<CommandOutput, ManagedError>,
    ) -> Result<CommandOutput, ManagedError> {
        if self.reaped {
            return outcome;
        }
        let group = rustix::process::kill_process_group(
            rustix::process::Pid::from_child(&self.child),
            rustix::process::Signal::KILL,
        )
        .or_else(|error| {
            if error == rustix::io::Errno::SRCH {
                Ok(())
            } else {
                Err(error)
            }
        });
        let kill = self.child.kill();
        let reaped = wait_for_reap(HELPER_REAP_TIMEOUT, || self.child.try_wait());
        self.reaped = reaped.is_ok();
        if group.is_err() || reaped.is_err() {
            return Err(platform_error(format!(
                "helper cleanup unconfirmed: group={group:?}, kill={kill:?}, reap={reaped:?}; original={}",
                outcome
                    .as_ref()
                    .err()
                    .map_or_else(|| "completed helper".to_owned(), ToString::to_string),
            )));
        }
        outcome
    }
}

#[cfg(target_os = "linux")]
fn wait_for_reap(
    timeout: Duration,
    mut observe: impl FnMut() -> std::io::Result<Option<std::process::ExitStatus>>,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match observe() {
            Ok(Some(_)) => return Ok(()),
            Err(error) => return Err(format!("helper wait failed: {:?}", error.kind())),
            Ok(None) if started.elapsed() >= timeout => {
                return Err("helper reap deadline exceeded".to_owned());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

pub(crate) fn checked(program: &str, args: &[String]) -> Result<Vec<u8>, ManagedError> {
    checked_with_timeout(program, args, HELPER_TIMEOUT)
}

pub(crate) fn checked_with_timeout(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<Vec<u8>, ManagedError> {
    let output = run(
        Path::new(program),
        args,
        &[],
        timeout,
        HELPER_OUTPUT_BYTES,
        None,
    )?;
    if !output.success {
        return Err(platform_error(format!(
            "{} failed: {}",
            program,
            milkdrift_contracts::truncate_utf8(&String::from_utf8_lossy(&output.stderr), 256)
        )));
    }
    Ok(output.stdout)
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
