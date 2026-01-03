# Shot ✨

A high-performance, atomic-based rate limiting ecosystem for Rust services. 

## The Ecosystem

This project is split into two specialized crates:

* **`shot-limit`**: The core engine. It contains the logic for rate-limiting strategies (Token Bucket, Fixed Window, Sliding Window). It is zero-cost, generic, and can be used independently of any networking framework.
* **`tower-shot`**: The integration layer. It provides `tower::Layer` and `tower::Service` implementations that wrap `shot-limit` strategies, making them easy to use with `axum`, `tonic`, or any other Tower-compatible stack.

## Why Choose Shot?

I was motivated to see if there was a better way to perform Rate Limiting than the existing `tower` implementation. I wanted configurable, by strategy, rate limiting and I wanted to eliminate the use of Buffering.

From these core goals, I've ended up with two `tower` Layers:
 - `ManagedRateLimitLayer` - opinionated and providing timeouts and load shedding. 
 - `RateLimitLayer` - drop in replacement for the existing `tower` Rate Limit.

## Performance & Scaling

Because the underlying `shot-limit` strategies are lock-free, these crates are designed to scale linearly with your hardware. Whether you are running on a single-core edge function or a 128-core bare-metal server, **Shot** ensures that rate limiting is never the bottleneck in your stack.

The performance of the various strategies and layers is demonstrated in the included benchmarks. The results are good on my Mac M1 laptop, but I encourage anyone using this to verify the results are good in their environment.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
