//! # Tower Shot
//!
//! `tower-shot` is a high-performance, production-ready rate limiting stack built for
//! the [Tower](https://github.com/tower-rs/tower) ecosystem.
//!
//! It solves the "buffer bloat" problem common in channel-based rate limiters by providing
//! **atomic, lock-free strategies** and a **Managed Architecture**.
//!
//! ## Three Modes of Operation
//!
//! 1. **Raw (`RateLimitLayer`)**:
//!    - **Best for:** Standard Tower compliance, custom stacks.
//!    - **Behavior:** Returns `Poll::Pending` when full. Requires a `Buffer` or custom backpressure handling.
//!
//! 2. **Latency (`make_latency_svc`)**:
//!    - **Best for:** Protecting P99 latency and SLAs (Fail Fast).
//!    - **Behavior:** Immediately rejects requests with [`ShotError::Overloaded`] if the limit is reached.
//!      No queuing, no waiting.
//!
//! 3. **Throughput (`make_timeout_svc`)**:
//!    - **Best for:** Maximizing successful requests (Wait & Retry).
//!    - **Behavior:** If the limit is reached, it waits (retries) for a permit until the specified `timeout`.
//!
//! ## Feature Flags
//!
//! - `axum`: Enables `IntoResponse` for [`ShotError`], allowing automatic conversion
//!   to HTTP status codes:
//!   - `503 Service Unavailable` (Overloaded)
//!   - `408 Request Timeout` (Timeout)
//!   - `500 Internal Server Error` (Inner error)

mod error;
mod layer;
mod service;
mod utils;

#[cfg(test)]
mod tests;

#[cfg(doc)]
use shot_limit::Strategy;

pub use error::ShotError;
pub use layer::RateLimitLayer;
pub use service::RateLimitService;
pub use utils::ServiceBuilderExt;
pub use utils::make_latency_svc;
pub use utils::make_timeout_svc;