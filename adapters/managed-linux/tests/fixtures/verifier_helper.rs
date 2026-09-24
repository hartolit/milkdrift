//! Native deliberately non-cleaning verifier for the platform's owned-container cleanup tests.
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args_os().nth(1).ok_or("input path absent")?;
    let value: Value = serde_json::from_slice(&fs::read(input)?)?;
    let field = |name| {
        value
            .get(name)
            .and_then(Value::as_str)
            .ok_or("verifier input field absent")
    };
    let output = Command::new("/usr/bin/podman")
        .args([
            "run",
            "--detach",
            "--pull=never",
            "--network=none",
            &format!("--name={}", field("container")?),
            &format!(
                "--label=org.milkdrift.platform={}",
                field("platform_owner")?
            ),
            &format!(
                "--label=org.milkdrift.verification={}",
                field("evaluation")?
            ),
            "--entrypoint=/bin/sleep",
            field("image")?,
            "120",
        ])
        .output()?;
    if !output.status.success() {
        return Err("fixture container launch failed".into());
    }
    fs::write(
        Path::new(field("directory")?).join("launched"),
        output.stdout,
    )?;
    if value["application"]["timeout"] == true {
        std::thread::sleep(Duration::from_secs(60));
    }
    println!(
        "{}",
        json!([{"name":"lifecycle","passed":true,"diagnostic":"test container launched"}])
    );
    Ok(())
}
