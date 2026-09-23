use super::*;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

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
