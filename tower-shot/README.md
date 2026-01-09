# Tower Shot 🥃

A high-performance, atomic-backed rate limiting middleware for `tower` and `axum` that prioritizes **latency protection**.

## Why Tower Shot?

The `tower` rate limiter is not `Clone` and usually requires the use of `tower::buffer::Buffer` to make a service stack Cloneable (which is a requirement for most web frameworks, such as `axum`).

The positioning of `Buffer` is a fairly complicated (nuanced) business and can easily lead to issues with memory consumption that only become apparent at scale (i.e.: the worst kinds of issues...)

Here's a brief explanation of why...

Let's imagine you have a service and you wish to make sure that it doesn't take more than 1 second to complete, you want to accept 1,000 requests/second and you want to shed load if either of these two requirements are broken.

You are using the `tower::ServiceBuilder` to create your service stack and you are considering how to order your services. You know that you have to put your timeout() ahead of your load_shed() (so that the timeout acts as the "failsafe" for your requests). But where should you put the buffer?

### Option 1.(Let's call this the Responsive Option)

```rust
let tower_svc = ServiceBuilder::new()
    .buffer(capacity.get()) // Buffer size
    .timeout(timeout)
    .load_shed()
    .rate_limit(capacity.get() as u64, period)
    .service(service_fn(mock_db_call));
```

If we put the buffer at the top of the stack, our timeout is now no longer measuring the total request time, but only the time remaining in the inner services. The time spent queuing in the buffer is now unconstrained and this can lead to memory management issues.

On the plus side, the reponse to the client is snappy, since the likelihood is that a Buffer will be ready to accept a request.

### Option 2. (Let's call this the Production Option)

```rust
let tower_svc = ServiceBuilder::new()
    .timeout(timeout)
    .load_shed()
    .buffer(capacity.get()) // Buffer size
    .rate_limit(capacity.get() as u64, period)
    .service(service_fn(mock_db_call));
```

If we put the buffer immediately above the rate_limit(), then our timeout() is measuring the total request time. That's good, we are now getting the memory safe behaviour we'd like, requests that queue in our buffer for too long will be timed out. This is the safest way to use Tower rate limiting if your service must be `Clone`.

### Summary of alternatives

However we configure the built in tower Rate Limit, we are going to have problems. Option 1 results in a lack of control and potential buffer bloat. Option 2 will result in excessive timeouts when loads are high.

