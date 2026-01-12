//! Benchmarks for `tower-shot` rate-limiting layers.
//!
//! These benchmarks are designed to evaluate the performance and behavior of different
//! rate-limiting strategies under both low and high contention.
//!
//! ### Scenarios:
//! - **Standard Layers**: Benchmarks the raw overhead of the rate-limiting logic (e.g., GCRA atomics)
//!   without additional management logic.
//! - **Managed Latency (Shed-First)**: Benchmarks the "Load Shed" behavior where requests exceeding
//!   capacity are immediately rejected with an error.
//! - **Managed Throughput (Retry-Based)**: Benchmarks the "Backpressure" behavior where requests
//!   exceeding capacity are queued and retried until they succeed or timeout.
//!
//! ### Realistic Contention:
//! The parameters (Capacity < Burst Size) are specifically tuned to force the limiters into a
//! saturated state. This ensures we are benchmarking the actual enforcement logic (retries,
//! load-shedding, and CAS contention) rather than just the no-op overhead of a free limiter.

use std::hint::black_box;
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use criterion::BenchmarkGroup;
use criterion::Criterion;
use criterion::Throughput;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::measurement::WallTime;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use governor::Quota;
use governor::RateLimiter;
use governor::clock::Clock;
use governor::clock::DefaultClock;
use governor::clock::QuantaClock;
use governor::middleware::NoOpMiddleware;
use governor::state::InMemoryState;
use governor::state::NotKeyed;
use http::Request;
use http::Response;
use shot_limit::FixedWindow;
use shot_limit::Gcra;
use shot_limit::SlidingWindow;
use shot_limit::TokenBucket;
use tokio::time::Sleep;
use tokio::time::sleep;
use tower::BoxError;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::limit::RateLimitLayer as TowerNativeRateLimit;
use tower::service_fn;
use tower::util::BoxCloneSyncService;
use tower_shot::RateLimitLayer;
use tower_shot::make_latency_svc;
use tower_shot::make_timeout_svc;

// --- HELPERS & TYPES ---

type BenchService = BoxCloneSyncService<Request<String>, Response<String>, BoxError>;

async fn noop_handler(_req: Request<String>) -> Result<Response<String>, BoxError> {
    Ok(Response::new("ok".to_string()))
}

async fn run_one_req(mut svc: BenchService) -> Result<Response<String>, BoxError> {
    let req = Request::builder().body("test".to_string()).unwrap();
    match svc.ready().await {
        Ok(_) => svc.call(req).await,
        Err(e) => Err(e),
    }
}

/// Generic runner for single-call overhead benchmarks
fn bench_group(
    group: &mut BenchmarkGroup<WallTime>,
    rt: &tokio::runtime::Runtime,
    id: &str,
    svc: BenchService,
) {
    group.bench_function(id, |b| {
        b.to_async(rt).iter(|| {
            let svc = svc.clone();
            async move {
                let res = run_one_req(svc).await;
                black_box(res)
            }
        });
    });
}

