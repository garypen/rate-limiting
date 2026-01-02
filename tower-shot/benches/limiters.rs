use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use criterion::BenchmarkGroup;
use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::measurement::WallTime;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use governor::Quota;
use governor::RateLimiter;
use http::Request;
use http::Response;
use shot_limit::FixedWindow;
use shot_limit::SlidingWindow;
use shot_limit::TokenBucket;
use tower::BoxError;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::limit::RateLimitLayer as TowerNativeRateLimit;
use tower::service_fn;
use tower::util::BoxCloneSyncService;
use tower_shot::ManagedRateLimitLayer;
use tower_shot::RateLimitLayer;

// --- HELPERS & TYPES ---

type BenchService = BoxCloneSyncService<Request<String>, Response<String>, BoxError>;

async fn noop_handler(_req: Request<String>) -> Result<Response<String>, BoxError> {
    Ok(Response::new("ok".to_string()))
}

/// Generic runner for single-call overhead benchmarks
fn bench_overhead(
    group: &mut BenchmarkGroup<WallTime>,
    rt: &tokio::runtime::Runtime,
    id: &str,
    svc: BenchService,
) {
    group.bench_function(id, |b| {
        b.to_async(rt).iter(|| {
            let mut s = svc.clone();
            async move {
                let req = Request::builder().body("test".to_string()).unwrap();
                let res = s.ready().await.unwrap().call(req).await;
                black_box(res)
            }
        });
    });
}

/// Generic runner for burst/contention benchmarks
fn bench_burst(
    group: &mut BenchmarkGroup<WallTime>,
    rt: &tokio::runtime::Runtime,
    id: &str,
    svc: BenchService,
    burst_size: usize,
) {
    group.bench_function(id, |b| {
        b.to_async(rt).iter(|| {
            let s = svc.clone();
            async move {
                let mut futures = FuturesUnordered::new();
                for _ in 0..burst_size {
                    let mut local_svc = s.clone();
                    futures.push(async move {
                        let req = Request::builder().body("test".to_string()).unwrap();
                        local_svc.ready().await.unwrap().call(req).await
                    });
                }
                while let Some(res) = futures.next().await {
                    let _ = black_box(res);
                }
            }
        });
    });
}

// --- MAIN BENCHMARK ---

fn bench_all_scenarios(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();
    // ENTER the runtime context so Tower's RateLimit can find the reactor
    let _guard = rt.enter();

    let limit_u = 100_000;
    let limit = NonZeroUsize::new(limit_u).unwrap();
    let period = Duration::from_millis(1);
    let burst_size = 1000;

    // 1. Setup Shared Strategies
    // Use a massive burst limit so we measure the overhead of the math,
    // not the time spent sleeping.
    let bucket = Arc::new(TokenBucket::new(
        NonZeroUsize::new(100_000_000).unwrap(), // Huge capacity
        NonZeroUsize::new(100_000).unwrap(),
        period,
    ));
    // let bucket = Arc::new(TokenBucket::new(limit, 100, period));
    let fixed = Arc::new(FixedWindow::new(limit, period));
    let sliding = Arc::new(SlidingWindow::new(limit, period));
    let governor = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(limit_u.try_into().unwrap()).unwrap(),
    )));

    // 2. Define Scenarios (ID, Service)
    // This makes adding new strategies or layers trivial.
    let scenarios: Vec<(&str, BenchService)> = vec![
        (
            "tower_native",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .buffer(1_024)
                    .layer(TowerNativeRateLimit::new(limit_u as u64, period))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_standard_fixed",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(RateLimitLayer::new(fixed.clone()))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_standard_sliding",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(RateLimitLayer::new(sliding.clone()))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_standard_bucket",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(RateLimitLayer::new(bucket.clone()))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_fixed",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedRateLimitLayer::new(
                        fixed.clone(),
                        Duration::from_millis(100),
                    ))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_sliding",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedRateLimitLayer::new(
                        sliding.clone(),
                        Duration::from_millis(100),
                    ))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_bucket",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedRateLimitLayer::new(
                        bucket.clone(),
                        Duration::from_millis(100),
                    ))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "governor",
            BoxCloneSyncService::new(service_fn(move |req| {
                let limiter = governor.clone();
                async move {
                    if limiter.check().is_ok() {
                        noop_handler(req).await
                    } else {
                        Err("Rate limited".into())
                    }
                }
            })),
        ),
    ];

    // 3. Run Overhead Group
    let mut overhead_group = c.benchmark_group("Middleware Overhead");
    for (id, svc) in &scenarios {
        bench_overhead(&mut overhead_group, &rt, id, svc.clone());
    }
    overhead_group.finish();

    // 4. Run Contention Group
    let mut contention_group = c.benchmark_group("High Contention (1000 Tasks)");
    for (id, svc) in &scenarios {
        bench_burst(&mut contention_group, &rt, id, svc.clone(), burst_size);
    }
    contention_group.finish();
}

criterion_group!(benches, bench_all_scenarios);
criterion_main!(benches);
