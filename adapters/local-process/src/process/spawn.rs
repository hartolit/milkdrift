use std::{
    ffi::OsString,
    path::Path,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
};

#[cfg(unix)]
type OwnedPipe = std::os::fd::OwnedFd;
#[cfg(windows)]
type OwnedPipe = std::os::windows::io::OwnedHandle;

// Configure only the daemon endpoint. The child's endpoint must retain ordinary
// blocking stdio semantics. All fallible pipe setup precedes external entry.
fn nonblocking(pipe: impl Into<OwnedPipe>) -> std::io::Result<OwnedPipe> {
    let pipe = pipe.into();
    #[cfg(unix)]
    {
        use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
        fcntl_setfl(&pipe, fcntl_getfl(&pipe)? | OFlags::NONBLOCK)?;
        Ok(pipe)
    }
    #[cfg(windows)]
    {
        use interprocess::os::windows::named_pipe::{DuplexPipeStream, pipe_mode::Bytes};
        use std::os::windows::io::AsRawHandle;

        let original = pipe.as_raw_handle();
        let stream = DuplexPipeStream::<Bytes>::try_from(pipe).map_err(std::io::Error::other)?;
        // Interprocess may reopen a handle for overlapped I/O. Our workers use
        // synchronous nonblocking operations; refuse a changed handle before spawn.
        let result = if stream.as_raw_handle() == original {
            stream.set_nonblocking(true)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "nonblocking process pipes require synchronous handles",
            ))
        };
        // Recover ownership even on setup failure. Never invoke the library's
        // implicit flush/limbo pool: our endpoints close their handles directly.
        let pipe = OwnedPipe::try_from(stream).map_err(|stream| {
            stream.evade_limbo();
            std::io::Error::other("process pipe unexpectedly has shared ownership")
        })?;
        result?;
        Ok(pipe)
    }
}

pub(super) fn spawn(
    executable: &Path,
    working_directory: &Path,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
    piped_stdin: bool,
) -> std::io::Result<Child> {
    let (stdout, stdout_child) = std::io::pipe()?;
    let stdout = ChildStdout::from(nonblocking(stdout)?);
    let (stderr, stderr_child) = std::io::pipe()?;
    let stderr = ChildStderr::from(nonblocking(stderr)?);
    let (stdin, stdin_child) = if piped_stdin {
        let (child, parent) = std::io::pipe()?;
        (
            Some(ChildStdin::from(nonblocking(parent)?)),
            Stdio::from(child),
        )
    } else {
        (None, Stdio::null())
    };
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(working_directory)
        .env_clear()
        .stdin(stdin_child)
        .stdout(stdout_child)
        .stderr(stderr_child);
    for (name, value) in environment {
        command.env(name, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    // Command retains its endpoints until dropped; retaining them would hide EOF.
    drop(command);
    child.stdin = stdin;
    child.stdout = Some(stdout);
    child.stderr = Some(stderr);
    Ok(child)
}
