# Shot ✨

A high-performance, atomic-based rate limiting ecosystem for Rust services. 

## The Ecosystem

This project is split into three specialized crates:

* **`shot-limit`**: The core engine. It contains the logic for rate-limiting strategies (Generic Cell Rate Algorithm (GCRA), Token Bucket, Fixed Window, Sliding Window). It is zero-cost, generic, and can be used independently of any networking framework.
* **`tower-shot`**: The integration layer for `async` Rust. It provides `tower::Layer` and `tower::Service` implementations that wrap `shot-limit` strategies, making them easy to use with `axum`, `tonic`, or any other Tower-compatible stack.
* **`py-shot-limit`**: Python bindings for the core engine, making the high-performance strategies available in Python.

## Why Choose Shot?

**Shot** was created to provide a flexible rate-limiting solution for Rust applications, focusing on configurability, performance, and eliminating buffer bloat.

It offers several `tower` layers and helpers to suit different needs:
 - `make_timeout_svc`: Maximizes throughput by retrying requests within a timeout.
 - `make_latency_svc`: Provides aggressive load shedding to protect service latency.
 - `RateLimitLayer`: A drop-in replacement for the existing `tower` rate-limiting layer, designed for maximum performance.

## Performance & Scaling

Because the underlying `shot-limit` strategies are lock-free, these crates are designed to scale linearly with your hardware. Whether you are running on a single-core edge function or a 128-core bare-metal server, **Shot** ensures that rate limiting is never the bottleneck in your stack.

The performance of the various strategies and layers is demonstrated in the included benchmarks. The results are good on my Mac M1 laptop, but I encourage anyone using this to verify the results are good in their environment.

## Performance Summary

### `shot-limit` (Core Strategies)
The core `shot-limit` strategies demonstrate extremely low overhead, with single-threaded operations typically in the low nanoseconds. For instance:
- **Fixed Window:** ~2.58 ns (single-threaded)
- **GCRA:** ~2.32 ns (single-threaded)

These strategies scale efficiently, maintaining high performance under multi-threaded loads (e.g., Fixed Window at ~0.42 ns per operation with 8 threads). For full details, see [`shot-limit/README.md`](./shot-limit/README.md).

### `tower-shot` (Middleware)
`tower-shot` introduces minimal overhead while providing robust rate-limiting middleware.
- **Standard `tower-shot` Layer:** ~99.7 µs latency in saturated tests (precisely matching the 10k req/s target).
- **Managed `tower-shot` Layer:** ~252 ns latency (Load Shedding), offering a massive speedup compared to `tower`'s buffered approach (~117.5 µs).

When compared against `governor` in a fully Tower-compliant "Wait-until-Ready" configuration, `tower-shot` performs **on par** (~99.7 µs vs ~99.7 µs), while maintaining its architectural advantage over the native Tower implementation.

Under high contention (1,000 concurrent tasks), `tower-shot` layers maintain excellent performance, with the Managed layer processing requests in approximately 312 µs, compared to ~14.3 ms for the native Tower implementation. For full details, see [`tower-shot/README.md`](./tower-shot/README.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
