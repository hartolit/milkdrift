use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use milkdrift_authority::SensitiveSecret;

const STREAM_READ_BYTES: usize = 8 * 1024;
pub(super) const IO_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stream {
    Stdout,
    Stderr,
}

pub(super) enum StreamMessage {
    Data(Stream, Vec<u8>),
    Overflow(Stream),
    Closed(Stream),
    Failed(Stream, std::io::ErrorKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum IoCompletion {
    Complete,
    Interrupted,
}

pub(super) type IoWorker = JoinHandle<Result<IoCompletion, String>>;

// The invocation owner requests interruption; workers alone close their pipes.
// Input stops at termination/parent exit, output keeps draining until the shared
// cleanup deadline. Neither OS I/O nor channel backpressure may hide a blocking wait.
#[derive(Default)]
pub(super) struct IoCancellation {
    pub(super) input: Arc<AtomicBool>,
    pub(super) output: Arc<AtomicBool>,
}

fn would_block(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
}

#[cfg(unix)]
pub(super) fn pipe_reader(pipe: impl Read + Send + 'static) -> impl Read + Send + 'static {
    pipe
}

#[cfg(windows)]
pub(super) fn pipe_reader(
    pipe: impl Into<std::os::windows::io::OwnedHandle>,
) -> impl Read + Send + 'static {
    // std's Windows Read maps both idle ERROR_NO_DATA and closed ERROR_BROKEN_PIPE
    // to EOF. Preserve the raw distinction with the dependency's unnamed receiver;
    // it owns one handle and has no background flush/limbo behavior.
    struct Reader(interprocess::unnamed_pipe::Recver);
    impl Read for Reader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            match self.0.read(buffer) {
                Err(error) if error.raw_os_error() == Some(232) => {
                    Err(std::io::ErrorKind::WouldBlock.into())
                }
                Err(error) if matches!(error.raw_os_error(), Some(109 | 233)) => Ok(0),
                result => result,
            }
        }
    }
    Reader(interprocess::unnamed_pipe::Recver::from(pipe.into()))
}

fn send(sender: &SyncSender<StreamMessage>, mut message: StreamMessage, stop: &AtomicBool) -> bool {
    loop {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(message) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(returned)) => {
                message = returned;
                thread::sleep(IO_POLL_INTERVAL);
            }
        }
    }
}

pub(super) fn spawn_reader<R: Read + Send + 'static>(
    stream: Stream,
    mut reader: R,
    maximum: u64,
    sender: SyncSender<StreamMessage>,
    stop: Arc<AtomicBool>,
) -> Result<IoWorker, String> {
    thread::Builder::new()
        .spawn(move || {
            let mut accepted = 0_u64;
            let mut overflow_sent = false;
            let mut buffer = [0_u8; STREAM_READ_BYTES];
            loop {
                if stop.load(Ordering::Acquire) {
                    // One nonblocking probe preserves already available EOF even
                    // when the monitor was delayed persisting a progress report.
                    return Ok(if matches!(reader.read(&mut buffer), Ok(0)) {
                        IoCompletion::Complete
                    } else {
                        IoCompletion::Interrupted
                    });
                }
                let count = match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(error) if would_block(&error) => {
                        thread::sleep(IO_POLL_INTERVAL);
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = send(&sender, StreamMessage::Failed(stream, error.kind()), &stop);
                        return Err(format!("stream read failed: {:?}", error.kind()));
                    }
                };
                let remaining = maximum.saturating_sub(accepted);
                let take = usize::try_from(remaining).unwrap_or(usize::MAX).min(count);
                if take != 0 {
                    if !send(
                        &sender,
                        StreamMessage::Data(stream, buffer[..take].to_vec()),
                        &stop,
                    ) {
                        return Ok(IoCompletion::Interrupted);
                    }
                    accepted = accepted.saturating_add(u64::try_from(take).unwrap_or(u64::MAX));
                }
                if take < count && !overflow_sent {
                    if !send(&sender, StreamMessage::Overflow(stream), &stop) {
                        return Ok(IoCompletion::Interrupted);
                    }
                    overflow_sent = true;
                }
            }
            let _ = send(&sender, StreamMessage::Closed(stream), &stop);
            Ok(IoCompletion::Complete)
        })
        .map_err(|error| format!("stream worker spawn failed: {:?}", error.kind()))
}

