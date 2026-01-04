use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use quanta::Clock;
use quanta::Instant;

use super::Reason;
use super::Strategy;

/// A classic Token Bucket algorithm.
///
/// It maintains a "bucket" of tokens that is replenished over time.
/// This allows for bursts of traffic up to the bucket's capacity while
/// maintaining a steady average rate.
#[derive(Debug)]
pub struct TokenBucket {
    capacity_units: u64,
    /// Number of units (tokens * 10^9) added per nanosecond.
    refill_rate_units_per_ns: f64,
    /// Current units in bucket (scaled by 10^9).
    units: AtomicU64,
    last_update_ns: AtomicU64,
    clock: Clock,
    anchor: Instant,
}

impl TokenBucket {
    const UNITS_SCALE: u64 = 1_000_000_000; // Cost of 1 token

    /// Creates a new `TokenBucket`.
    ///
    /// # Arguments
    /// * `capacity` - Max tokens the bucket can hold (burst size).
    /// * `increment` - Tokens added during the period (steady-state rate).
    /// * `period` - The duration over which `increment` is added.
    pub fn new(capacity: NonZeroUsize, increment: NonZeroUsize, period: Duration) -> Self {
        let clock = Clock::new();
        let anchor = clock.now();

        let refill_rate_units_per_ns = if period.as_nanos() > 0 {
            (increment.get() as u64 * Self::UNITS_SCALE) as f64 / period.as_nanos() as f64
        } else {
            0f64
        };
        Self {
            capacity_units: capacity.get() as u64 * Self::UNITS_SCALE,
            refill_rate_units_per_ns,
            // Start with a full bucket
            units: AtomicU64::new(capacity.get() as u64 * Self::UNITS_SCALE),
            last_update_ns: AtomicU64::new(0),
            clock,
            anchor,
        }
    }
}

impl Strategy for TokenBucket {
    #[inline]
    fn process(&self) -> ControlFlow<Reason> {
        let now = self.clock.now().duration_since(self.anchor).as_nanos() as u64;
        let mut spins = 0;

        loop {
            if spins > 10 {
                std::thread::yield_now();
            }

            let last_update = self.last_update_ns.load(Ordering::Acquire);
            let current_units = self.units.load(Ordering::Acquire);

            // 1. Calculate how many units have accumulated since the last call
            let elapsed = now.saturating_sub(last_update);

            let refill = (elapsed as f64 * self.refill_rate_units_per_ns).floor() as u64;
            let new_units = std::cmp::min(self.capacity_units, current_units + refill);

            // 2. Check if we have at least 1 full token (10^9 units)
            if new_units < Self::UNITS_SCALE {
                let missing_units = Self::UNITS_SCALE - new_units;
                // Time until 1 token is available
                let wait_ns = if self.refill_rate_units_per_ns > 0f64 {
                    (missing_units as f64 / self.refill_rate_units_per_ns).ceil() as u64
                } else {
                    // You will never get any more tokens, if there are no refills.
                    // There's no "good" answer here, so let's make it a second
                    Self::UNITS_SCALE
                };

                return ControlFlow::Break(Reason::Overloaded {
                    retry_after: Duration::from_nanos(wait_ns),
                });
            }

            // 3. Atomically update state
            if self
                .last_update_ns
                .compare_exchange_weak(last_update, now, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                self.units
                    .store(new_units - Self::UNITS_SCALE, Ordering::Release);
                return ControlFlow::Continue(());
            }
            // If CAS fails, another thread updated time; loop and recalculate.
            spins += 1;
        }
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
            NonZeroUsize::new(1).unwrap(),
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
            NonZeroUsize::new(1).unwrap(),
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
            NonZeroUsize::new(1).unwrap(),
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
