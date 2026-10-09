//! Running replications in parallel.

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

/// Calls `f(r)` for replications `r = 1..=count` on up to `threads` threads
/// and returns the results in replication order.
///
/// Each replication has its own random streams, so results don't depend on
/// the number of threads or on which thread ran what.
pub fn parallel_replications<T: Send>(
    count: u32,
    threads: usize,
    f: impl Fn(u32) -> T + Sync,
) -> Vec<T> {
    let next = AtomicU32::new(1);
    let threads = threads.clamp(1, count.max(1) as usize);
    let (f, next) = (&f, &next);
    let mut results: Vec<(u32, T)> = thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(move || {
                    let mut done = Vec::new();
                    loop {
                        let r = next.fetch_add(1, Ordering::Relaxed);
                        if r > count {
                            break done;
                        }
                        done.push((r, f(r)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("a replication panicked"))
            .collect()
    });
    results.sort_by_key(|(r, _)| *r);
    results.into_iter().map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_order() {
        for threads in [1, 3, 16] {
            let out = parallel_replications(10, threads, |r| r * r);
            assert_eq!(out, (1..=10).map(|r| r * r).collect::<Vec<_>>());
        }
        assert!(parallel_replications(0, 4, |r| r).is_empty());
    }
}
