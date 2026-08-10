//! Integration coverage for the visible xtask command surface.

use std::error::Error;
use std::process::Command;

#[test]
fn help_uses_the_milkdrift_identity() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("--help")
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Milkdrift workspace tooling"));
    assert!(!stdout.contains("llm-app"));
    Ok(())
}
