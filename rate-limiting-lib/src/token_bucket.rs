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
pub struct TokenBucket {
    capacity: usize,
    remaining: AtomicUsize,
    interval: Duration,
    last: AtomicU64,
    increment: usize,
    anchor: Instant,
}

impl Strategy for TokenBucket {
    fn process(&self) -> ControlFlow<Reason> {
        self.refill();
        if self.remaining.load(Ordering::Relaxed) == 0 {
            let wait = self.interval.as_nanos() as u64 / self.increment as u64;
            ControlFlow::Break(Reason::Overloaded {
                retry_after: Duration::from_nanos(wait),
            })
        } else {
            self.remaining.fetch_sub(1, Ordering::Relaxed);
            ControlFlow::Continue(())
        }
    }
}

impl TokenBucket {
    pub fn new(capacity: NonZeroUsize, increment: usize, interval: Duration) -> Self {
        Self {
            capacity: capacity.get(),
            remaining: AtomicUsize::new(capacity.get()),
            interval,
            last: AtomicU64::new(0),
            increment,
            anchor: Instant::now(),
        }
    }

    fn refill(&self) {
        let now = Instant::now().duration_since(self.anchor).as_nanos() as u64;
        // Use fetch_update to prevent race conditions. Ignore the result.
        let _ = self
            .last
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                let elapsed = now.saturating_sub(last);
                let intervals_passed = elapsed / self.interval.as_nanos() as u64;

                if intervals_passed > 0 {
                    let added = (intervals_passed as usize) * self.increment;
                    let current = self.remaining.load(Ordering::Acquire);
                    self.remaining
                        .store((current + added).min(self.capacity), Ordering::Release);

                    // Advance the clock by the exact intervals consumed
                    Some(last + (intervals_passed * self.interval.as_nanos() as u64))
                } else {
                    None // No update needed
                }
            });

        /*
        let elapsed = now - self.last;

        // Calculate how many intervals have fully passed
        let intervals_passed = elapsed.as_nanos() / self.interval.as_nanos();
        let added = (intervals_passed as usize) * self.increment;

        if added > 0 {
            let new = usize::min(
                self.capacity,
                self.remaining.load(Ordering::Relaxed) + added,
            );
            self.remaining.store(new, Ordering::Relaxed);

            // Only move 'last' forward by the duration of the consumed intervals
            // This keeps the "remainder" time for the next call!
            self.last += self.interval * (intervals_passed as u32);
        }
        */
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_enforces_limits_starting_full() {
        let capacity = 2; // Small capacity for easy testing
        let rl = TokenBucket::new(
            NonZeroUsize::new(capacity).unwrap(),
            1,
            Duration::from_millis(100),
        );

        // 1. Should be able to burst up to capacity immediately
        assert_eq!(rl.process(), ControlFlow::Continue(()));
        assert_eq!(rl.process(), ControlFlow::Continue(()));

        // 2. Third request should fail (exhausted)
        assert!(matches!(rl.process(), ControlFlow::Break(..)));

        // 3. Wait for one refill interval
        std::thread::sleep(Duration::from_millis(110));

        // 4. Should have 1 new token
        assert_eq!(rl.process(), ControlFlow::Continue(()));
        assert!(matches!(rl.process(), ControlFlow::Break(..)));
    }

    #[test]
    fn test_token_accumulation_under_high_frequency() {
        // 1 token every 100ms
        let rl = TokenBucket::new(
            NonZeroUsize::new(10).unwrap(),
            1,
            Duration::from_millis(100),
        );

        // Call process every 30ms for 4 iterations (Total 120ms)
        // In a correct implementation, at least 1 token should have been added.
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(30));
            let _ = rl.process();
        }

        // Total time elapsed: ~90-100ms.
        // If we wait one more tiny bit, we should definitely have a token.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            rl.process(),
            ControlFlow::Continue(()),
            "Token should have accumulated despite frequent calls"
        );
    }

    #[tokio::test]
    async fn test_token_accumulation_deterministic() {
        // This freezes time!
        tokio::time::pause();

        let rl = TokenBucket::new(
            NonZeroUsize::new(10).unwrap(),
            1,
            Duration::from_millis(100),
        );

        // Advance time manually without relying on the OS clock
        for _ in 0..3 {
            tokio::time::advance(Duration::from_millis(30)).await;
            let _ = rl.process();
        }

        // Total 90ms. Advance 11ms to hit 101ms.
        tokio::time::advance(Duration::from_millis(11)).await;

        assert_eq!(
            rl.process(),
            ControlFlow::Continue(()),
            "Token should have accumulated at 101ms"
        );
    }
}
