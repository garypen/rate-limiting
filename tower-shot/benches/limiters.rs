use std::hint::black_box;
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::sync::Arc;
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
use http::Request;
use http::Response;
use shot_limit::FixedWindow;
use shot_limit::Gcra;
use shot_limit::SlidingWindow;
use shot_limit::TokenBucket;
use tower::BoxError;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::limit::RateLimitLayer as TowerNativeRateLimit;
use tower::service_fn;
use tower::util::BoxCloneSyncService;
use tower_shot::ManagedLatencyLayer;
use tower_shot::ManagedThroughputLayer;
use tower_shot::RateLimitLayer;

// --- HELPERS & TYPES ---

type BenchService = BoxCloneSyncService<Request<String>, Response<String>, BoxError>;

async fn noop_handler(_req: Request<String>) -> Result<Response<String>, BoxError> {
    Ok(Response::new("ok".to_string()))
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

// --- GOVERNOR ADAPTER ---

use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::InMemoryState;
use governor::state::NotKeyed;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::time::Sleep;
use tokio::time::sleep;

struct GovernorService<S> {
    inner: S,
    limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
    sleep: Option<Pin<Box<Sleep>>>,
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
                let wait: Duration = negative.wait_time_from(DefaultClock::default().now());
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

    let capacity_u = 100_000;
    let increment_u = 100_000;
    let capacity = NonZeroUsize::new(capacity_u).unwrap();
    let increment = NonZeroUsize::new(increment_u).unwrap();
    let period = Duration::from_millis(1);
    let timeout = Duration::from_millis(100);
    let burst_size = 1000;

    // 1. Setup Shared Strategies
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let gcra = Arc::new(Gcra::new(capacity, period));
    let governor = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(increment_u.try_into().unwrap()).unwrap(),
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
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedThroughputLayer::new(fixed.clone(), timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_latency_fixed",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedLatencyLayer::new(fixed, timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_throughput_sliding",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedThroughputLayer::new(sliding.clone(), timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_latency_sliding",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedLatencyLayer::new(sliding, timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_throughput_bucket",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedThroughputLayer::new(bucket.clone(), timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_latency_bucket",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedLatencyLayer::new(bucket, timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_throughput_gcra",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedThroughputLayer::new(gcra.clone(), timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        (
            "shot_managed_latency_gcra",
            BoxCloneSyncService::new(
                ServiceBuilder::new()
                    .layer(ManagedLatencyLayer::new(gcra, timeout))
                    .service(service_fn(noop_handler)),
            ),
        ),
        ("managed_governor", {
            BoxCloneSyncService::new(ServiceBuilder::new().timeout(timeout).load_shed().service(
                service_fn(move |req| {
                    let limiter = governor.clone();
                    async move {
                        if limiter.check().is_ok() {
                            noop_handler(req).await
                        } else {
                            Err("Rate limited".into())
                        }
                    }
                }),
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
            bench_group(&mut throughput_group, &rt, id, svc.clone());
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
    contention_throughput_group.throughput(Throughput::Elements(burst_size as u64));
    for (id, svc) in &scenarios {
        if id.contains("managed_throughput") || id.contains("governor") || id.contains("tower") {
            bench_burst(
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
