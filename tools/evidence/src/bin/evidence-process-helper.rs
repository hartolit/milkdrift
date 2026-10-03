//! Deterministic child process used by the local-process operational evidence lane.

use std::io::Write as _;

fn main() -> std::io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--endpoint") {
        return stalled_cli();
    }
    let blocks = match std::env::args().nth(1).as_deref() {
        Some("emit") | None => 32,
        Some("--overflow-fixture") => 1_152,
        Some("--deadline-fixture") => {
            std::thread::sleep(std::time::Duration::from_secs(30));
            return Ok(());
        }
        Some("--selected-evidence") => {
            return std::fs::write("evidence.txt", b"selected architecture evidence\n");
        }
        Some("--omitted-evidence") => {
            return std::fs::write("evidence.txt", b"omitted unrelated evidence\n");
        }
        Some(_) => return Err(std::io::Error::other("unknown evidence helper mode")),
    };
    let stdout_block = [b'o'; 8_192];
    let stderr_block = [b'e'; 8_192];
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    for _ in 0..blocks {
        stdout.write_all(&stdout_block)?;
        stderr.write_all(&stderr_block)?;
    }
    stdout.flush()?;
    stderr.flush()
}

// A held socket proves this exact helper entered and that watchdog cleanup closed it.
// The parent releases successful cases explicitly; the fallback is finite even if it dies.
fn stalled_cli() -> std::io::Result<()> {
    use std::{io::Read as _, net::TcpStream, time::Duration};
    let address = std::env::args()
        .nth(2)
        .ok_or_else(|| std::io::Error::other("fixture endpoint absent"))?;
    let mut control = TcpStream::connect(address)?;
    control.set_read_timeout(Some(Duration::from_secs(30)))?;
    control.set_write_timeout(Some(Duration::from_secs(2)))?;
    control.write_all(&std::process::id().to_be_bytes())?;
    let mut release = [0];
    control.read_exact(&mut release)?;
    println!("{{\"status\":\"success\",\"value\":null}}");
    Ok(())
}
