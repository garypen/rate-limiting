# Tower Shot 🥃

A high-performance, atomic-backed rate limiting middleware for `tower` and `axum` that solves the "Buffer Bloat" problem.

## Why Tower Shot?

The native `tower` rate limiter is not `Clone` and usually requires the use of `tower::buffer::Buffer` to make a service stack shareable. This introduces significant challenges:
1.  **Buffer Bloat:** Requests sit in the buffer while waiting for a rate-limit permit. This "invisible queue" increases latency and memory usage.
2.  **Configuration Complexity:** Placing the `Buffer` before or after other layers (like `Timeout`) changes the failure semantics (e.g., does the timeout include the queue time?).
3.  **Performance overhead:** Under high contention, the channel-based design of `Buffer` becomes a bottleneck.

`tower-shot` solves these issues by providing **atomic, lock-free strategies** and a **Managed Architecture** that eliminates the need for buffers entirely.

## Choose Your Mode

`tower-shot` offers three distinct ways to handle rate limiting, depending on your application's priorities:

| Mode | Component | Best For... | Behavior |
| :--- | :--- | :--- | :--- |
| **Raw** | `RateLimitLayer` | **Standard Compliance** | A drop-in replacement for `tower::limit::RateLimit`. Returns `Poll::Pending` when full. Requires you to handle backpressure (usually via `Buffer`). |
| **Latency** | `make_latency_svc` | **Protecting P99 / SLA** | **Fail Fast**. If the limit is reached, it rejects the request *immediately* (in nanoseconds). No queuing. Keeps accepted requests fast. |
| **Throughput** | `make_timeout_svc` | **Max Throughput** | **Wait & Retry**. If the limit is reached, it retries until a permit is available or the timeout is reached. Bounds the wait time. |

---

## The Proof: Stress Testing

We subjected the system to a massive burst of **50,000 concurrent requests** against a **10,000 req/s** limit.

### 1. Raw Rate Limit (Direct Backpressure)
*Using `RateLimitLayer` directly (no Buffer).*
Unlike native Tower, `tower-shot` services are cloneable and share state without a `Buffer`.
*   **Result:** The system enforces the limit by returning `Poll::Pending`. The runtime handles the "queue" of waiting tasks.
*   **Latency:** The P99 time to acquire a permit is **~4.0 seconds**.
*   **Risk:** Unbounded waiting; clients hang for seconds.

### 2. Managed Latency (The "Fail Fast" Solution)
*Using `make_latency_svc`.*
*   **Result:** The system aggressively sheds load. Only ~2,500 requests (the ones that fit purely in the window) are accepted.
*   **Latency:** The P99 time to acquire a permit is **~541 nanoseconds**.
*   **Benefit:** The service remains completely responsive. No "death spiral."

### 3. Managed Throughput (The Balanced Solution)
*Using `make_timeout_svc`.*
*   **Result:** The system processes ~6,000 req/s (close to the Raw limit).
*   **Latency:** The P99 time is bounded by the timeout (e.g., 3.0s).
*   **Benefit:** Maximizes successful requests while strictly enforcing a maximum wait time.

### 4. Standard Tower Rate Limit (Buffered)
*Using `tower::limit::RateLimit` wrapped in `tower::buffer::Buffer`.*
Native Tower rate limiters are not shareable by default, requiring a `Buffer` (actor pattern) to enforce a global limit across tasks.
*   **Result:** Throughput and latency are comparable to the Raw strategy, but with the additional overhead of the Buffer actor.
*   **Latency:** The P99 time to acquire a permit is **~3.0 seconds**.

| Metric | Raw (Direct) | Managed Throughput | Managed Latency |
| :--- | :--- | :--- | :--- |
| **Success Rate** | ~6,260 req/s | ~6,035 req/s | ~2,500 req/s |
| **P50 Ready Time** | ~2.0 s | ~1.0 s | **125 ns** |
| **P99 Ready Time** | ~4.0 s | ~3.0 s | **541 ns** |
| **Failure Mode** | Unbounded Waiting | Timeout | **Immediate Rejection** |

---

## Performance Benchmarks

`tower-shot` is designed to be the fastest rate limiter in the ecosystem.

### 1. High Contention (Scaling)
When **1,000 concurrent tasks** compete for a permit, `tower-shot`'s atomic design shines.
*   **`tower-shot` (Managed):** ~172 µs
*   **`governor` (Managed):** ~317 µs
*   **`tower` (Native Managed):** ~13,500 µs (13.5 ms)

> **Result:** `tower-shot` is **~80x faster** than native Tower and **~1.8x faster** than Governor under high contention.

### 2. Load Shedding Speed
When the system is overloaded, how fast can we say "No"?
*   **`tower-shot`:** ~125 ns
*   **`governor`:** ~265 ns

---

## Installation

```toml
[dependencies]
# tower-shot = "0.1.0"
# shot-limit = "0.1.0"
tower-shot = { version = "0.1.0", git = "https://github.com/garypen/rate-limiting.git", branch = "main" }
shot-limit = { version = "0.1.0", git = "https://github.com/garypen/rate-limiting.git", branch = "main" }
tower = "0.5"
axum = "0.8"
tokio = { version = "1", features = ["full"] }
```

## Usage Examples

### 1. Maximize Throughput (Wait & Retry)
Use this when you want to handle as many requests as possible, even if they have to wait a bit.

```rust
use std::time::Duration;
use std::sync::Arc;
use shot_limit::TokenBucket;
use tower_shot::make_timeout_svc;

// 1. Define Strategy: 100 requests per second
let strategy = Arc::new(TokenBucket::new(100, 10, Duration::from_secs(1)));

// 2. Wrap your service
// Requests will wait up to 500ms for a permit before timing out.
let service = make_timeout_svc(
    strategy, 
    Duration::from_millis(500), 
    my_service
);
```

### 2. Protect Latency (Fail Fast)
Use this for critical APIs where a slow response is worse than an error.

```rust
use shot_limit::FixedWindow;
use tower_shot::make_latency_svc;

// 1. Define Strategy
let strategy = Arc::new(FixedWindow::new(100, Duration::from_secs(1)));

// 2. Wrap your service
// Requests are rejected IMMEDIATELY if the limit is exceeded.
let service = make_latency_svc(strategy, my_service);
```

### 3. Raw Layer (Custom Composition)
Use this with `axum` or `ServiceBuilder` if you want to manage the stack manually.

```rust
use tower::ServiceBuilder;
use tower_shot::RateLimitLayer;

let app = Router::new()
    .route("/", get(|| async { "Hello!" }))
    .layer(
        ServiceBuilder::new()
            .timeout(Duration::from_secs(2)) // Handle the wait
            .layer(RateLimitLayer::new(strategy)) // Returns Pending when full
    );
```

## Error Handling

`tower-shot` provides a unified `ShotError` that integrates with `axum`.

| Error | HTTP Status | Meaning |
| :--- | :--- | :--- |
| `ShotError::Overloaded` | `503 Service Unavailable` | Limit reached (Latency mode). |
| `ShotError::Timeout` | `408 Request Timeout` | Wait time exceeded (Throughput mode). |
| `ShotError::Inner(e)` | `500 Internal Server Error` | Application error. |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.