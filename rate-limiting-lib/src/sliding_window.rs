use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use super::Reason;
use super::Strategy;

#[derive(Debug)]
pub struct SlidingWindow {
    capacity: usize,
    current: AtomicUsize,
    previous: AtomicUsize,
    interval: u64,
    expires: AtomicU64,
    anchor: Instant,
}

impl Strategy for SlidingWindow {
    fn process(&self) -> ControlFlow<Reason> {
        let now = Instant::now().duration_since(self.anchor).as_nanos() as u64;
        let mut expires = self.expires.load(Ordering::Acquire);

        // 1. Window Rotation
        if now >= expires {
            let elapsed = now.saturating_sub(expires);
            let intervals_passed = (elapsed / self.interval) + 1;
            let next_expires = expires + (intervals_passed * self.interval);

            if self
                .expires
                .compare_exchange(expires, next_expires, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                if intervals_passed == 1 {
                    let last = self.current.swap(0, Ordering::SeqCst);
                    self.previous.store(last, Ordering::SeqCst);
                } else {
                    self.current.store(0, Ordering::SeqCst);
                    self.previous.store(0, Ordering::SeqCst);
                }
                expires = next_expires;
            } else {
                expires = self.expires.load(Ordering::Acquire);
            }
        }

        // 2. Weight Calculation
        let window_start = expires.saturating_sub(self.interval);
        let time_into_window = now.saturating_sub(window_start);
        let progress = (time_into_window as f64 / self.interval as f64).min(1.0);
        let weight = 1.0 - progress;

        let p_count = self.previous.load(Ordering::Acquire) as f64;
        let c_count = self.current.load(Ordering::Acquire) as f64;

        // 3. Enforcement
        let guesstimate = (p_count * weight) + c_count;
        if guesstimate >= self.capacity as f64 {
            let wait = self.calculate_retry_after(guesstimate, now, expires);
            ControlFlow::Break(Reason::Overloaded { retry_after: wait })
        } else {
            self.current.fetch_add(1, Ordering::SeqCst);
            ControlFlow::Continue(())
        }
    }
}

impl SlidingWindow {
    pub fn new(capacity: NonZeroUsize, interval: Duration) -> Self {
        let interval = interval.as_nanos() as u64;
        Self {
            capacity: capacity.get(),
            current: Default::default(),
            previous: Default::default(),
            interval,
            expires: AtomicU64::new(interval),
            anchor: Instant::now(),
        }
    }
    fn calculate_retry_after(&self, guesstimate: f64, now: u64, expires: u64) -> Duration {
        let p_count = self.previous.load(Ordering::Acquire) as f64;

        if p_count <= 0.0 {
            // If there's no previous count, we are blocked by the current window.
            // We must wait for the current window to expire entirely.
            return Duration::from_nanos(expires.saturating_sub(now));
        }

        // How much 'weight' do we need to lose to get below capacity?
        let excess = guesstimate - (self.capacity as f64);

        // Weight drops at a rate of (1.0 / interval) per nanosecond.
        // Time to lose 'excess' = excess / (p_count / interval)
        let nanos_to_wait = (excess * (self.interval as f64) / p_count) as u64;

        // Buffer by 1ms to ensure we don't wake up in a "floating point tie"
        Duration::from_nanos(nanos_to_wait).saturating_add(Duration::from_millis(1))
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
        // Use a short interval so we rotate frequently
        let rl = SlidingWindow::new(NonZeroUsize::new(100).unwrap(), Duration::from_millis(10));

        let mut count = 0;
        for _i in 0..500 {
            let outcome = rl.process();
            if outcome.is_continue() {
                count += 1;
            } else {
                // If blocked, sleep for a full interval to ensure a rotation occurs
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
        let interval = Duration::from_millis(10);
        let rl = SlidingWindow::new(NonZeroUsize::new(10).unwrap(), interval);

        // Use tokens
        let _ = rl.process();

        // Sleep for 10x the interval
        std::thread::sleep(interval * 10);

        // The first call after a long idle should reset the window to 'now'
        // and provide full capacity.
        assert!(rl.process().is_continue());

        // Check if expires was bumped to the future correctly
        // (Note: This may fail with your current `self.expires += self.interval` logic)
        let now = Instant::now().duration_since(rl.anchor).as_nanos() as u64;
        let expires = rl.expires.load(Ordering::Acquire);
        assert!(now < expires, "Window should jump to current time");
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
