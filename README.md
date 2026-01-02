# Shot RateLimit 🦀

A high-performance, atomic-first rate limiting library for the Tower/Axum ecosystem. 

Most rate limiters force excess traffic into high-latency queues. **Shot** is designed for high-concurrency services where the goal is to protect the system and maintain microsecond responsiveness, even under a "Thundering Herd" attack.

## 🏗️ The Core Philosophy: Fail-Fast vs. Wait-in-Line

Choosing a rate-limiting strategy is a trade-off between **Goodput** and **Responsiveness**.



### 1. The "Fail-Fast" Stack (`tower-shot` Managed)
Prioritizes the experience of users who *can* be served immediately. Excess traffic is rejected instantly.
* **Mechanism:** Coordinates `LoadShed` and `Timeout` with `shot-limit` atomic synchronization.
* **Outcome:** **Ultra-Low Latency**. 5,000 requests hitting a 1,000 req/s limit are resolved in **~13ms**.
* **Best For:** Public APIs, interactive microservices, and "Tier 1" infrastructure.

### 2. The "Wait-in-Line" Stack (`tower-shot` Buffered)
Prioritizes processing every request, regardless of wait time.
* **Mechanism:** Excess requests are queued in an internal MPSC buffer.
* **Outcome:** **100% Success Rate**, but **High Latency**. 5,000 requests hitting a 1,000 req/s limit take **~5 seconds** to clear.
* **Best For:** Background workers, database migrations, and non-interactive syncs.

---

## 📊 Performance Benchmarks: The "Thundering Herd"

We simulated **5,000 concurrent requests** hitting a service limited to **1,000 req/s**. 

| Strategy | Mode | Success Rate | P99 (Success) | Total Test Time | Behavior |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Shot Sliding** | **Managed** | 20% | **11.5ms** | **13.3ms** | **Fail-Fast (Responsive)** |
| **Shot Fixed** | **Managed** | 20% | 12.0ms | 12.6ms | Fast Burst Shedding |
| **Shot Sliding** | **Buffered** | 100% | 4964ms | 5.01s | Precise Queueing |
| **Shot Fixed** | **Buffered** | 100% | 4024ms | 4.03s | Simple Queueing |
| **Tower Baseline**| **Buffered** | 100% | 4014ms | 4.02s | Standard Queueing |

---

## 🚀 Quick Start (Axum)

The `ManagedRateLimitLayer` is the "Batteries-Included" stack from `tower-shot`. It is **Bufferless** and **Lock-free**, providing a `Clone` service without the overhead of background worker tasks.

```rust
use std::num::NonZeroUsize;
use std::time::Duration;
use axum::{routing::get, Router};
use tower_shot::ManagedRateLimitLayer;
use shot_limit::SlidingWindow;

#[tokio::main]
async fn main() {
    // 1. Define a precise strategy from shot-limit
    let strategy = SlidingWindow::new(
        NonZeroUsize::new(1000).unwrap(), 
        Duration::from_secs(1)
    );

    // 2. Build the Managed Layer (Shedding + Timeout) from tower-shot
    // No background workers or MPSC channels are spawned.
    let rate_limit_layer = ManagedRateLimitLayer::new(
        strategy,
        Duration::from_millis(500) // Max wait time before failing fast
    );

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .layer(rate_limit_layer);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

```

## 🔬 Key Architectural Features

### 1. Lock-Free Synchronization
Unlike traditional rate limiters that wrap internal state in a `Mutex`, **Shot** utilizes **Lock-free Atomics**. Multiple threads can check and update the limit simultaneously using atomic instructions (like `compare_exchange`). This prevents kernel-level context switches, avoids thread parking, and eliminates "Lock Contention" in high-concurrency environments.



### 2. Bufferless "Shot" Architecture
Most Tower layers require `tower::buffer::Buffer` to satisfy `Clone` bounds, which spawns a background worker and an MPSC channel. `tower-shot` avoids this overhead entirely. By using internal `Arc` state and manual `Clone` implementations, the layer remains cloneable and thread-safe while staying "naked" on the executor.



### 3. Reactor-Aware Self-Waking
When used in **Buffered** mode, the service doesn't just "sleep." It calculates the exact nanosecond wait time required and registers a `tokio::time::sleep` future only when the strategy allows. This ensures the CPU remains idle until the exact moment the limit window opens, minimizing wake-up jitter.

---

## 🕒 Algorithm Selection (`shot-limit`)

| Algorithm | Raw Overhead | Precision | Best Use Case |
| :--- | :--- | :--- | :--- |
| **Fixed Window** | **26 ns** | Low | Internal high-throughput tasks; accepts boundary bursts. |
| **Token Bucket** | **28 ns** | Medium | Interactive APIs; allows initial bursts for page loads. |
| **Sliding Window** | **45 ns** | **Highest** | Public "Tier 1" APIs; prevents any boundary gaming. |



---

### Implementation Note: Shared State
All `shot-limit` strategies are designed to be shared. When you `clone()` a Service or Layer in `tower-shot`, they share the underlying atomic state. **You do not need to wrap these in an extra Arc yourself**; the library handles reference counting internally to ensure the limit is enforced globally across all clones.
