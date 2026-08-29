use std::{
    collections::BTreeMap,
    process::Child,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use milkdrift_capability::InvocationId;

pub(super) struct ProcessControl {
    pub(super) cancel_requested: AtomicBool,
    #[cfg(unix)]
    process_group: rustix::process::Pid,
}

impl ProcessControl {
    pub(super) fn new(child: &Child) -> Self {
        Self {
            cancel_requested: AtomicBool::new(false),
            #[cfg(unix)]
            process_group: rustix::process::Pid::from_child(child),
        }
    }

    pub(super) fn request_graceful(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            signal_group(self.process_group, rustix::process::Signal::TERM)
        }
        #[cfg(not(unix))]
        {
            Ok(())
        }
    }

    pub(super) fn request_force(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            signal_group(self.process_group, rustix::process::Signal::KILL)
        }
        #[cfg(not(unix))]
        {
            Ok(())
        }
    }

    pub(super) fn group_absent(&self) -> bool {
        #[cfg(unix)]
        {
            match rustix::process::test_kill_process_group(self.process_group) {
                Ok(()) => false,
                Err(error) => error == rustix::io::Errno::SRCH,
            }
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

#[cfg(unix)]
fn signal_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<(), String> {
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::SRCH => Ok(()),
        Err(error) => Err(format!("process-group signal failed: {error}")),
    }
}

pub(super) struct ActiveRegistration {
    active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
    invocation: InvocationId,
}

impl ActiveRegistration {
    pub(super) fn insert(
        active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
        invocation: InvocationId,
        control: Arc<ProcessControl>,
    ) -> Result<Self, String> {
        let mut owners = active
            .lock()
            .map_err(|_error| "process ownership state is unavailable".to_owned())?;
        if owners.insert(invocation.clone(), control).is_some() {
            return Err("invocation already owns a live local process".to_owned());
        }
        drop(owners);
        Ok(Self { active, invocation })
    }
}

impl Drop for ActiveRegistration {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.invocation);
        }
    }
}

pub(super) fn terminate_child_immediately(child: &mut Child, control: &ProcessControl) {
    let _ = control.request_force();
    let _ = child.kill();
    let _ = child.wait();
}

pub(super) fn wait_for_group_absence(control: &ProcessControl, maximum: Duration) -> bool {
    let deadline = Instant::now() + maximum;
    loop {
        if control.group_absent() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
