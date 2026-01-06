# py-shot-limit

High-performance rate-limiting strategies for Python, powered by the Rust `shot-limit` core engine.

This crate provides Python bindings for `shot-limit`, allowing you to use the same lock-free, atomic-based rate-limiting algorithms directly in Python applications. It brings the performance and scalability of Rust to your Python services.

## Features

- **Blazing Fast**: Directly wraps the native Rust `shot-limit` strategies, offering nanosecond-level performance.
- **Thread-Safe**: The underlying atomics make these strategies safe to share across threads without locks.
- **Multiple Strategies**: Includes Token Bucket, Fixed Window, Sliding Window, and GCRA.

## Setup

To set up the Python environment and run tests:

1.  **Build the Rust project with Python bindings**:
    ```bash
    maturin develop --release
    ```
    This command will build the Rust code and install the `py_shot_limit` Python package into your active Python environment.

2.  **Activate the Python virtual environment** (if you're using one, which is recommended):
    ```bash
    source .venv/bin/activate
    ```
    If you don't have a virtual environment, you can create one using `python -m venv .venv` and then activate it.

3.  **Run the Python tests**:
    ```bash
    pytest
    ```

OR (if you just want to run tests)

1.  Just run `cargo test` which will do this for you.

## Example Usage (Python)

```python
import py_shot_limit as shot_limit
import time

# TokenBucket Example
bucket = shot_limit.TokenBucket(capacity=2, increment=1, period_secs=5)
print(f"TokenBucket Call 1: {'Allowed' if bucket.process() else 'Denied'}")
time.sleep(6) # Wait for refill
print(f"TokenBucket Call 2: {'Allowed' if bucket.process() else 'Denied'}")

# FixedWindow Example
fw = shot_limit.FixedWindow(capacity=2, period_secs=1)
print(f"FixedWindow Call 1: {'Allowed' if fw.process() else 'Denied'}")
print(f"FixedWindow Call 2: {'Allowed' if fw.process() else 'Denied'}") # Allowed
print(f"FixedWindow Call 3: {'Allowed' if fw.process() else 'Denied'}") # Denied
```

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or [MIT license](../../LICENSE-MIT) at your option.
