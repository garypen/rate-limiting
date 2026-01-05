# Shot ✨

A high-performance, atomic-based rate limiting ecosystem for Rust services. 

## The Ecosystem

This project is split into three specialized crates:

* **`shot-limit`**: The core engine. It contains the logic for rate-limiting strategies (Token Bucket, Fixed Window, Sliding Window). It is zero-cost, generic, and can be used independently of any networking framework.
* **`tower-shot`**: The integration layer for `async` Rust. It provides `tower::Layer` and `tower::Service` implementations that wrap `shot-limit` strategies, making them easy to use with `axum`, `tonic`, or any other Tower-compatible stack.
* **`py-shot-limit`**: Python bindings for the core engine, making the high-performance strategies available in Python.

## Why Choose Shot?

**Shot** was created to provide a better rate-limiting solution for Rust applications, focusing on configurability, performance, and eliminating buffer bloat.

It offers two `tower` layers to suit different needs:
 - `ManagedRateLimitLayer`: An opinionated layer that provides timeouts and load shedding, ideal for protecting public-facing services.
 - `RateLimitLayer`: A drop-in replacement for the existing `tower` rate-limiting layer, designed for maximum performance.

## Performance & Scaling

Because the underlying `shot-limit` strategies are lock-free, these crates are designed to scale linearly with your hardware. Whether you are running on a single-core edge function or a 128-core bare-metal server, **Shot** ensures that rate limiting is never the bottleneck in your stack.

The performance of the various strategies and layers is demonstrated in the included benchmarks. The results are good on my Mac M1 laptop, but I encourage anyone using this to verify the results are good in their environment.

## Performance Summary

### `shot-limit` (Core Strategies)
The core `shot-limit` strategies demonstrate extremely low overhead, with single-threaded operations typically in the low nanoseconds. For instance:
- **Fixed Window:** ~1.87 ns (single-threaded)
- **GCRA:** ~2.01 ns (single-threaded)

These strategies scale efficiently, maintaining high performance under multi-threaded loads (e.g., Fixed Window at ~0.33 ns per operation with 8 threads). For full details, see [`shot-limit/README.md`](./shot-limit/README.md).

### `tower-shot` (Middleware)
`tower-shot` introduces minimal overhead while providing robust rate-limiting middleware.
- **Standard `tower-shot` Layer:** ~104 ns latency, offering a significant speedup (59x faster) compared to `tower::limit::RateLimit`.
- **Managed `tower-shot` Layer:** ~211 ns latency, providing advanced features like timeouts and load shedding while remaining highly performant (29x faster than `tower::limit::RateLimit`).

Under high contention (1,000 concurrent tasks), `tower-shot` layers maintain excellent performance, with the Standard layer processing requests in approximately 180 µs. For full details, see [`tower-shot/README.md`](./tower-shot/README.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
