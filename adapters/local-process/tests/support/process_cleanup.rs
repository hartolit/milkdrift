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
            if process_alive(pid).unwrap_or(true) {
                #[cfg(unix)]
                if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
                }
                #[cfg(windows)]
                let _ = Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output();
            }
        }
    }
}

pub(crate) fn process_alive(pid: u32) -> Result<bool, Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        let pid = rustix::process::Pid::from_raw(pid as i32).ok_or("invalid child PID")?;
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
