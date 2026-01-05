# Tower Shot 🥃

A high-performance, atomic-backed rate limiting middleware for `tower` and `axum` that prioritizes **latency protection** over "lucky" successes.

## Why Tower Shot?

Standard rate limiters often lead to **Buffer bloat**. When a burst hits, requests queue up in deep buffers, leading to multi-second tail latencies even for "successful" requests. 

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

## Installation

Add `tower-shot` and `shot-limit` to your `Cargo.toml`:

```toml
[dependencies]
# tower-shot = "0.1.0"
# shot-limit = "0.1.0"
# Until I release to crates.io, you can use git
tower-shot = { version = "0.1.0", git = "https://github.com/garypen/rate-limiting.git", branch = "main" }
shot-limit = { version = "0.1.0", git = "https://github.com/garypen/rate-limiting.git", branch = "main" }
tower = { version = "0.5.2", features = ["full"] }
axum = "0.8.8"
tokio = { version = "1.48.0", features = ["full"] }

## Quick Start

The following example demonstrates how to set up a `ManagedRateLimitLayer` using a `TokenBucket` strategy in an Axum application. 



```rust
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::routing::get;

use shot_limit::TokenBucket;
use tower::BoxError;
use tower::ServiceBuilder;
use tower_shot::ManagedRateLimitLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Define the rate limiting strategy
    let capacity = 100.try_into()?;
    let refill_amount = 10.try_into()?;
    let period = Duration::from_secs(1);
    
    // Using Arc allows the strategy to be shared across threads
    let strategy = Arc::new(TokenBucket::new(capacity, refill_amount, period));

    // 2. Configure the managed layer with a 500ms timeout
    // ManagedRateLimitLayer does not use internal buffers.
    let timeout = Duration::from_millis(500);
    let managed_layer = ManagedRateLimitLayer::new(strategy, timeout);

    // 3. Build the Axum router
    let app = Router::new()
        .route("/", get(|| async { "Hello, Shot!" }))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|err: BoxError| async move {
                    (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        format!("Rate limit exceeded: {}", err),
                    )
                }))
                .layer(managed_layer)
                .map_err(BoxError::from),
        );

    // 4. Run the server
    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("📡 Listening on http://{}", addr);
    
    axum::serve(listener, app).await?;

    Ok(())
}
```

### Choosing Your Layer: Standard vs. Managed

`tower-shot` provides two primary layers to balance raw performance with operational safety. Both are designed without internal buffers to ensure predictable resource consumption.



| Feature | Standard Rate Limit | Managed Rate Limit |
| :--- | :--- | :--- |
| **Buffering** | None | **None** |
| **Strategy** | Any `shot-limit` Strategy | Any `shot-limit` Strategy |
| **Failure Mode** | Fails immediately if no permit | Fails after `timeout` duration |
| **Typical Use** | Low-latency internal APIs | Public-facing SLAs |
| **Overhead** | ~104 ns | ~211 ns |

#### When to use Standard
The **Standard** layer is the absolute fastest path. It is ideal for high-volume internal microservices where you want a "hard wall" and expect the client to handle retries or backoff immediately.

#### When to use Managed
The **Managed** layer should be your default for user-facing applications. It provides a small "wait window" defined by your `timeout`. 



Even without a buffer, the Managed layer allows requests to wait briefly for the next available token (e.g., waiting 50ms for a bucket refill). If a permit isn't available by the end of the timeout, it fails fast to protect your system from "hanging" requests.

## Features

- **Atomic Strategies**: Uses `shot-limit` for lock-free, $O(1)$ decision making.
- **Managed Stack**: Pre-composed `Timeout` + `LoadShed` + `RateLimit` layers.
- **Axum Integration**: Native `IntoResponse` implementation for `ShotError`.
  - `408 Request Timeout`: Returned when your Latency SLA is exceeded.
  - `503 Service Unavailable`: Returned when the rate limit is hit (Load Shedding).
- **Zero Buffer bloat**: Non-blocking approach that rejects traffic at `poll_ready` rather than queueing indefinitely.



---

## Error Handling

Tower Shot categorizes failures so your clients can react appropriately without custom middleware:

| Error Variant | HTTP Status | Meaning |
| :--- | :--- | :--- |
| `ShotError::Overloaded` | `503` | Rate limit reached; request shed to protect resources. |
| `ShotError::Timeout` | `408` | Request passed the limiter but the inner service was too slow. |
| `ShotError::Inner(e)` | `500` | The underlying service returned an error. |

---

## Performance

`tower-shot` is designed for high-throughput services where middleware overhead must be kept to an absolute minimum. In our benchmarks, `tower-shot` consistently outperforms the native Tower implementation by a factor of **59x** and even edges out specialized crates like `governor`.

### Benchmarks

The crate includes the various benchmarks and test we executed to generate our performance comparisons.
The testing is performed on a 2021 Mac M1 laptop.

```bash
# Run micro-benchmarks to see atomic overhead
cargo bench

# Run the resilience stress test to see SLA enforcement
cargo run --bin stress_test --release
```

### Latency Comparison

The following table shows the raw overhead introduced by the middleware for a single request (Lower is better):

| Implementation | Latency (ns) | Relative Speed |
|:---|:---:|:---:|
| `tower::limit::RateLimit` | 6214.2 ns | 1x |
| `governor` | 129.40 ns | 48x faster |
| **`tower-shot` (Standard)** | **104.34 ns** | **59x faster** |
| **`tower-shot` (Managed)** | **210.96 ns** | **29x faster** |

### High Contention Scaling

When under pressure from **1,000 concurrent tasks** competing for permits, `tower-shot` maintains its lead by minimizing lock contention:

* **`tower-shot` (Standard):** 171.64 µs
* **`governor`:** 197.77 µs
* **`tower::limit::RateLimit`:** 483.48 µs

### Key Takeaways

* **Negligible Overhead:** Adding the standard `RateLimitLayer` adds only **~104 nanoseconds** to your request path—virtually invisible in most networked applications.
* **Predictable Stability:** While native Tower implementations often show significant jitter (up to **17% outliers**) under load, `tower-shot` remains stable with significantly fewer timing outliers.
* **Managed Efficiency:** The "Managed" layer provides failsafe timeouts and backpressure **without using internal buffers**, ensuring that even your managed paths remain **29x faster** than the basic native Tower limiter.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
