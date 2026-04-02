use greentic_operator::operator_i18n;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn i18n_workload_finishes_quickly() {
    let (tx, rx) = mpsc::channel();
    let start = Instant::now();

    std::thread::spawn(move || {
        for _ in 0..20_000 {
            let translated = operator_i18n::tr_for_locale(
                "cli.main.help.tagline",
                "Greentic operator tooling",
                "en-US",
            );
            assert!(!translated.is_empty());
        }
        let _ = tx.send(());
    });

    rx.recv_timeout(Duration::from_secs(2))
        .expect("i18n workload timed out");
    println!("perf_timeout: elapsed={:?}", start.elapsed());
}
