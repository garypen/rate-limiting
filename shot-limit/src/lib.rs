//! # shot-limit
//!
//! `shot-limit` provides high-performance, lock-free rate limiting strategies.
//!
//! ## Core Philosophy
//!
//! Most rate limiters rely on a `Mutex` to protect internal state, which creates a bottleneck
//! under high thread contention. `shot-limit` uses atomic primitives and Compare-And-Swap (CAS)
//! loops to ensure that state transitions are non-blocking and scale linearly with CPU cores.
//!
//! ## Key Concepts
//!
//! * **Lock-Free**: No `Mutex` or `RwLock` in the hot path.
//! * **Lazy Evaluation**: Tokens and windows are recalculated at the moment of the request,
//!   eliminating the need for background worker threads or timers.
//! * **Strategy Trait**: A unified interface for different limiting algorithms.
//!
//! ## Example
//!
//! ```rust
//! use shot_limit::TokenBucket;
//! use shot_limit::Strategy;
//! use std::time::Duration;
//! use std::num::NonZeroUsize;
//!
//! let limit = NonZeroUsize::new(100).unwrap();
//! let period = Duration::from_secs(60);
//! let bucket = TokenBucket::new(limit, 100, period);
//!
//! if bucket.process().is_continue() {
//!     // Request allowed
//! }
//! ```

use std::fmt::Debug;
use std::ops::ControlFlow;
use std::time::Duration;

mod fixed_window;
mod sliding_window;
mod token_bucket;

pub use fixed_window::FixedWindow;
pub use sliding_window::SlidingWindow;
pub use token_bucket::TokenBucket;

/// Reasons why a request might be rejected by a strategy.
#[derive(Debug, PartialEq)]
pub enum Reason {
    Overloaded { retry_after: Duration },
}

/// The core trait for all rate-limiting algorithms.
///
/// Strategies must be `Send` and `Sync` to allow sharing across thread boundaries
/// via `Arc`.
pub trait Strategy: Debug {
    /// Attempts to process a single request.
    ///
    /// This method is non-blocking and uses atomic operations to update
    /// internal state.
    ///
    /// # Errors
    ///
    /// Returns `Reason` if the rate limit has been reached.
    fn process(&self) -> ControlFlow<Reason>;
}