(That's all assuming you've managed to configure the size of your `buffer` layer correctly. It's pretty tricky to get this completely right, but for these illustrations I've just set it to the rate limit capacity and I think that's good enough.)

There is a short stress testing program that tries to illustrate all of this, `src/bin/stress_test.rs`. See the Benchmarks section for details on how to run it. Be prepared to process a lot of numbers...

`tower-shot` uses a **Managed Architecture** that pairs atomic rate-limiting strategies with aggressive Load Shedding and Timeout SLAs. 

Configuring and using `tower-shot` is both simpler, since buffer is not required, and more expressive, since you can choose the rate limiting strategy which best represents your goals. For example, if you want to continue using a Fixed Window (which is the strategy supported by the tower Rate Limit) strategy rate limit, your drop-in replacement would look like this:

```rust
let fixed = Arc::new(FixedWindow::new(capacity, period));
let fixed_layer = ManagedThroughputLayer::new(fixed, timeout);
let tower_svc = ServiceBuilder::new()
    .timeout(timeout)
    .layer(fixed_layer)
    .service(service_fn(mock_db_call));
```

### The Proof (Stress Test Results)
Under a burst of 50,000 concurrent requests with a 10,000-request capacity:

| Metric | Raw Rate Limiter | **Tower Shot (Managed Retry)** |
| :--- | :--- | :--- |
| **P99 Latency** | **4,500 ms** | **0.5 ms** |
| **System Health** | Severely Backlogged | Responsive |
| **Failure Mode** | Unbounded Latency | **SLA Enforcement** |

> **The Result:** Tower Shot ensures that the 10,000 requests that *can* be handled are processed at near-instant speeds, while excess traffic is shed or retried efficiently to protect your P99 and system stability.


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
```

## Quick Start

The following example demonstrates how to set up a `ManagedThroughputLayer` using a `TokenBucket` strategy in an Axum application. 



```rust
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::routing::get;

use shot_limit::TokenBucket;
use tower::BoxError;
use tower::ServiceBuilder;
use tower_shot::ManagedThroughputLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Define the rate limiting strategy
    let capacity = 100.try_into()?;
    let refill_amount = 10.try_into()?;
    let period = Duration::from_secs(1);
    
    // Using Arc allows the strategy to be shared across threads
    let strategy = Arc::new(TokenBucket::new(capacity, refill_amount, period));

    // 2. Configure the managed layer with a 500ms timeout
    // ManagedThroughputLayer handles retries and timeouts automatically.
    let timeout = Duration::from_millis(500);
    let managed_layer = ManagedThroughputLayer::new(strategy, timeout);

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

`tower-shot` provides several layers to balance raw performance with operational safety. 



| Feature | `RateLimitLayer` | `ManagedThroughputLayer` | `ManagedLatencyLayer` |
| :--- | :--- | :--- | :--- |
| **Strategy** | Any `shot-limit` | Any `shot-limit` | Any `shot-limit` |
| **Failure Mode** | Return `Poll::Pending` | Retries, then `ShotError::Timeout` | Immediate `ShotError::Overloaded` |
| **Best For** | Internal microservices | User-facing APIs (Max Throughput) | Critical Load Protection |

#### When to use `RateLimitLayer`
The absolute fastest path. Ideal for internal microservices where the client handles backoff. Note: You will need to decide how to handle `Poll::Pending` yourself.

#### When to use `ManagedThroughputLayer`
Ideal for maximizing throughput. It allows requests to wait briefly (retrying) if tokens aren't immediately available, up to a hard timeout.

#### When to use `ManagedLatencyLayer`
Ideal for protecting services from "buffer bloat". It immediately rejects excess traffic, ensuring that the requests that *are* accepted are processed with minimal latency.

## Features

- **Atomic Strategies**: Uses `shot-limit` for lock-free, $O(1)$ decision making.
- **Managed Stack**: Pre-composed `Timeout` + `LoadShed`/`Retry` + `RateLimit` layers.
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
| `ShotError::Timeout` | `408` | Request passed the limiter but the entire process (including retries) took too long. |
| `ShotError::Inner(e)` | `500` | The underlying service returned an error. |

---

## Performance

`tower-shot` is designed for high-throughput services where middleware overhead must be kept to an absolute minimum. In our benchmarks, `tower-shot`'s Managed layers outperform the native Tower implementation by a factor of **~450x** in load-shedding scenarios and **~45x** under high contention, while performing on par with `governor`.

### Benchmarks

The crate includes the various benchmarks and tests we executed to generate our performance comparisons.

The testing is performed on a 2021 Mac M1 laptop.

```bash
# Run micro-benchmarks to see atomic overhead
cargo bench

# Run the resilience stress test to see SLA enforcement
cargo run --bin stress_test --release
```

### Latency Comparison (Saturated Load)

The following table shows the behavior of the middleware when the system is fully saturated (10,000 req/s capacity).

| Implementation | Latency | Behavior |
|:---|:---:|:---:|
| `tower::limit::RateLimit` | 118.5 µs | Buffered Wait (High Latency) |
| **`tower-shot` (Standard)** | **99.7 µs** | **Precise Wait (Target: 100 µs)** |
| **`tower-shot` (Managed)** | **263 ns** | **Fast Rejection (Load Shed)** |
| `governor` (Managed) | 244 ns | Fast Rejection (Load Shed) |

**Note:** The `governor` benchmark uses a `Service` adapter that correctly implements the Tower `poll_ready` contract. In this saturated configuration, `tower-shot`'s optimized atomic implementation proves to be highly accurate, while the Managed Layer offers a failure mode (Load Shedding) that is **450x faster** than buffering.

### High Contention Scaling

When multiple tasks compete for permits, lock contention and context switching become significant performance bottlenecks. This is especially true for the native `tower` rate limiter, which relies on a `Buffer` (and its underlying channel) to support concurrent access.

Our "High Contention" benchmark simulates this by launching **1,000 concurrent tasks** that all attempt to acquire a permit simultaneously.

The results show that `tower-shot`'s atomic design effectively eliminates this bottleneck, processing requests **45x faster** than the native Tower implementation and maintaining parity with `governor`:

* **`tower-shot` (Managed):** 310 µs
* **`governor` (Managed):** 320 µs
* **`tower` (Native Managed):** 14,055 µs (14 ms)

> **The Difference:** The native Tower implementation forces all requests through a single channel (the `Buffer`), creating a serialization point that degrades performance under load. `tower-shot` avoids this entirely, using atomic counters to handle concurrent requests in parallel.

### Key Takeaways

* **Precise Control:** The standard `RateLimitLayer` enforces the 100µs rate limit with ~0.3% error margin.
* **Architectural Superiority:** By avoiding the need for `tower::Buffer` and intermediate channels, `tower-shot` eliminates the bottlenecks found in native Tower rate limiting.
* **Managed Efficiency:** The "Managed" layers provide failsafe timeouts and backpressure **without using internal buffers**, ensuring that your load-shedding path remains **45x faster** (under contention) than the basic native Tower limiter.

### Comparison with `governor`

Based on our benchmarks and architectural design, `tower-shot` and `governor` are performance peers, but they offer different strengths:

1.  **Performance (A Dead Heat):**
    *   **Precision:** Both enforce rate limits with extreme accuracy (within ~0.3%).
    *   **Load Shedding:** Both offer sub-microsecond rejection speeds (245ns vs 267ns).
    *   **High Contention:** `tower-shot` performs slightly better under extreme pressure (1,000 tasks), processing bursts in **311µs** compared to `governor`'s **326µs**.

2.  **Architectural Philosophy:**
    *   **`governor`** is the gold standard for generic, feature-rich rate limiting in Rust. It requires third-party or custom adapters to integrate with Tower's `poll_ready` contract.
    *   **`tower-shot`** is an operations-focused middleware suite designed specifically to solve the **Tower Buffer problem**. It provides native, pre-composed layers (Managed vs Standard) and choice of "Retry-until-Timeout" or "Shed-immediately" failure modes without configuration complexity.

**Conclusion:** If you need a highly generic, standalone rate limiting library, choose `governor`. If you are building an **Axum or Tower-based service** and want high-performance rate limiting that protects your P99 latency out-of-the-box, `tower-shot` is optimized for you.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
