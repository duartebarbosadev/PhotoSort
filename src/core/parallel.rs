//! Independent blocks of CPU work spread over a few scoped threads, for the
//! grouping workers. The caller's thread works too and stays the only one
//! that asks whether the run was cancelled or reports progress.
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

/// Runs `work` for every block below `count` on up to `threads` threads and
/// returns the results in block order. `work` is given its block and a cheap
/// "stop now" check. `None` when cancelled or when a block gave up.
/// `progress` hears how many blocks are finished, on the calling thread.
pub(crate) fn blocks<T: Send>(
    threads: usize,
    count: usize,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize),
    work: impl Fn(usize, &dyn Fn() -> bool) -> Option<T> + Sync,
) -> Option<Vec<T>> {
    let stop = AtomicBool::new(false);
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let run = |between: &mut dyn FnMut()| {
        let mut results = Vec::new();
        loop {
            between();
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let block = next.fetch_add(1, Ordering::Relaxed);
            if block >= count {
                break;
            }
            match work(block, &|| stop.load(Ordering::Relaxed)) {
                Some(result) => {
                    results.push((block, result));
                    finished.fetch_add(1, Ordering::Relaxed);
                }
                None => {
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
        results
    };
    let mut watch = || {
        if cancelled() {
            stop.store(true, Ordering::Relaxed);
        }
        progress(finished.load(Ordering::Relaxed));
    };
    let mut results = std::thread::scope(|scope| {
        let helpers: Vec<_> = (1..threads.min(count))
            .map(|_| scope.spawn(|| run(&mut || {})))
            .collect();
        let mut results = run(&mut watch);
        for helper in helpers {
            while !helper.is_finished() {
                watch();
                std::thread::sleep(Duration::from_millis(2));
            }
            match helper.join() {
                Ok(more) => results.extend(more),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        results
    });
    watch();
    if stop.load(Ordering::Relaxed) || results.len() != count {
        return None;
    }
    results.sort_unstable_by_key(|(block, _)| *block);
    Some(results.into_iter().map(|(_, result)| result).collect())
}

/// A dot product whose sum may be vectorized. It is not bit-equal to a
/// left-to-right sum: callers pair it with `dot_error` and settle anything
/// that close with their exact arithmetic.
#[inline]
pub(crate) fn fast_dot(a: &[f32], b: &[f32]) -> f32 {
    const LANES: usize = 16;
    let ((head_a, tail_a), (head_b, tail_b)) = (a.as_chunks::<LANES>(), b.as_chunks::<LANES>());
    // Plain indexed loops over fixed-size arrays: the form the compiler
    // turns into vector instructions without relying on iterator inlining.
    let mut lanes = [0.0f32; LANES];
    for chunk in 0..head_a.len().min(head_b.len()) {
        let (a, b) = (&head_a[chunk], &head_b[chunk]);
        for lane in 0..LANES {
            lanes[lane] += a[lane] * b[lane];
        }
    }
    let mut sum = 0.0;
    for lane in lanes {
        sum += lane;
    }
    for index in 0..tail_a.len().min(tail_b.len()) {
        sum += tail_a[index] * tail_b[index];
    }
    sum
}

/// More than the distance between two f32 dot products of the same
/// `width`-long vectors of at most the given lengths, however each was
/// summed. Each sum is within `width * 2^-24 * |a| * |b|` of the true value
/// (Higham, Accuracy and Stability of Numerical Algorithms, 3.5); this is
/// twice both together, plus room for underflow.
pub(crate) fn dot_error(width: usize, length_a: f64, length_b: f64) -> f64 {
    2.0 * width as f64 * f64::from(f32::EPSILON) * length_a * length_b + 1e-30
}
