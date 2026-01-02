# shot-limit

Core atomic rate-limiting strategies for high-throughput Rust services. This crate provides the lock-free logic used by the `tower-shot` middleware ecosystem.

## Features

- **Atomic-First Design**: Uses `std::sync::atomic` primitives for state management.
- **Lock-Free Hot Path**: No `Mutex` or `RwLock` contention, even under extreme thread pressure.
- **Zero Dependencies**: Focused, lightweight, and fast.
- **Lazy Evaluation**: Refills and window rotations are calculated at the time of the request, eliminating the need for background worker threads.

## Performance

Built for extreme scale on modern hardware. The following benchmarks were recorded on an **Apple M1 (8-core)** using the included `criterion` suite:

| Strategy | Single-Threaded | 8-Thread Parallel (Effective) |
| :--- | :--- | :--- |
| **Token Bucket** | 24.3 ns | **3.3 ns** |
| **Fixed Window** | 26.2 ns | **3.8 ns** |
| **Sliding Window** | 35.5 ns | **5.1 ns** |

*Note: Total throughput at 8 threads exceeds **300 million operations per second** for the Token Bucket strategy.*



## Usage

Each strategy implements the `Strategy` trait, which provides a non-blocking `process()` method.

```rust
use shot_limit::TokenBucket;
use shot_limit::Strategy;
use std::time::Duration;
use std::num::NonZeroUsize;

let limit = NonZeroUsize::new(100).unwrap();
let period = Duration::from_secs(60);

// Initialize a Token Bucket with 100 tokens, refilling every minute
let bucket = TokenBucket::new(limit, 100, period);

match bucket.process() {
    Ok(_) => {
        // Request allowed
    }
    Err(_) => {
        // Rate limit exceeded
    }
}
```

## Strategies

### Token Bucket
The most flexible strategy. It allows for a burst of requests up to a defined capacity and replenishes tokens at a steady rate. Best for smoothing out traffic spikes and providing a consistent experience.

### Fixed Window
Divides time into fixed slots (e.g., 1-minute windows). Simple and extremely low overhead, but can allow twice the rate limit at window boundaries. Use this when performance is the absolute priority and slight boundary bursts are acceptable.

### Sliding Window
A weighted algorithm that accounts for the previous window's traffic to smooth out boundary bursts. Provides significantly more accuracy than Fixed Window with only a minor performance trade-off for the additional floating-point calculations.



## Development

Run the benchmark suite to verify performance on your specific architecture. On high-performance ARM or x86 chips, you should see linear scaling across multiple threads.

```bash
cargo bench
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
