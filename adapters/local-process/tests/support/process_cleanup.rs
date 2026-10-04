#[cfg(windows)]
use std::process::Command;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn read_pids(path: &Path) -> Vec<u32> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.parse().ok())
        .collect()
}

pub(crate) struct ProbeCleanup(pub(crate) PathBuf);

impl Drop for ProbeCleanup {
    fn drop(&mut self) {
        for pid in read_pids(&self.0) {
            // An inspection failure must not suppress the fallback kill attempt.
            if process_alive(pid).unwrap_or(true)
                && let Err(error) = terminate(pid)
            {
                #[expect(
                    clippy::print_stderr,
                    reason = "The fixture unwind guard must expose failed fallback cleanup without a second panic"
                )]
                {
                    eprintln!("fixture process {pid} cleanup failed: {error}");
                }
            }
        }
    }
}

fn terminate(pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("invalid child PID")?;
        // Attempt both routes before examining errors; a missing group must not hide
        // a surviving immediate child, and ESRCH is already evidence of absence.
        for result in [
            rustix::process::kill_process_group(pid, rustix::process::Signal::KILL),
            rustix::process::kill_process(pid, rustix::process::Signal::KILL),
        ] {
            if let Err(error) = result
                && error != rustix::io::Errno::SRCH
            {
                return Err(error.into());
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let output = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output()?;
        if !output.status.success() {
            return Err("taskkill did not confirm fixture termination".into());
        }
        Ok(())
    }
}

pub(crate) fn process_alive(pid: u32) -> Result<bool, Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("invalid child PID")?;
        match rustix::process::test_kill_process(pid) {
            Ok(()) => Ok(true),
            Err(rustix::io::Errno::SRCH) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()?;
        if !output.status.success() {
            return Err("tasklist inspection failed".into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
    }
}
