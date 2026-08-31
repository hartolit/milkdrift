//! Deterministic child process used by the local-process operational evidence lane.

use std::io::Write as _;

fn main() -> std::io::Result<()> {
    let stdout_block = [b'o'; 8_192];
    let stderr_block = [b'e'; 8_192];
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    for _ in 0..32 {
        stdout.write_all(&stdout_block)?;
        stderr.write_all(&stderr_block)?;
    }
    stdout.flush()?;
    stderr.flush()
}
