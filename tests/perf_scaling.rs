use greentic_operator::operator_i18n;
use std::time::{Duration, Instant};

const ITERATIONS_PER_THREAD: usize = 2_500;

fn run_i18n_workload(threads: usize) -> Duration {
    let start = Instant::now();

    let handles: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..ITERATIONS_PER_THREAD {
                    let translated = operator_i18n::tr_for_locale(
                        "cli.main.help.tagline",
                        "Greentic operator tooling",
                        "en-US",
                    );
                    assert!(!translated.is_empty());
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("worker thread should finish");
    }

    start.elapsed()
}

#[test]
fn i18n_lookup_scales_without_major_regression() {
    let t1 = run_i18n_workload(1);
    let t4 = run_i18n_workload(4);
    println!("perf_scaling: threads=1 elapsed={t1:?}");
    println!("perf_scaling: threads=4 elapsed={t4:?}");

    assert!(
        t4 <= t1.mul_f64(6.0),
        "4 threads slowed down too much: t1={t1:?}, t4={t4:?}"
    );
}