/// Generic runner for single-call throughput benchmarks (counts successes)
fn bench_throughput_group(
    group: &mut BenchmarkGroup<WallTime>,
    rt: &tokio::runtime::Runtime,
    id: &str,
    svc: BenchService,
) {
    group.bench_function(id, |b| {
        b.to_async(rt).iter_custom(|iters| {
            let svc = svc.clone();
            async move {
                let mut total_successes = 0;
                let start = std::time::Instant::now();

                for _ in 0..iters {
                    let res = run_one_req(svc.clone()).await;
                    if res.is_ok() {
                        total_successes += 1;
                    }
                    let _ = black_box(res);
                }

                let elapsed = start.elapsed();

                if total_successes == 0 {
                    return Duration::from_secs(100_000);
                }

                // Adjust duration to make Criterion report "Successes per second"
                // when we set throughput to Elements(1).
                // Rate = iters / AdjustedTime
                // Target = TotalSuccesses / RealTime
                // iters / AdjustedTime = TotalSuccesses / RealTime
                // AdjustedTime = iters * RealTime / TotalSuccesses

                elapsed.mul_f64((iters as f64) / (total_successes as f64))
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
            let svc = svc.clone();
            async move {
                let mut futures = FuturesUnordered::new();
                for _ in 0..burst_size {
                    futures.push(run_one_req(svc.clone()));
                }
                while let Some(res) = futures.next().await {
                    let _ = black_box(res);
                }
            }
        });
    });
}

/// Generic runner for burst throughput benchmarks (counts successes)
fn bench_throughput_burst(
    group: &mut BenchmarkGroup<WallTime>,
    rt: &tokio::runtime::Runtime,
    id: &str,
    svc: BenchService,
    burst_size: usize,
) {
    group.bench_function(id, |b| {
        b.to_async(rt).iter_custom(|iters| {
            let svc = svc.clone();
            async move {
                let mut total_successes = 0;
                let start = std::time::Instant::now();

                for _ in 0..iters {
                    let s = svc.clone();
                    let mut futures = FuturesUnordered::new();
                    // Initial fill only - NO RETRIES
                    for _ in 0..burst_size {
                        futures.push(run_one_req(s.clone()));
                    }

                    while let Some(res) = futures.next().await {
                        if res.is_ok() {
                            total_successes += 1;
                        }
                    }
                }

                let elapsed = start.elapsed();

                if total_successes == 0 {
                    return Duration::from_secs(100_000);
                }

                // Adjust duration to make Criterion report "Successes per second"
                // when we set throughput to Elements(1).
                // Rate = iters / AdjustedTime
                // Target = TotalSuccesses / RealTime
                // iters / AdjustedTime = TotalSuccesses / RealTime
                // AdjustedTime = iters * RealTime / TotalSuccesses

                elapsed.mul_f64((iters as f64) / (total_successes as f64))
            }
        });
    });
}

// --- GOVERNOR ADAPTER ---

struct GovernorService<S> {
    inner: S,
    limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
    sleep: Option<Pin<Box<Sleep>>>,
    clock: QuantaClock,
}

impl<S> Clone for GovernorService<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limiter: self.limiter.clone(),
            sleep: None,
            clock: self.clock.clone(),
        }
    }
}

