use std::{
    ffi::OsString,
    path::Path,
    process::{Child, Command, Stdio},
};

pub(super) fn spawn(
    executable: &Path,
    working_directory: &Path,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
    piped_stdin: bool,
) -> std::io::Result<Child> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(working_directory)
        .env_clear()
        .stdin(if piped_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}
