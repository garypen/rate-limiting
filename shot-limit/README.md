# shot-limit

Core atomic rate-limiting strategies for high-throughput Rust services. This crate provides the lock-free logic used by the `tower-shot` middleware ecosystem.

## Features

- **High-Performance Primitives**: Uses `std::sync::atomic` and the `quanta` TSC-based clock for state management.
- **Lock-Free Hot Path**: No `Mutex` or `RwLock` contention.
- **Lazy Evaluation**: Refills and window rotations are calculated at the time of the request, eliminating the need for background worker threads.

## Performance

Built for extreme scale on modern hardware. The following benchmarks were recorded on an **Apple M1 (8-core)** using the included `criterion` suite:

| Strategy | Single-Threaded | 8-Thread Parallel |
|:---|:---:|:---:|
| **Token Bucket** | 3.11 ns | 0.71 ns |
| **Fixed Window** | 2.58 ns | 0.42 ns |
| **Sliding Window** | 4.94 ns | 1.09 ns |
| **GCRA** | 2.32 ns | 0.41 ns |

*Note: Total throughput at 8 threads exceeds **2.3 billion operations per second** for the Fixed Window strategy.*



## Usage

Each strategy implements the `Strategy` trait, which provides a `process()` method.

```rust
use shot_limit::TokenBucket;
use shot_limit::Strategy;
use std::time::Duration;
use std::num::NonZeroUsize;

let capacity = NonZeroUsize::new(100).unwrap();
let increment = NonZeroUsize::new(100).unwrap();
let period = Duration::from_secs(60);

// Initialize a Token Bucket with a capacity of 100 tokens, refilling 100 tokens every minute
let bucket = TokenBucket::new(capacity, increment, period);

if bucket.process().is_continue() {
    // Request allowed
} else {
    // Rate limit exceeded
}
```

## Strategies

### Token Bucket
The most flexible strategy. It allows for a burst of requests up to a defined capacity and replenishes tokens at a steady rate. Best for smoothing out traffic spikes and providing a consistent experience.

### Fixed Window
Divides time into fixed slots (e.g., 1-minute windows). Simple and extremely low overhead, but can allow twice the rate limit at window boundaries. Use this when performance is the absolute priority and slight boundary bursts are acceptable.

### Sliding Window
A weighted algorithm that accounts for the previous window's traffic to smooth out boundary bursts. Provides significantly more accuracy than Fixed Window with only a minor performance trade-off for the additional floating-point calculations.

### GCRA (Generic Cell Rate Algorithm)
A highly efficient and mathematically elegant algorithm that provides a strict, predictable rate limit without the burstiness of a token bucket. It's an excellent choice when you need to enforce a smooth, even flow of traffic.

## Development

Run the benchmark suite to verify performance on your specific architecture. On high-performance ARM or x86 chips, you should see linear scaling across multiple threads.

```bash
cargo bench
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
