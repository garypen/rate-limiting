use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use hdrhistogram::Histogram;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use shot_limit::FixedWindow;
use shot_limit::Gcra;
use shot_limit::SlidingWindow;
use shot_limit::TokenBucket;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tower::BoxError;
use tower::Layer;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::service_fn;
use tower_shot::ManagedLatencyLayer;
use tower_shot::ManagedThroughputLayer;
use tower_shot::RateLimitLayer;
use tower_shot::ShotError;

async fn mock_db_call(req: u64) -> Result<&'static str, tower::BoxError> {
    // Simulate real-world work (100 - 4_000 ms of DB latency)
    sleep(Duration::from_millis(req)).await;
    Ok("success")
}

#[derive(Default)]
struct RejectionCounter {
    timeouts: usize,
    sheds: usize,
    inner: usize,
    unknown: usize,
}

async fn run_load_test<S>(name: &str, svc: S, total_reqs: usize)
where
    S: Service<u64, Response = &'static str, Error = tower::BoxError> + Clone + Send + 'static,
    S::Future: Send,
{
    let mut hist_elapsed = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap();
    let mut hist_ready = Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).unwrap();

    let mut tasks = JoinSet::new();
    let mut rejections = RejectionCounter::default();
    // 1. Create a seeded RNG using a simple u64
    // Any time you use '42', you will get the exact same sequence.
    let mut rng = StdRng::seed_from_u64(42);

    // 2. Generate your set of random sleep durations (same for each test)
    let mut numbers: Vec<u64> = (0..total_reqs)
        .map(|_| rng.random_range(100..=4_000))
        .collect();

    let barrier = Arc::new(Barrier::new(total_reqs));

    let start = Arc::new(Mutex::new(None));

    for _ in 0..total_reqs {
        let mut local_svc = svc.clone();
        let bar = barrier.clone();
        let duration = numbers.pop().unwrap();
        let start_clone = start.clone();
        tasks.spawn(async move {
            if bar.wait().await.is_leader() {
                *start_clone.lock().unwrap() = Some(Instant::now());
            }
            let req_start = Instant::now();
            let ready_res = local_svc.ready().await;
            let ready = req_start.elapsed();

            if let Ok(ready_svc) = ready_res {
                let res = ready_svc.call(duration).await;
                (res, ready, req_start.elapsed())
            } else {
                (ready_res.map(|_| ""), ready, req_start.elapsed())
            }
        });
    }

    let mut success_count = 0;

    while let Some(task) = tasks.join_next().await {
        let (res, ready, elapsed) = task.expect("Task panicked");
        match res {
            Ok(_) => {
                success_count += 1;
                hist_ready.record(ready.as_nanos() as u64).unwrap();
                hist_elapsed.record(elapsed.as_micros() as u64).unwrap();
            }
            Err(e) => {
                // Check if the error is one of our domain errors
                if let Some(shot_err) = e.downcast_ref::<ShotError>() {
                    match shot_err {
                        ShotError::Timeout => rejections.timeouts += 1,
                        ShotError::Overloaded => rejections.sheds += 1,
                        ShotError::RateLimited { .. } => rejections.sheds += 1,
                        ShotError::Inner(_) => rejections.inner += 1,
                    }
                } else if let Some(_) = e.downcast_ref::<tower::timeout::error::Elapsed>() {
                    rejections.timeouts += 1;
                } else if let Some(_) = e.downcast_ref::<tower::load_shed::error::Overloaded>() {
                    rejections.sheds += 1;
                } else {
                    rejections.unknown += 1;
                }
            }
        }
    }

    let total_duration = start.lock().unwrap().unwrap().elapsed();
    let throughput = total_reqs as f64 / total_duration.as_secs_f64();
    let goodput = success_count as f64 / total_duration.as_secs_f64();

    println!("--- {} ---", name);
    println!("Total Duration:  {:.2?}", total_duration);
    println!("Success/Total:   {}/{}", success_count, total_reqs);
    println!("Total Rate:      {:.2} req/sec", throughput);
    println!("Success Rate:    {:.2} req/sec (Goodput)", goodput);

    if success_count > 0 {
        println!("P50 (Elapsed):   {}µs", hist_elapsed.value_at_quantile(0.5));
        println!(
            "P99 (Elapsed):   {}µs",
            hist_elapsed.value_at_quantile(0.99)
        );
        println!("P50 (Ready):     {}ns", hist_ready.value_at_quantile(0.5));
        println!("P99 (Ready):     {}ns", hist_ready.value_at_quantile(0.99));
    }

    let total_errors =
        rejections.timeouts + rejections.sheds + rejections.inner + rejections.unknown;
    println!("Errors:          {}", total_errors);
    if total_errors > 0 {
        println!("  └─ Timeouts:   {}", rejections.timeouts);
        println!("  └─ LoadSheds:  {}", rejections.sheds);
        if rejections.inner > 0 {
            println!("  └─ Inner:      {}", rejections.inner);
        }
        if rejections.unknown > 0 {
            println!("  └─ Unknown:    {}", rejections.unknown);
        }
    }
    println!();
}
#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let capacity = 10_000.try_into()?;
    let increment = 10_000.try_into()?;
    let period = Duration::from_secs(1);
    let timeout = Duration::from_millis(4_900);
    let total_reqs = 50_000;

    // 1.a. Managed Retry Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let fixed_svc = ManagedThroughputLayer::new(fixed, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Throughput Fixed Window", fixed_svc, total_reqs).await;

    // 1.b. Managed Load Shed Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let fixed_svc = ManagedLatencyLayer::new(fixed, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Latency Fixed Window", fixed_svc, total_reqs).await;

    // 1.c. Raw Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let fixed_svc = RateLimitLayer::new(fixed).layer(service_fn(mock_db_call));
    run_load_test("Raw Fixed Window", fixed_svc, total_reqs).await;

    // 2.a. Managed Retry Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let sliding_svc = ManagedThroughputLayer::new(sliding, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Throughput Sliding Window", sliding_svc, total_reqs).await;

    // 2.b. Managed Load Shed Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let sliding_svc = ManagedLatencyLayer::new(sliding, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Latency Sliding Window", sliding_svc, total_reqs).await;

    // 2.c. Raw Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let sliding_svc = RateLimitLayer::new(sliding).layer(service_fn(mock_db_call));
    run_load_test("Raw Sliding Window", sliding_svc, total_reqs).await;

    // 3.a. Managed Retry Token Bucket Stress
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let bucket_svc = ManagedThroughputLayer::new(bucket, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Throughput Token Bucket", bucket_svc, total_reqs).await;

    // 3.b. Managed Load Shed Token Bucket Stress
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let bucket_svc = ManagedLatencyLayer::new(bucket, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Latency Token Bucket", bucket_svc, total_reqs).await;

    // 3.c. Raw Token Bucket Stress
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let bucket_svc = RateLimitLayer::new(bucket).layer(service_fn(mock_db_call));
    run_load_test("Raw Token Bucket", bucket_svc, total_reqs).await;

    // 4.a. Managed Retry Gcra Stress
    let gcra = Arc::new(Gcra::new(capacity, period));
    let gcra_svc = ManagedThroughputLayer::new(gcra, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Throughput Gcra", gcra_svc, total_reqs).await;

    // 4.b. Managed Load Shed Gcra Stress
    let gcra = Arc::new(Gcra::new(capacity, period));
    let gcra_svc = ManagedLatencyLayer::new(gcra, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Latency Gcra", gcra_svc, total_reqs).await;

    // 4.c. Raw Gcra Stress
    let gcra = Arc::new(Gcra::new(capacity, period));
    let gcra_svc = RateLimitLayer::new(gcra).layer(service_fn(mock_db_call));
    run_load_test("Raw Gcra", gcra_svc, total_reqs).await;

    // 5.a. Tower Built-in (Managed with Responsive timeout and load shedding)
    let tower_svc = ServiceBuilder::new()
        .buffer(capacity.get()) // Buffer size
        .timeout(timeout)
        .load_shed()
        .rate_limit(capacity.get() as u64, period)
        .service(service_fn(mock_db_call));
    run_load_test("Managed Responsive Tower RateLimit", tower_svc, total_reqs).await;

    // 5.b. Tower Built-in (Managed with Production timeout and load shedding)
    let tower_svc = ServiceBuilder::new()
        .timeout(timeout)
        .load_shed()
        .buffer(capacity.get()) // Buffer size
        .rate_limit(capacity.get() as u64, period)
        .service(service_fn(mock_db_call));
    run_load_test("Managed Production Tower RateLimit", tower_svc, total_reqs).await;

    // 5.c. Tower Built-in (Raw)
    let tower_svc = ServiceBuilder::new()
        .buffer(capacity.get()) // Buffer size
        .rate_limit(capacity.get() as u64, period)
        .service(service_fn(mock_db_call));
    run_load_test("Raw Tower RateLimit", tower_svc, total_reqs).await;

    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(())
}
