use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use shot_limit::Strategy;
use tower::Layer;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::service_fn;

use shot_limit::{FixedWindow, SlidingWindow, TokenBucket};
use tower_shot::{BufferedRateLimitLayer, ManagedRateLimitLayer, RateLimitLayer};

// No-op service to isolate middleware overhead
async fn noop(_req: ()) -> Result<&'static str, tower::BoxError> {
    Ok("ok")
}

/*
fn bench_limiters(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("Rate Limiters");

    // Use a massive limit for overhead benchmarking so we don't trigger the "wait"
    let limit = 1_000_000;
    let period = Duration::from_secs(1);
    let nz_limit = NonZeroUsize::new(limit).unwrap();

    let bucket_strat = Arc::new(TokenBucket::new(nz_limit, limit, period));

    group.bench_function("shot_managed_token_bucket_overhead", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = ManagedRateLimitLayer::new(bucket_strat.clone(), period);
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                // This should now be Ok() every time because capacity is 1M
                let _ = svc.ready().await.unwrap().call(()).await.unwrap();
            },
        );
    });

    group.finish();
}
*/

fn bench_limiters(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("Rate Limiters");

    let limit = 1_000_000_000;
    let period = Duration::from_secs(1);
    let nz_limit = limit.try_into().unwrap();

    // Shared Strategies
    let fixed_strat = Arc::new(FixedWindow::new(nz_limit, period));
    let sliding_strat = Arc::new(SlidingWindow::new(nz_limit, period));
    // TokenBucket: 10k capacity, refilling 1k every interval
    let bucket_strat = Arc::new(TokenBucket::new(nz_limit, 1000, period));

    // --- MANAGED BENCHMARKS ---
    // Managed layers use LoadShedding. If the limit is hit, they return an Error.
    // We explicitly do NOT unwrap the final .call() result to prevent panics.

    group.bench_function("shot_managed_fixed_window", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = ManagedRateLimitLayer::new(fixed_strat.clone(), period);
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                let _ = svc.ready().await.unwrap().call(()).await;
            },
        );
    });

    group.bench_function("shot_managed_sliding_window", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = ManagedRateLimitLayer::new(sliding_strat.clone(), period);
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                let _ = svc.ready().await.unwrap().call(()).await;
            },
        );
    });

    group.bench_function("shot_managed_token_bucket", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = ManagedRateLimitLayer::new(bucket_strat.clone(), period);
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                let _ = svc.ready().await.unwrap().call(()).await;
            },
        );
    });

    // --- BUFFERED BENCHMARKS ---
    // Buffered layers queue requests, so we expect Ok("ok") results.

    group.bench_function("shot_buffered_token_bucket", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = BufferedRateLimitLayer::new(bucket_strat.clone(), limit);
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                svc.ready().await.unwrap().call(()).await.unwrap();
            },
        );
    });

    // --- RAW STRATEGY COMPARISONS ---

    group.bench_function("token_bucket_raw_layer", |b| {
        let base_svc = RateLimitLayer::new(bucket_strat.clone()).layer(service_fn(noop));
        b.to_async(&rt).iter_with_setup(
            || base_svc.clone(),
            |mut svc| async move {
                let _ = svc.ready().await.unwrap().call(()).await;
            },
        );
    });

    group.finish();
}

fn bench_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("Raw Strategies (Sync)");

    let limit = 10_000;
    let period = Duration::from_secs(1);
    let nz_limit = limit.try_into().unwrap();

    let fixed = FixedWindow::new(nz_limit, period);
    let sliding = SlidingWindow::new(nz_limit, period);
    let bucket = TokenBucket::new(nz_limit, 1000, period);

    group.bench_function("fixed_window_atomic_only", |b| {
        b.iter(|| fixed.process());
    });

    group.bench_function("sliding_window_atomic_only", |b| {
        b.iter(|| sliding.process());
    });

    group.bench_function("token_bucket_atomic_only", |b| {
        b.iter(|| bucket.process());
    });

    group.finish();
}

criterion_group!(benches, bench_limiters, bench_strategies);
criterion_main!(benches);
