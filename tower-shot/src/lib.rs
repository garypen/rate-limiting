//! # Tower Shot
//!
//! `tower-shot` is a high-performance, production-ready rate limiting stack built for
//! the [Tower](https://github.com/tower-rs/tower) ecosystem.
//!
//! ## The Managed Stack
//! The rate limiter returns `Poll::Pending` when full. You can use `make_timeout_svc`
//! or `make_latency_svc` to handle common production requirements:
//!
//! 1. **[`make_timeout_svc`]**: Maximizes throughput by enforcing a hard timeout.
//! 2. **[`make_latency_svc`]**: Immediately rejects requests with `ShotError::Overloaded`
//!    if the rate limit is reached, preventing memory exhaustion.
//!
//! Both approaches also provide:
//! - **Timeouts**: Bounded execution time for the entire request process.
//! - **Error Mapping**: Automatically converts internal Tower errors (like
//!   `tower::timeout::error::Elapsed`) into a unified, cloneable [`ShotError`] domain.
//!
//! ## Feature Flags
//!
//! - `axum`: Enables `IntoResponse` for [`ShotError`], allowing automatic conversion
//!   to HTTP status codes (408, 503, 500).

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
