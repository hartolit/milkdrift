//! Cross-platform local-process integration-test fixture.

use std::{
    env,
    fs::OpenOptions,
    io::{Read, Write},
    process::{Command, ExitCode},
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "fixture error: {error}");
            ExitCode::from(111)
        }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().ok_or("missing fixture command")?;
    match command.as_str() {
        "inspect" => {
            let output = arguments.next().ok_or("missing output path")?;
            let environment_name = arguments.next().ok_or("missing environment name")?;
            let literal = arguments.next().ok_or("missing literal argument")?;
            let mut stdin = Vec::new();
            std::io::stdin().read_to_end(&mut stdin)?;
            let document = serde_json::json!({
                "literal": literal,
                "stdin": String::from_utf8_lossy(&stdin),
                "selected_environment": env::var(&environment_name).ok(),
                "ambient_home": env::var("HOME").ok(),
                "ambient_path": env::var("PATH").ok()
            });
            std::fs::write(output, serde_json::to_vec(&document)?)?;
            writeln!(std::io::stdout(), "fixture stdout")?;
            writeln!(std::io::stderr(), "fixture stderr")?;
            Ok(0)
        }
        "emit" => {
            let bytes: usize = arguments.next().ok_or("missing byte count")?.parse()?;
            let stdout_thread =
                thread::spawn(move || write_repeated(std::io::stdout(), b'o', bytes));
            let stderr_thread =
                thread::spawn(move || write_repeated(std::io::stderr(), b'e', bytes));
            stdout_thread
                .join()
                .map_err(|_| "stdout writer panicked")??;
            stderr_thread
                .join()
                .map_err(|_| "stderr writer panicked")??;
            Ok(0)
        }
        "echo-env" => {
            let name = arguments.next().ok_or("missing environment name")?;
            writeln!(std::io::stdout(), "{}", env::var(name)?)?;
            Ok(0)
        }
        "sleep" => {
            let millis: u64 = arguments.next().ok_or("missing sleep duration")?.parse()?;
            thread::sleep(Duration::from_millis(millis));
            Ok(0)
        }
        "exit" => {
            let code: u8 = arguments.next().ok_or("missing exit code")?.parse()?;
            Ok(code)
        }
        "signal" => {
            #[cfg(unix)]
            {
                rustix::process::kill_process(
                    rustix::process::getpid(),
                    rustix::process::Signal::TERM,
                )?;
                thread::sleep(Duration::from_secs(1));
                Ok(112)
            }
            #[cfg(not(unix))]
            {
                Ok(112)
            }
        }
        "tree" => {
            let pid_file = arguments.next().ok_or("missing pid file")?;
            let millis = arguments.next().ok_or("missing tree duration")?;
            append_pid(&pid_file)?;
            let executable = env::current_exe()?;
            let mut child = Command::new(executable)
                .arg("tree-child")
                .arg(&pid_file)
                .arg(&millis)
                .spawn()?;
            thread::sleep(Duration::from_millis(millis.parse()?));
            let _ = child.wait();
            Ok(0)
        }
        "tree-child" => {
            let pid_file = arguments.next().ok_or("missing pid file")?;
            let millis = arguments.next().ok_or("missing tree duration")?;
            append_pid(&pid_file)?;
            let executable = env::current_exe()?;
            let mut child = Command::new(executable)
                .arg("tree-grandchild")
                .arg(&pid_file)
                .arg(&millis)
                .spawn()?;
            thread::sleep(Duration::from_millis(millis.parse()?));
            let _ = child.wait();
            Ok(0)
        }
        "tree-grandchild" => {
            let pid_file = arguments.next().ok_or("missing pid file")?;
            let millis: u64 = arguments.next().ok_or("missing tree duration")?.parse()?;
            append_pid(&pid_file)?;
            thread::sleep(Duration::from_millis(millis));
            Ok(0)
        }
        _ => Err("unknown fixture command".into()),
    }
}

fn write_repeated(mut writer: impl Write, byte: u8, bytes: usize) -> Result<(), std::io::Error> {
    let chunk = vec![byte; 8 * 1024];
    let mut remaining = bytes;
    while remaining != 0 {
        let take = remaining.min(chunk.len());
        writer.write_all(&chunk[..take])?;
        remaining -= take;
    }
    writer.flush()
}

fn append_pid(path: &str) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", std::process::id())?;
    file.flush()
}
