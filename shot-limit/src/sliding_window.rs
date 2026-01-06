use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use quanta::Clock;
use quanta::Instant;

use super::Reason;
use super::Strategy;

/// A Sliding Window Counter implementation.
///
/// It maintains a count for the current fixed window and the previous one.
/// The effective count is: (previous_count * %_of_window_left) + current_count.
#[derive(Debug)]
pub struct SlidingWindow {
    capacity: usize,
    period_ns: u64,
    /// Current window's request count
    current_count: AtomicUsize,
    /// Previous window's request count
    previous_count: AtomicUsize,
    /// Timestamp (nanos from anchor) for the start of the current window
    current_window_start: AtomicU64,
    clock: Clock,
    anchor: Instant,
}

impl SlidingWindow {
    pub fn new(capacity: NonZeroUsize, period: Duration) -> Self {
        let clock = Clock::new();
        let anchor = clock.now();
        Self {
            capacity: capacity.get(),
            period_ns: period.as_nanos() as u64,
            current_count: AtomicUsize::new(0),
            previous_count: AtomicUsize::new(0),
            current_window_start: AtomicU64::new(0),
            clock,
            anchor,
        }
    }
}

impl Strategy for SlidingWindow {
    #[inline]
    fn process(&self) -> ControlFlow<Reason> {
        let now = self.clock.now().duration_since(self.anchor).as_nanos() as u64;
        let mut window_start = self.current_window_start.load(Ordering::Acquire);

        // 1. Check if we need to slide the window
        if now >= window_start + self.period_ns {
            let new_window_start = (now / self.period_ns) * self.period_ns;

            if self
                .current_window_start
                .compare_exchange(
                    window_start,
                    new_window_start,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // If we moved at least two windows forward, previous_count is 0
                let prev_val = if now >= window_start + (2 * self.period_ns) {
                    0
                } else {
                    self.current_count.load(Ordering::Acquire)
                };

                self.previous_count.store(prev_val, Ordering::Release);
                self.current_count.store(0, Ordering::Release);
                window_start = new_window_start;
            } else {
                // May have been set by a different thread
                window_start = self.current_window_start.load(Ordering::Acquire);
            }
        }

        // 2. Calculate the weighted count
        let prev_count = self.previous_count.load(Ordering::Acquire) as f64;
        let curr_count = self.current_count.load(Ordering::Acquire);

        let elapsed_in_window = now - window_start;
        let weight = (self.period_ns - elapsed_in_window) as f64 / self.period_ns as f64;
        let estimated_count = (prev_count * weight).floor() as usize + curr_count;

        if estimated_count < self.capacity {
            self.current_count.fetch_add(1, Ordering::SeqCst);
            ControlFlow::Continue(())
        } else {
            // Estimate wait time based on when the weighted count would drop below capacity
            let missing = estimated_count - self.capacity;
            let retry_after_ns = if missing > 0 {
                ((missing as f64 / self.capacity as f64) * self.period_ns as f64).ceil() as u64
            } else {
                0
            };
            ControlFlow::Break(Reason::Overloaded {
                retry_after: Duration::from_nanos(retry_after_ns),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    //
    // Ensure that blasting requests in means we enforce our limit
    //
    #[test]
    fn it_loosely_enforces_limits_without_sleep() {
        let rl = SlidingWindow::new(NonZeroUsize::new(100).unwrap(), Duration::from_millis(10));

        let mut count = 0;
        for _i in 0..500 {
            let outcome = rl.process();
            if matches!(outcome, ControlFlow::Continue(_)) {
                count += 1;
            }
        }
        assert_eq!(count, 100);
    }

    #[test]
    fn it_loosely_enforces_sleepy_limits_with_sleep() {
        // Use a short period so we rotate frequently
        let rl = SlidingWindow::new(NonZeroUsize::new(100).unwrap(), Duration::from_millis(10));

        let mut count = 0;
        for _i in 0..500 {
            let outcome = rl.process();
            if outcome.is_continue() {
                count += 1;
            } else {
                // If blocked, sleep for a full period to ensure a rotation occurs
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        // We expect to have processed significantly more than the 100 capacity
        // because the "previous" window keeps sliding away.
        assert!(
            count > 200,
            "Should have allowed much more than capacity, got {}",
            count
        );
    }

    #[test]
    fn test_sliding_window_concurrency() {
        use std::sync::Arc;
        use std::thread;

        let capacity = 100;
        let rl = Arc::new(SlidingWindow::new(
            NonZeroUsize::new(capacity).unwrap(),
            Duration::from_millis(500),
        ));

        let mut handles = vec![];
        for _ in 0..capacity {
            let rl_clone = Arc::clone(&rl);
            handles.push(thread::spawn(move || rl_clone.process()));
        }

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let success_count = results.iter().filter(|r| r.is_continue()).count();

        assert_eq!(
            success_count, capacity,
            "Atomic sliding window should allow exactly capacity during burst"
        );
    }

    #[test]
    fn test_sliding_window_long_idle() {
        let period = Duration::from_millis(10);
        let rl = SlidingWindow::new(NonZeroUsize::new(10).unwrap(), period);

        // Use a token
        let _ = rl.process();

        // Sleep for 10x the period (100ms)
        std::thread::sleep(period * 10);

        // This call triggers the "slide" logic
        assert!(rl.process().is_continue());

        // Check internal state instead of a single 'expires' field
        let now = rl.clock.now().duration_since(rl.anchor).as_nanos() as u64;
        let window_start = rl.current_window_start.load(Ordering::Acquire);
        let prev_count = rl.previous_count.load(Ordering::Acquire);

        // 1. The window start should be the most recent boundary (now - (now % period))
        assert!(
            window_start <= now,
            "Window start should not be in the future"
        );
        assert!(
            now < window_start + rl.period_ns,
            "Now must fall within the current window"
        );

        // 2. Since we slept for 10 periods, the previous window's count must be 0
        assert_eq!(
            prev_count, 0,
            "Previous count should be cleared after long idle"
        );
    }

    #[test]
    fn it_enforces_sliding_limits() {
        let rl = SlidingWindow::new(NonZeroUsize::new(100).unwrap(), Duration::from_millis(100));

        // 1. Fill the FIRST window
        for _ in 0..100 {
            let _ = rl.process();
        }

        // 2. Wait 100ms to ensure the first window is definitely over and rotates
        std::thread::sleep(Duration::from_millis(100));

        // Trigger one call to force the rotation logic to run
        // Now: previous = 100, current = 0, expires = 200ms
        let _ = rl.process();

        // 3. Now sleep 60ms into the SECOND window
        std::thread::sleep(Duration::from_millis(60));

        let mut extra = 0;
        while rl.process().is_continue() {
            extra += 1;
        }

        // Since we slept for 60ms, and we recover 60 tokens per 100ms, we should have at least 60
        // tokens.
        assert!(
            extra >= 60,
            "Should have recovered capacity from the PREVIOUS window"
        );
    }

    #[test]
    fn test_sliding_window_partial_recovery() {
        let rl = SlidingWindow::new(NonZeroUsize::new(2).unwrap(), Duration::from_millis(100));

        // Fill Window A
        let _ = rl.process(); // c=1
        let _ = rl.process(); // c=2

        // Force Window A to end
        std::thread::sleep(Duration::from_millis(110));

        // This call triggers rotation.
        // It sees p=2, c=0, weight=~0.9. (2 * 0.9) + 0 = 1.8.
        // 1.8 < 2.0, so it ALLOWS.
        // Now p=2, c=1.
        assert!(
            rl.process().is_continue(),
            "First call in new window should pass if weight dropped slightly"
        );

        // Next call: (2 * ~0.9) + 1 = 2.8.
        // 2.8 >= 2.0.
        assert!(rl.process().is_break(), "Second call should block");
    }

    #[test]
    fn test_sliding_window_prevents_double_burst() {
        let rl = SlidingWindow::new(NonZeroUsize::new(100).unwrap(), Duration::from_millis(100));

        // Fill Window A
        for _ in 0..100 {
            let _ = rl.process();
        }

        // Move slightly into Window B
        std::thread::sleep(Duration::from_millis(20));

        // Even though it's a "new" window, weight is ~0.8.
        // Guesstimate = (100 * 0.8) + 0 = 80.
        // We should only be able to fit ~20 more, not a full 100.
        let mut extra = 0;
        while rl.process().is_continue() {
            extra += 1;
        }

        assert!(
            extra < 50,
            "Should not allow a full second burst immediately"
        );
    }
}
