use super::*;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn cleanup_requires_exit_evidence_and_exposes_wait_failure() {
    let mut observations = 0;
    let result = wait_for_reap(Duration::ZERO, || {
        observations += 1;
        Ok(None)
    });
    assert_eq!(result, Err("helper reap deadline exceeded".to_owned()));
    assert_eq!(
        observations, 1,
        "an expired budget must not start another wait"
    );
    let error = wait_for_reap(Duration::from_secs(1), || {
        Err(std::io::ErrorKind::PermissionDenied.into())
    });
    assert!(error.is_err_and(|error| error.contains("PermissionDenied")));
}

#[test]
fn failed_helper_is_killed_and_reaped_before_returning_the_original_error() -> Result {
    use std::os::unix::process::CommandExt as _;
    let child = Command::new("/bin/sleep")
        .arg("10")
        .process_group(0)
        .spawn()?;
    let mut owned = OwnedChild {
        child,
        reaped: false,
    };
    let error = owned.finish(Err(platform_error("injected observation failure")));
    assert!(error.is_err_and(|error| error.to_string().contains("injected observation failure")));
    assert!(owned.reaped);
    assert!(owned.child.try_wait()?.is_some());
    Ok(())
}

#[test]
fn helper_output_is_combined_and_overflow_stops_capture() -> Result {
    let command = |text: &str, limit| {
        run(
            Path::new("/bin/sh"),
            &["-c".to_owned(), text.to_owned()],
            &[],
            Duration::from_secs(2),
            limit,
            None,
        )
    };
    let result = command("printf 1234; printf 56789 >&2", 9)?;
    assert_eq!(result.stdout, b"1234");
    assert_eq!(result.stderr, b"56789");
    assert!(
        command("printf 1234; printf 56789 >&2", 8)
            .err()
            .ok_or("combined overflow accepted")?
            .to_string()
            .contains("output limit")
    );
    assert!(command("while :; do printf 0123456789; done", 31).is_err());
    Ok(())
}

#[test]
fn varied_deadlines_and_cancellation_reap_the_helper_group() -> Result {
    for milliseconds in [40, 110] {
        let start = Instant::now();
        let result = run(
            Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 10".to_owned()],
            &[],
            Duration::from_millis(milliseconds),
            128,
            None,
        );
        assert!(
            result
                .err()
                .ok_or("deadline ignored")?
                .to_string()
                .contains("deadline")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        signal.store(true, Ordering::Release);
    });
    let result = run(
        Path::new("/bin/sh"),
        &["-c".to_owned(), "sleep 10".to_owned()],
        &[],
        Duration::from_secs(5),
        128,
        Some(&cancel),
    );
    thread.join().map_err(|_| "cancel thread panicked")?;
    assert!(
        result
            .err()
            .ok_or("cancellation ignored")?
            .to_string()
            .contains("cancelled")
    );
    Ok(())
}
