use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use hdrhistogram::Histogram;
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
use tower_shot::ManagedRateLimitLayer;
use tower_shot::RateLimitLayer;
use tower_shot::ShotError;

async fn mock_db_call(_req: ()) -> Result<&'static str, tower::BoxError> {
    // Simulate real-world work (500ms of DB latency)
    sleep(Duration::from_millis(500)).await;
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
    S: Service<(), Response = &'static str, Error = tower::BoxError> + Clone + Send + 'static,
    S::Future: Send,
{
    let mut hist_elapsed = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap();
    let mut hist_ready = Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).unwrap();

    let mut tasks = JoinSet::new();
    let mut rejections = RejectionCounter::default();

    let start = Instant::now();

    let barrier = Arc::new(Barrier::new(total_reqs));

    for _ in 0..total_reqs {
        let mut local_svc = svc.clone();
        let bar = barrier.clone();
        tasks.spawn(async move {
            bar.wait().await;
            let req_start = Instant::now();
            let ready_res = local_svc.ready().await;
            let ready = req_start.elapsed();

            if let Ok(ready_svc) = ready_res {
                let res = ready_svc.call(()).await;
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

    let total_duration = start.elapsed();
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
    let timeout = Duration::from_millis(550);
    let total_reqs = 50_000;

    // 1.a. Managed Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let fixed_svc = ManagedRateLimitLayer::new(fixed, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Fixed Window", fixed_svc, total_reqs).await;

    // 1.b. Raw Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(capacity, period));
    let fixed_svc = RateLimitLayer::new(fixed).layer(service_fn(mock_db_call));
    run_load_test("Raw Fixed Window", fixed_svc, total_reqs).await;

    // 2.a. Managed Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let sliding_svc = ManagedRateLimitLayer::new(sliding, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Sliding Window", sliding_svc, total_reqs).await;

    // 2.b. Raw Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(capacity, period));
    let sliding_svc = RateLimitLayer::new(sliding).layer(service_fn(mock_db_call));
    run_load_test("Raw Sliding Window", sliding_svc, total_reqs).await;

    // 3.a. Token Bucket Stress
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let bucket_svc = ManagedRateLimitLayer::new(bucket, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Token Bucket", bucket_svc, total_reqs).await;

    // 3.b. Token Bucket Stress
    let bucket = Arc::new(TokenBucket::new(capacity, increment, period));
    let bucket_svc = RateLimitLayer::new(bucket).layer(service_fn(mock_db_call));
    run_load_test("Raw Token Bucket", bucket_svc, total_reqs).await;

    // 4.a. Gcra Stress
    let bucket = Arc::new(Gcra::new(capacity, period));
    let bucket_svc = ManagedRateLimitLayer::new(bucket, timeout).layer(service_fn(mock_db_call));
    run_load_test("Managed Gcra", bucket_svc, total_reqs).await;

    // 4.b. Gcra Stress
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

    Ok(())
}
