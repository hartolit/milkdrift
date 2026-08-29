use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    sync::mpsc::SyncSender,
    thread::{self, JoinHandle},
};

use milkdrift_authority::SensitiveSecret;

const STREAM_READ_BYTES: usize = 8 * 1024;

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

pub(super) fn spawn_reader<R: Read + Send + 'static>(
    stream: Stream,
    mut reader: R,
    maximum: u64,
    sender: SyncSender<StreamMessage>,
) -> JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        let mut accepted = 0_u64;
        let mut overflow_sent = false;
        let mut buffer = [0_u8; STREAM_READ_BYTES];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) => {
                    let _ = sender.send(StreamMessage::Failed(stream, error.kind()));
                    return Err(format!("stream read failed: {:?}", error.kind()));
                }
            };
            let remaining = maximum.saturating_sub(accepted);
            let take = usize::try_from(remaining).unwrap_or(usize::MAX).min(count);
            if take != 0 {
                sender
                    .send(StreamMessage::Data(stream, buffer[..take].to_vec()))
                    .map_err(|_error| "stream receiver disconnected".to_owned())?;
                accepted = accepted.saturating_add(u64::try_from(take).unwrap_or(u64::MAX));
            }
            if take < count && !overflow_sent {
                sender
                    .send(StreamMessage::Overflow(stream))
                    .map_err(|_error| "stream receiver disconnected".to_owned())?;
                overflow_sent = true;
            }
        }
        sender
            .send(StreamMessage::Closed(stream))
            .map_err(|_error| "stream receiver disconnected".to_owned())?;
        Ok(())
    })
}

pub(super) fn spawn_stdin_writer(
    stdin: Option<std::process::ChildStdin>,
    bytes: Option<Vec<u8>>,
) -> Option<JoinHandle<Result<(), String>>> {
    stdin.zip(bytes).map(|(mut stdin, bytes)| {
        thread::spawn(move || {
            stdin
                .write_all(&bytes)
                .map_err(|error| format!("stdin write failed: {:?}", error.kind()))?;
            stdin
                .flush()
                .map_err(|error| format!("stdin flush failed: {:?}", error.kind()))
        })
    })
}

pub(super) fn join_io(
    thread: Option<JoinHandle<Result<(), String>>>,
    name: &str,
) -> Result<(), String> {
    match thread {
        Some(thread) => thread.join().map_err(|_panic| format!("{name} panicked"))?,
        None => Ok(()),
    }
}

pub(super) fn join_reader(
    thread: JoinHandle<Result<(), String>>,
    name: &str,
) -> Result<(), String> {
    thread.join().map_err(|_panic| format!("{name} panicked"))?
}

pub(super) fn progress_message(stream: Stream, bytes: &[u8]) -> String {
    let prefix = match stream {
        Stream::Stdout => "stdout: ",
        Stream::Stderr => "stderr: ",
    };
    let value = String::from_utf8_lossy(bytes);
    let mut message = format!("{prefix}{value}");
    if message.len() > 4_096 {
        let mut end = 4_096;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
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
