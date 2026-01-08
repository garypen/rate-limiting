//! # Tower Shot
//!
//! `tower-shot` is a high-performance, production-ready rate limiting stack built for
//! the [Tower](https://github.com/tower-rs/tower) ecosystem.
//!
//! ## The Managed Stack
//! Unlike raw rate limiters that return `Poll::Pending` when full, this crate provides
//! "managed" layers. These are pre-composed stacks designed to handle common production requirements:
//!
//! 1. **[`ManagedThroughputLayer`]**: Maximizes throughput by retrying requests that are
//!    rate-limited, within a hard timeout.
//! 2. **[`ManagedLatencyLayer`]**: Immediately rejects requests with `ShotError::Overloaded`
//!    if the rate limit is reached, preventing memory exhaustion.
//!
//! Both layers also provide:
//! - **Timeouts**: Bounded execution time for the entire request process.
//! - **Error Mapping**: Automatically converts internal Tower errors (like
//!   `tower::timeout::error::Elapsed`) into a unified, cloneable [`ShotError`] domain.
//!
//! ## Feature Flags
//!
//! - `axum`: Enables `IntoResponse` for [`ShotError`], allowing automatic conversion
//!   to HTTP status codes (408, 503, 500).

mod error;
mod latency_layer;
mod layer;
mod retrylimit_layer;
mod retrylimit_service;
mod service;
mod throughput_layer;

#[cfg(test)]
mod tests;

#[cfg(doc)]
use shot_limit::Strategy;

pub use error::ShotError;
pub use latency_layer::ManagedLatencyLayer;
pub use layer::RateLimitLayer;
pub use service::RateLimitService;
pub use throughput_layer::ManagedThroughputLayer;