pub(super) fn spawn_stdin_writer<W: Write + Send + 'static>(
    stdin: Option<W>,
    bytes: Option<Vec<u8>>,
    stop: Arc<AtomicBool>,
) -> Result<Option<IoWorker>, String> {
    stdin
        .zip(bytes)
        .map(|(mut stdin, bytes)| {
            thread::Builder::new()
                .spawn(move || {
                    let mut remaining = bytes.as_slice();
                    while !remaining.is_empty() {
                        if stop.load(Ordering::Acquire) {
                            return Ok(IoCompletion::Interrupted);
                        }
                        match stdin.write(&remaining[..remaining.len().min(STREAM_READ_BYTES)]) {
                            // A full nonblocking Windows byte pipe can accept zero bytes.
                            Ok(0) => thread::sleep(IO_POLL_INTERVAL),
                            Ok(count) => remaining = &remaining[count..],
                            Err(error) if would_block(&error) => thread::sleep(IO_POLL_INTERVAL),
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(error) => {
                                return Err(format!("stdin write failed: {:?}", error.kind()));
                            }
                        }
                    }
                    // Pipes are unbuffered here. FlushFileBuffers would wait for the child
                    // to consume stdin and would defeat interruption on Windows.
                    Ok(IoCompletion::Complete)
                })
                .map_err(|error| format!("stdin worker spawn failed: {:?}", error.kind()))
        })
        .transpose()
}

pub(super) fn join_io(worker: Option<IoWorker>, name: &str) -> Result<IoCompletion, String> {
    match worker {
        Some(worker) => worker.join().map_err(|_panic| format!("{name} panicked"))?,
        None => Ok(IoCompletion::Complete),
    }
}

pub(super) fn progress_message(stream: Stream, bytes: &[u8]) -> String {
    let prefix = match stream {
        Stream::Stdout => "stdout: ",
        Stream::Stderr => "stderr: ",
    };
    let value = String::from_utf8_lossy(bytes);
    let mut message = format!("{prefix}{value}");
    let boundary = milkdrift_contracts::truncate_utf8(&message, 4_096).len();
    message.truncate(boundary);
    message
}

pub(super) fn redact_capture(capture: &mut Vec<u8>, secrets: &[SensitiveSecret]) {
    for secret in secrets {
        secret.expose(|value| replace_all(capture, value, b"[redacted]"));
    }
}

fn replace_all(target: &mut Vec<u8>, needle: &[u8], replacement: &[u8]) {
    if needle.is_empty() || target.len() < needle.len() {
        return;
    }
    let mut output = Vec::with_capacity(target.len());
    let mut offset = 0_usize;
    while offset < target.len() {
        if target[offset..].starts_with(needle) {
            output.extend_from_slice(replacement);
            offset = offset.saturating_add(needle.len());
        } else {
            output.push(target[offset]);
            offset = offset.saturating_add(1);
        }
    }
    *target = output;
}

#[cfg(unix)]
pub(super) fn secret_os_string(bytes: &[u8]) -> Result<OsString, String> {
    use std::os::unix::ffi::OsStringExt;
    if bytes.contains(&0) {
        return Err("resolved secret contains NUL".to_owned());
    }
    Ok(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
pub(super) fn secret_os_string(bytes: &[u8]) -> Result<OsString, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_error| "resolved secret is not valid UTF-8 on this platform".to_owned())?;
    if value.contains('\0') {
        return Err("resolved secret contains NUL".to_owned());
    }
    Ok(OsString::from(value))
}

#[cfg(unix)]
pub(super) fn os_bytes_len(value: &OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().len()
}

#[cfg(not(unix))]
pub(super) fn os_bytes_len(value: &OsStr) -> usize {
    value.to_string_lossy().len()
}
