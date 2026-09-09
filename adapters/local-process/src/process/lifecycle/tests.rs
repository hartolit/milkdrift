use super::*;
use std::{
    ffi::OsString,
    io::{Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

#[path = "../../../tests/support/process_cleanup.rs"]
mod process_cleanup;
use process_cleanup::{ProbeCleanup, process_alive};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const DEADLINE: Duration = Duration::from_secs(5);

// The test executable supplies a real pipe-holding child without requiring an
// independently built helper binary for `cargo test --lib`.
#[test]
fn pipe_holder() -> TestResult {
    let Some(marker) = std::env::var_os("MILKDRIFT_IO_CLEANUP_PROBE") else {
        return Ok(());
    };
    std::fs::write(marker, std::process::id().to_string())?;
    std::io::stdout().write_all(&vec![b'o'; 256 * 1024])?;
    std::io::stderr().write_all(&vec![b'e'; 256 * 1024])?;
    thread::sleep(Duration::from_secs(30));
    Ok(())
}

struct Completion {
    index: usize,
    waiting: SyncSender<usize>,
    release: Receiver<()>,
    completed: Arc<AtomicUsize>,
}

impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self.waiting.send(self.index);
        let _ = self.release.recv_timeout(DEADLINE);
        self.completed.fetch_add(1, Ordering::SeqCst);
    }
}

struct TrackedPipe<T> {
    pipe: T,
    _completion: Completion,
}

impl<T: Read> Read for TrackedPipe<T> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.pipe.read(bytes)
    }
}

impl<T: Write> Write for TrackedPipe<T> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pipe.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.pipe.flush()
    }
}

fn cleanup_workers(worker_count: usize, unwind: bool) -> TestResult {
    let directory = tempfile::tempdir()?;
    let marker = directory.path().join("child.pid");
    let pid_path = directory.path().join("cleanup.pid");
    let fallback = ProbeCleanup(pid_path.clone());
    let child = crate::process::spawn::spawn(
        &std::env::current_exe()?,
        directory.path(),
        &[
            OsString::from("--exact"),
            OsString::from("process::lifecycle::tests::pipe_holder"),
            OsString::from("--nocapture"),
        ],
        &[(
            OsString::from("MILKDRIFT_IO_CLEANUP_PROBE"),
            marker.clone().into_os_string(),
        )],
        true,
    )?;
    let pid = child.id();
    let mut process = RunningProcess::new(child, Duration::from_millis(100));
    std::fs::write(pid_path, pid.to_string())?;
    let active = Arc::new(Mutex::new(BTreeMap::new()));
    let invocation = InvocationId::new("cleanup-workers")?;
    process.register(active.clone(), invocation.clone())?;
    let (waiting_sender, waiting) = mpsc::sync_channel(3);
    let completed = Arc::new(AtomicUsize::new(0));
    let mut releases = Vec::new();
    let mut completion = |index| {
        let (release, receiver) = mpsc::sync_channel(1);
        releases.push(release);
        Completion {
            index,
            waiting: waiting_sender.clone(),
            release: receiver,
            completed: completed.clone(),
        }
    };
    process.stdin = spawn_stdin_writer(
        Some(TrackedPipe {
            pipe: process.child.stdin.take().ok_or("missing stdin")?,
            _completion: completion(0),
        }),
        Some(vec![b'i'; 2 * 1024 * 1024]),
    )?;
    let (sender, receiver) = mpsc::sync_channel(1);
    process.receiver = Some(receiver);
    if worker_count >= 2 {
        process.stdout = Some(spawn_reader(
            Stream::Stdout,
            TrackedPipe {
                pipe: process.child.stdout.take().ok_or("missing stdout")?,
                _completion: completion(1),
            },
            1024 * 1024,
            sender.clone(),
        )?);
    }
    if worker_count >= 3 {
        process.stderr = Some(spawn_reader(
            Stream::Stderr,
            TrackedPipe {
                pipe: process.child.stderr.take().ok_or("missing stderr")?,
                _completion: completion(2),
            },
            1024 * 1024,
            sender,
        )?);
    }
    let ready_deadline = Instant::now() + DEADLINE;
    while !marker.exists() && Instant::now() < ready_deadline {
        thread::sleep(Duration::from_millis(5));
    }
    // This channel remains full until cleanup disconnects it. Reader completion
    // gates distinguish joining from merely dropping JoinHandles.
    let (finished_sender, finished) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _process = process;
            if unwind {
                std::panic::resume_unwind(Box::new("cleanup probe"));
            }
        }));
        let _ = finished_sender.send(result.is_err());
    });
    let evidence = (|| -> TestResult {
        for _ in 0..worker_count {
            waiting.recv_timeout(DEADLINE)?;
        }
        assert!(
            !process_alive(pid)?,
            "child must exit before I/O joins finish"
        );
        assert!(finished.try_recv().is_err(), "cleanup detached I/O workers");
        assert_eq!(completed.load(Ordering::SeqCst), 0);
        assert!(
            active
                .lock()
                .map_err(|_| "active lock poisoned")?
                .contains_key(&invocation)
        );
        Ok(())
    })();
    for release in releases {
        let _ = release.send(());
    }
    drop(fallback);
    let result = finished.recv_timeout(DEADLINE);
    if result.is_ok() {
        worker.join().map_err(|_| "cleanup worker panicked")?;
    }
    evidence?;
    assert_eq!(result?, unwind);
    assert_eq!(completed.load(Ordering::SeqCst), worker_count);
    assert!(
        active
            .lock()
            .map_err(|_| "active lock poisoned")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn cleanup_joins_all_started_io_before_unregistering() -> TestResult {
    for worker_count in 1..=3 {
        cleanup_workers(worker_count, false)?;
    }
    Ok(())
}

#[test]
fn unwinding_joins_all_io_before_unregistering() -> TestResult {
    cleanup_workers(3, true)
}

#[test]
fn duplicate_registration_preserves_the_original_control() -> TestResult {
    let directory = tempfile::tempdir()?;
    let spawn = || {
        crate::process::spawn::spawn(
            &std::env::current_exe()?,
            directory.path(),
            &[OsString::from("--list")],
            &[],
            false,
        )
    };
    let mut original = RunningProcess::new(spawn()?, Duration::from_millis(100));
    let mut duplicate = RunningProcess::new(spawn()?, Duration::from_millis(100));
    let active = Arc::new(Mutex::new(BTreeMap::new()));
    let invocation = InvocationId::new("duplicate-control")?;
    original.register(active.clone(), invocation.clone())?;
    assert!(
        duplicate
            .register(active.clone(), invocation.clone())
            .is_err()
    );
    drop(duplicate);
    assert!(Arc::ptr_eq(
        active
            .lock()
            .map_err(|_| "active lock poisoned")?
            .get(&invocation)
            .ok_or("lost registration")?,
        &original.control
    ));
    drop(original);
    assert!(
        active
            .lock()
            .map_err(|_| "active lock poisoned")?
            .is_empty()
    );
    Ok(())
}
