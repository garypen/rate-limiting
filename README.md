# Shot ✨

A high-performance, atomic-based rate limiting ecosystem for Rust services. 

## The Ecosystem

This project is a workspace consisting of two specialized crates designed to work in tandem:

1.  **`shot-limit`**: The engine. Provides lock-free, atomic-based rate limiting strategies (Token Bucket, Fixed Window, Sliding Window). Optimized for $O(1)$ performance and zero thread contention.
2.  **`tower-shot`**: The armor. A `tower` middleware layer that wraps the core strategies with **Managed Resilience**, adding Load Shedding and Latency SLAs (Timeouts).

## Why Choose Shot?

Traditional rate limiters often focus only on "allowing" or "denying" requests, ignoring the impact of queuing and downstream latency. **Shot** is built on a "Shed-First" philosophy:

* **Zero-Trust Latency**: Prevents "Buffer Bloat" by rejecting excess traffic in nanoseconds.
* **Predictable P99s**: Integrated timeouts ensure that if a request can't be handled within your SLA, it's failed quickly to save system resources.
* **Atomic Precision**: By using atomics instead of `Mutex` locks, the system scales linearly with your CPU core count.

### Performance at a Glance (Apple M1)

| Metric | Result |
| :--- | :--- |
| **Throughput** | >300M operations / second |
| **Contention Overhead** | ~3.3ns per check (8 threads) |
| **P99 Protection** | 12ms under 5x burst load (Managed) |
---

## Workspace Layout

```text
.
├── shot-limit/   # Core atomic strategies (Zero-dependency)
└── tower-shot/   # Tower middleware & Managed layers
```

## Getting Started

To use the full managed stack in an Axum project, add `tower-shot` to your dependencies and configure a managed layer:

```rust
use tower_shot::ManagedRateLimitLayer;
use shot_limit::TokenBucket;
use std::sync::Arc;
use std::time::Duration;
use std::num::NonZeroUsize;

// 1. Define your strategy (from shot-limit)
let limit = NonZeroUsize::new(1000).unwrap();
let period = Duration::from_secs(1);
let strategy = Arc::new(TokenBucket::new(limit, 1000, period));

// 2. Define your Latency SLA (Max wait time)
let max_wait = Duration::from_millis(500);

// 3. Create the Managed Layer for your Axum Router
let layer = ManagedRateLimitLayer::new(strategy, max_wait);
```

## Performance & Scaling

Because the underlying `shot-limit` strategies are lock-free, this workspace is designed to scale linearly with your hardware. Whether you are running on a single-core edge function or a 128-core bare-metal server, **Shot** ensures that rate limiting is never the bottleneck in your stack.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
