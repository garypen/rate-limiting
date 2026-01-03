use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use quanta::Clock;
use quanta::Instant;

use crate::Reason;
use crate::Strategy;

/// Generic Cell Rate Algorithm
#[derive(Debug)]
pub struct Gcra {
    /// Theoretical Arrival Time (TAT) in nanoseconds.
    tat: AtomicU64,
    emission_interval_ns: u64,
    delay_tolerance_ns: u64,
    clock: Clock,
    /// A fixed point in time (TSC tick) to calculate deltas from.
    anchor: Instant,
}

impl Gcra {
    pub fn new(limit: NonZeroUsize, period: Duration) -> Self {
        Self::with_clock(limit, period, Clock::new())
    }
    pub fn with_clock(limit: NonZeroUsize, period: Duration, clock: Clock) -> Self {
        let anchor = clock.now();
        let period_ns = period.as_nanos() as u64;

        Self {
            tat: AtomicU64::new(0),
            emission_interval_ns: period_ns / limit.get() as u64,
            delay_tolerance_ns: period_ns,
            clock,
            anchor,
        }
    }

    #[cfg(test)]
    pub(crate) fn remaining_capacity(&self) -> usize {
        let now_instant = self.clock.now();
        let now = now_instant.duration_since(self.anchor).as_nanos() as u64;
        let tat = self.tat.load(Ordering::Acquire);

        // Total capacity based on your new() math: period / (period/limit) = limit
        let total_capacity = (self.delay_tolerance_ns / self.emission_interval_ns) as u64;

        if tat <= now {
            return total_capacity as usize;
        }

        let diff = tat - now;

        // Use ceiling division to ensure that even a partial interval
        // counts as a 'used' slot.
        // Formula: (numerator + denominator - 1) / denominator
        let used_slots = (diff + self.emission_interval_ns - 1) / self.emission_interval_ns;

        if used_slots >= total_capacity {
            0
        } else {
            (total_capacity - used_slots) as usize
        }
    }
}

impl Strategy for Gcra {
    #[inline]
    fn process(&self) -> ControlFlow<Reason> {
        let now_instant = self.clock.now();
        let now = now_instant.duration_since(self.anchor).as_nanos() as u64;

        loop {
            let tat = self.tat.load(Ordering::Acquire);

            let arrival = if now > tat { now } else { tat };
            let next_tat = arrival + self.emission_interval_ns;

            // Change: Use >= to prevent the 'extra' burst slot.
            // This ensures that if tolerance is 1000ms and interval is 100ms,
            // the 11th request (which would put next_tat at 1100ms) is rejected.
            if next_tat > now + self.delay_tolerance_ns {
                let wait_ns = next_tat - (now + self.delay_tolerance_ns);
                return ControlFlow::Break(Reason::Overloaded {
                    retry_after: Duration::from_nanos(wait_ns),
                });
            }

            if self
                .tat
                .compare_exchange_weak(tat, next_tat, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return ControlFlow::Continue(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcra_flow_with_capacity_check() {
        let limit = NonZeroUsize::new(5).unwrap();
        let period = Duration::from_millis(500);
        let rl = Gcra::new(limit, period);

        assert_eq!(rl.remaining_capacity(), 5);

        for _ in 0..3 {
            let _ = rl.process();
        }
        // If it returns 3, it means the 3rd request hasn't "aged" into a full interval yet.
        let rem = rl.remaining_capacity();
        assert!(
            rem == 2 || rem == 3,
            "Expected 2 or 3 slots remaining, got {}",
            rem
        );

        for _ in 0..2 {
            let _ = rl.process();
        }
        assert_eq!(rl.remaining_capacity(), 0);
    }

    #[tokio::test]
    async fn test_gcra_deterministic_with_mock_clock() {
        // 1. Create a mock clock and a handle to control it
        let (clock, mock) = Clock::mock();

        // 2. Initialize GCRA with the mock clock
        let limit = NonZeroUsize::new(10).unwrap();
        let period = Duration::from_secs(1); // 100ms per slot
        let rl = Gcra::with_clock(limit, period, clock);

        // 3. Exhaust capacity
        for _ in 0..10 {
            let _ = rl.process();
        }
        assert_eq!(rl.remaining_capacity(), 0);
        assert!(rl.process().is_break());

        // 4. Advance time exactly 250ms
        mock.increment(Duration::from_millis(250));

        // 5. Now capacity should be exactly 2
        // (250ms / 100ms interval = 2 slots)
        let rem = rl.remaining_capacity();
        assert_eq!(rem, 2, "Expected exactly 2 slots, found {}", rem);

        assert!(rl.process().is_continue());
        assert_eq!(rl.remaining_capacity(), 1);
    }
}
