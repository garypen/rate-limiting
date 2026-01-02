# Tower Shot 🥃

A high-performance, atomic-backed rate limiting middleware for `tower` and `axum` that prioritizes **latency protection** over "lucky" successes.

## Why Tower Shot?

Standard rate limiters often lead to **Bufferbloat**. When a burst hits, requests queue up in deep buffers, leading to multi-second tail latencies even for "successful" requests. 

`tower-shot` uses a **Managed Architecture** that pairs atomic rate-limiting strategies with aggressive Load Shedding and Timeout SLAs. 

### The Proof (Stress Test Results)
Under a burst of 5,000 concurrent requests with a 1,000-request capacity:

| Metric | Raw Rate Limiter | **Tower Shot (Managed)** |
| :--- | :--- | :--- |
| **P99 Latency** | **4,030 ms** | **12 ms** |
| **System Health** | Backlogged | Responsive |
| **Failure Mode** | Hidden Latency | **SLA Enforcement** |

> **The Result:** Tower Shot ensures that the 1,000 requests that *can* be handled are processed at near-instant speeds, while excess traffic is shed immediately to protect your P99 and system stability.



---

## Quick Start (Axum)

Tower Shot is designed to plug directly into Axum's type system with zero-boilerplate error handling.

```rust
use tower_shot::{ManagedRateLimitLayer, strategy::TokenBucket};
use std::{sync::Arc, time::Duration, num::NonZeroUsize};
use axum::{Router, routing::get};

#[tokio::main]
async fn main() {
    // 1. Define your strategy (Atomic Token Bucket)
    let limit = NonZeroUsize::new(1000).unwrap();
    let strategy = Arc::new(TokenBucket::new(limit, 1000, Duration::from_secs(1)));

    // 2. Define your Latency SLA (Max execution time)
    let max_wait = Duration::from_millis(500);

    // 3. Create the Managed Layer
    let rate_limit_layer = ManagedRateLimitLayer::new(strategy, max_wait);

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .layer(rate_limit_layer);
    
    // Serve with hyper/tokio...
}
```

## Features

- **Atomic Strategies**: Uses `shot-limit` for lock-free, $O(1)$ decision making.
- **Managed Stack**: Pre-composed `Timeout` + `LoadShed` + `RateLimit` layers.
- **Axum Integration**: Native `IntoResponse` implementation for `ShotError`.
  - `408 Request Timeout`: Returned when your Latency SLA is exceeded.
  - `503 Service Unavailable`: Returned when the rate limit is hit (Load Shedding).
- **Zero Bufferbloat**: Non-blocking approach that rejects traffic at `poll_ready` rather than queueing indefinitely.



---

## Error Handling

Tower Shot categorizes failures so your clients can react appropriately without custom middleware:

| Error Variant | HTTP Status | Meaning |
| :--- | :--- | :--- |
| `ShotError::Overloaded` | `503` | Rate limit reached; request shed to protect resources. |
| `ShotError::Timeout` | `408` | Request passed the limiter but the inner service was too slow. |
| `ShotError::Inner(e)` | `500` | The underlying service returned an error. |

---

## Performance & Resilience

We believe in testing under fire. The crate includes a comprehensive stress test suite to verify behavior under extreme contention.

```bash
# Run micro-benchmarks to see atomic overhead
cargo bench

# Run the resilience stress test to see SLA enforcement
cargo run --bin stress_test --release
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
