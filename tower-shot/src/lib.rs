//! # Tower Shot
//!
//! `tower-shot` is a high-performance, production-ready rate limiting stack built for
//! the [Tower](https://github.com/tower-rs/tower) ecosystem.
//!
//! ## The Managed Stack
//! Unlike raw rate limiters that return `Poll::Pending` when full, this crate provides
//! the [`ManagedRateLimitLayer`]. This is a pre-composed stack designed to handle
//! common production requirements:
//!
//! 1. **Load Shedding**: Immediately rejects requests with `ShotError::Overloaded`
//!    if the service is at peak capacity, preventing memory exhaustion.
//! 2. **Timeouts**: Queues requests until the provided [`shot_limit::Strategy`] allows them,
//!    failing with `ShotError::Timeout` if the wait exceeds a defined duration.
//! 3. **Error Mapping**: Automatically converts internal Tower errors (like
//!    `tower::timeout::error::Elapsed`) into a unified, cloneable [`ShotError`] domain.
//!
//! ## Feature Flags
//!
//! - `axum`: Enables `IntoResponse` for [`ShotError`], allowing automatic conversion
//!   to HTTP status codes (408, 503, 500).

mod error;
mod layer;
mod managed_layer;
mod service;

#[cfg(test)]
mod tests;

#[cfg(doc)]
use shot_limit::Strategy;

// ... your code ...
pub use error::ShotError;
pub use layer::RateLimitLayer;
pub use managed_layer::ManagedRateLimitLayer;
pub use service::RateLimitService;
