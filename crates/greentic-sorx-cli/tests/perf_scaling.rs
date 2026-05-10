use std::time::{Duration, Instant};

fn run_workload(threads: usize) -> Duration {
    let start = Instant::now();
    let total_iterations = 400_000usize;
    let per_thread = total_iterations / threads;

    let handles: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(move || {
                let mut sum = 0usize;
                for i in 0..per_thread {
                    sum = sum.wrapping_add(i);
                }
                sum
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("worker thread should not panic");
    }

    start.elapsed()
}

#[test]
fn scaling_should_not_degrade_badly() {
    let t1 = run_workload(1);
    let t4 = run_workload(4);
    let t8 = run_workload(8);

    assert!(
        t4 <= t1.mul_f64(4.0),
        "4 threads slower than expected: t1={t1:?}, t4={t4:?}"
    );

    assert!(
        t8 <= t4.mul_f64(4.0),
        "8 threads slower than expected: t4={t4:?}, t8={t8:?}"
    );
}
