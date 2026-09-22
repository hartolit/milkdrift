//! Verify readiness and one bounded real inference before publishing an owned service generation.
use crate::{LinuxManagedPlatform, ModelService, command, platform_error, units};
use milkdrift_capability_host::managed::{ManagedError, ManagedPlatform};
use milkdrift_persistence::managed::{ApprovedSetup, ManagedObservation};
use std::{
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

pub(crate) fn verify(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<ManagedObservation, ManagedError> {
    let d = units::deployment(setup)?;
    let ModelService::Owned { port, backend, .. } = d.recipe.model_service else {
        return platform.observe(setup);
    };
    let observation = ready(platform, setup)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(platform_error)?;
    let response = client.post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&serde_json::json!({"model":"ornith","messages":[{"role":"user","content":"Reply OK."}],"max_tokens":1,"stream":false,"temperature":0}))
        .send().map_err(|_| platform_error("owned inference probe did not return a complete response"))?;
    if !response.status().is_success() {
        return Err(platform_error("owned inference probe was refused"));
    }
    let mut bytes = Vec::new();
    response
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(platform_error)?;
    if bytes.len() > 65_536 {
        return Err(platform_error(
            "owned inference probe exceeded its response bound",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| platform_error("owned inference response was not JSON"))?;
    if value
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .is_none()
        || value
            .pointer("/choices/0/message")
            .and_then(|v| v.as_object())
            .is_none()
    {
        return Err(platform_error(
            "owned server did not complete the inference probe",
        ));
    }
    if matches!(backend, crate::InferenceBackend::Vulkan { .. }) {
        let output = command::run(
            Path::new("/usr/bin/podman"),
            &["logs".to_owned(), "--tail=100".to_owned(), d.unit],
            &[],
            Duration::from_secs(15),
            1_048_576,
            None,
        )?;
        let logs = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .to_lowercase();
        // llama.cpp sends backend diagnostics to stderr. Require positive nonzero offload evidence,
        // in addition to the exact passed device and completed generation. This is not a throughput claim.
        let positive_offload = logs.lines().any(|line| {
            line.split_once("offloaded ").is_some_and(|(_, tail)| {
                tail.split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .is_some_and(|n| n > 0)
            })
        });
        if !output.success || !logs.contains("vulkan") || !positive_offload {
            return Err(platform_error(
                "completed inference lacks observed Vulkan and nonzero layer-offload evidence",
            ));
        }
    }
    Ok(ManagedObservation { summary: "owned server completed bounded inference; configured backend verified; no performance or reboot qualification".to_owned(), ..observation })
}

pub(crate) fn ready(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<ManagedObservation, ManagedError> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let observation = loop {
        if let Ok(observation) = platform.observe(setup)
            && observation.running
        {
            break observation;
        }
        if Instant::now() >= deadline {
            return Err(platform_error(
                "owned model did not become ready within 120 seconds",
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    Ok(observation)
}
