use std::sync::Arc;
use std::time::Duration;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use tower::Layer;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::service_fn;

use shot_limit::FixedWindow;
use shot_limit::SlidingWindow;
use shot_limit::TokenBucket;
use tower_shot::ManagedRateLimitLayer;
use tower_shot::RateLimitLayer;

// No-op service to isolate middleware overhead
async fn noop(_req: ()) -> Result<&'static str, tower::BoxError> {
    Ok("ok")
}

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

    // --- RAW STRATEGY COMPARISONS ---

    group.bench_function("shot_raw_fixed_window", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = RateLimitLayer::new(fixed_strat.clone());
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                let _ = svc.ready().await.unwrap().call(()).await;
            },
        );
    });

    group.bench_function("shot_raw_sliding_window", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let layer = RateLimitLayer::new(sliding_strat.clone());
                ServiceBuilder::new().layer(layer).service(service_fn(noop))
            },
            |mut svc| async move {
                let _ = svc.ready().await.unwrap().call(()).await;
            },
        );
    });

    group.bench_function("shot_raw_token_bucket", |b| {
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

criterion_group!(benches, bench_limiters);
criterion_main!(benches);
