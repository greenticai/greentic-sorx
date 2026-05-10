use std::time::{Duration, Instant};

#[test]
fn workload_should_finish_quickly() {
    let start = Instant::now();

    let mut sum = 0usize;
    for i in 0..5_000_000 {
        sum = sum.wrapping_add(i);
    }

    let elapsed = start.elapsed();

    assert!(sum > 0);
    assert!(
        elapsed < Duration::from_secs(2),
        "workload too slow: {elapsed:?}"
    );
}