impl<S, Req> Service<Req> for GovernorService<S>
where
    S: Service<Req, Error = BoxError>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // 1. Check sleep
        if let Some(ref mut fut) = self.sleep {
            match fut.as_mut().poll(cx) {
                Poll::Ready(_) => self.sleep = None,
                Poll::Pending => return Poll::Pending,
            }
        }

        // 2. Check inner
        match self.inner.poll_ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }

        // 3. Check governor
        match self.limiter.check() {
            Ok(_) => Poll::Ready(Ok(())),
            Err(negative) => {
                let wait: Duration = negative.wait_time_from(self.clock.now());
                let mut sleep_fut = Box::pin(sleep(wait));
                match sleep_fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        self.sleep = Some(sleep_fut);
                        Poll::Pending
                    }
                    Poll::Ready(_) => {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.inner.call(req)
    }
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

    // Configuration for "Realistic" Rate Limiting
    // Rate: 10,000 requests per second.
    // Capacity: 100 requests.
    // Period: 10ms.
    // Burst: 1000 requests.
    //
    // Analysis:
    // - Capacity (100) << Burst (1000).
    // - The burst will effectively exhaust the limiter immediately.
    // - This forces the Latency Service to reject and the Throughput Service to retry.
    let capacity_u = 100;
    // increment_u is technically unused by GCRA, but kept for reference or if TokenBucket is re-enabled.
    let increment_u = 100;
    let capacity = NonZeroUsize::new(capacity_u).unwrap();
    let increment = NonZeroUsize::new(increment_u).unwrap();

    let period = Duration::from_millis(10);
    let timeout = Duration::from_millis(500); // Allow enough time for retries to potentially succeed
    let burst_size = 1000;

    // 1. Setup Shared Strategies
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let gcra = Arc::new(Gcra::new(capacity, period));

    // Calculate requests per second for Governor to match GCRA settings
    // 100 req / 10ms = 10,000 req / s
    let requests_per_second = (capacity_u as u64 * 1000) / period.as_millis() as u64;
    let governor = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(requests_per_second as u32).unwrap(),
    )));

    // 2. Define Scenarios (ID, Service)
    // This makes adding new strategies or layers trivial.
    let scenarios: Vec<(&str, BenchService)> = vec![
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
            "shot_standard_gcra",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(RateLimitLayer::new(gcra.clone()))
                    .service(service_fn(noop_handler)),
            ),
        ),
        ("standard_governor", {
            BoxCloneSyncService::new(GovernorService {
                inner: service_fn(noop_handler),
                limiter: governor.clone(),
                sleep: None,
                clock: DefaultClock::default(),
            })
        }),
        (
            "standard_tower_native",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .buffer(capacity_u)
                    .layer(TowerNativeRateLimit::new(capacity_u as u64, period))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_throughput_fixed",
            make_timeout_svc(fixed.clone(), timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_latency_fixed",
            make_latency_svc(fixed, timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_throughput_sliding",
            make_timeout_svc(sliding.clone(), timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_latency_sliding",
            make_latency_svc(sliding, timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_throughput_bucket",
            make_timeout_svc(bucket.clone(), timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_latency_bucket",
            make_latency_svc(bucket, timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_throughput_gcra",
            make_timeout_svc(gcra.clone(), timeout, service_fn(noop_handler)),
        ),
        (
            "shot_managed_latency_gcra",
            make_latency_svc(gcra.clone(), timeout, service_fn(noop_handler)),
        ),
        ("managed_governor", {
            BoxCloneSyncService::new(ServiceBuilder::new().timeout(timeout).load_shed().service(
                GovernorService {
                    inner: service_fn(noop_handler),
                    limiter: governor,
                    sleep: None,
                    clock: DefaultClock::default(),
                },
            ))
        }),
        (
            "managed_tower_native",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .timeout(timeout)
                    .load_shed()
                    .buffer(capacity_u)
                    .layer(TowerNativeRateLimit::new(capacity_u as u64, period))
                    .service(service_fn(noop_handler)),
            ),
        ),
    ];

    // 3. Run Latency Group
    let mut latency_group = c.benchmark_group("Middleware Latency");
    // Pass 1: Standard/Latency (no throughput calc)
    for (id, svc) in &scenarios {
        if !id.contains("managed_throughput") {
            bench_group(&mut latency_group, &rt, id, svc.clone());
        }
    }
    latency_group.finish();

    // Pass 2: Throughput (throughput calc)
    let mut throughput_group = c.benchmark_group("Middleware Throughput");
    throughput_group.throughput(Throughput::Elements(1));
    for (id, svc) in &scenarios {
        if id.contains("managed_throughput")
            || id.contains("standard")
            || id.contains("governor")
            || id.contains("tower")
        {
            bench_throughput_group(&mut throughput_group, &rt, id, svc.clone());
        }
    }
    throughput_group.finish();

    // 4. Run Contention Group
    let mut contention_latency_group = c.benchmark_group("High Contention (1000 Tasks) Latency");
    // Pass 1: Standard/Latency
    for (id, svc) in &scenarios {
        if !id.contains("managed_throughput") {
            bench_burst(
                &mut contention_latency_group,
                &rt,
                id,
                svc.clone(),
                burst_size,
            );
        }
    }
    contention_latency_group.finish();

    // Pass 2: Throughput
    let mut contention_throughput_group =
        c.benchmark_group("High Contention (1000 Tasks) Throughput");
    contention_throughput_group.throughput(Throughput::Elements(1));
    for (id, svc) in &scenarios {
        if id.contains("managed_throughput")
            || id.contains("governor")
            || id.contains("tower")
            || id.contains("standard")
        {
            bench_throughput_burst(
                &mut contention_throughput_group,
                &rt,
                id,
                svc.clone(),
                burst_size,
            );
        }
    }
    contention_throughput_group.finish();
}

criterion_group!(benches, bench_all_scenarios);
criterion_main!(benches);
