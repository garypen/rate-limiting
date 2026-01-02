use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use super::Reason;
use super::Strategy;

/// A simple window-based limiter.
///
/// Divides time into fixed intervals. It is the most performant strategy
/// but can be susceptible to "boundary bursts" where double the limit is
/// allowed in a short period spanning two windows.
#[derive(Debug)]
pub struct FixedWindow {
    capacity: usize,
    remaining: AtomicUsize,
    expires: AtomicU64,
    interval: u64,
    anchor: Instant,
}

impl Strategy for FixedWindow {
    fn process(&self) -> ControlFlow<Reason> {
        let now = Instant::now().duration_since(self.anchor).as_nanos() as u64;
        let expires = self.expires.load(Ordering::Acquire);

        if now > expires {
            let next_expires = now + self.interval;
            if self
                .expires
                .compare_exchange(expires, next_expires, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                self.remaining.store(self.capacity, Ordering::Release);
            }
        }

        let old_remaining =
            self.remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
                    if val > 0 { Some(val - 1) } else { None }
                });

        match old_remaining {
            Ok(_) => ControlFlow::Continue(()),
            Err(_) => ControlFlow::Break(Reason::Overloaded {
                retry_after: Duration::from_nanos(expires - now),
            }),
        }
    }
}

impl FixedWindow {
    /// Creates a new `FixedWindow` strategy.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The maximum number of requests allowed within a single window.
    /// * `interval` - The duration of the fixed time window.
    pub fn new(capacity: NonZeroUsize, interval: Duration) -> Self {
        Self {
            capacity: capacity.get(),
            remaining: capacity.get().into(),
            interval: interval.as_nanos() as u64,
            expires: AtomicU64::new(interval.as_nanos() as u64),
            anchor: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_enforces_limits() {
        let rl = FixedWindow::new(NonZeroUsize::new(1).unwrap(), Duration::from_millis(10));

        assert_eq!(rl.process(), ControlFlow::Continue(()));
        assert!(matches!(rl.process(), ControlFlow::Break(..)));
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(rl.process(), ControlFlow::Continue(()));
    }

    #[test]
    fn test_idle_reset_drift() {
        let interval = Duration::from_millis(10);
        let rl = FixedWindow::new(NonZeroUsize::new(1).unwrap(), interval);

        // Use the first token
        let _ = rl.process();

        // Sleep for 5 intervals
        std::thread::sleep(interval * 5);

        // Ideally, this should reset to a fresh window immediately
        assert_eq!(rl.process(), ControlFlow::Continue(()));

        // Check if the NEW expiry is actually in the future, not still in the past
        let now = Instant::now().duration_since(rl.anchor).as_nanos() as u64;
        let expires = rl.expires.load(Ordering::Acquire);
        assert!(now < expires, "Expiry should have jumped to the future");
    }

    #[tokio::test]
    async fn test_actual_concurrency() {
        use std::sync::Arc;

        let capacity = 100;
        // Wrap in Arc to share across tasks
        let rl = Arc::new(FixedWindow::new(
            NonZeroUsize::new(capacity).unwrap(),
            Duration::from_secs(1),
        ));

        let mut handles = vec![];

        for _ in 0..capacity + 10 {
            let rl_clone = Arc::clone(&rl);
            handles.push(tokio::spawn(async move { rl_clone.process() }));
        }

        let results = futures::future::join_all(handles).await;
        let success_count = results
            .into_iter()
            .filter(|r| matches!(r, Ok(ControlFlow::Continue(()))))
            .count();

        // Even with multiple tasks, exactly 'capacity' should pass
        assert_eq!(success_count, capacity);
    }

    #[test]
    fn test_exact_window_boundary() {
        let interval = Duration::from_millis(50);
        let rl = FixedWindow::new(NonZeroUsize::new(1).unwrap(), interval);

        let _ = rl.process(); // Consume the only token

        // Wait until just before expiry
        std::thread::sleep(Duration::from_millis(40));
        assert!(matches!(rl.process(), ControlFlow::Break(..)));

        // Wait for the remaining 10ms + a tiny buffer
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(rl.process(), ControlFlow::Continue(()));
    }
}
