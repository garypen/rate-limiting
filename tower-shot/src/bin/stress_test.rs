use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use hdrhistogram::Histogram;
use shot_limit::FixedWindow;
use shot_limit::SlidingWindow;
use shot_limit::TokenBucket;
use tokio::time::sleep;
use tower::Layer;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;
use tower::service_fn;
use tower_shot::ManagedRateLimitLayer;
use tower_shot::RateLimitLayer;
use tower_shot::ShotError;

async fn mock_db_call(_req: ()) -> Result<&'static str, tower::BoxError> {
    // Simulate real-world work (10ms of DB latency)
    sleep(Duration::from_millis(10)).await;
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
    let mut hist = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap();
    let mut tasks = Vec::with_capacity(total_reqs);
    let mut rejections = RejectionCounter::default();

    let start = Instant::now();

    for _ in 0..total_reqs {
        let mut local_svc = svc.clone();
        tasks.push(tokio::spawn(async move {
            let req_start = Instant::now();
            let ready_res = local_svc.ready().await;

            if let Ok(ready_svc) = ready_res {
                let res = ready_svc.call(()).await;
                (res, req_start.elapsed())
            } else {
                (ready_res.map(|_| ""), req_start.elapsed())
            }
        }));
    }

    let mut success_count = 0;

    for task in tasks {
        let (res, elapsed) = task.await.expect("Task panicked");
        match res {
            Ok(_) => {
                success_count += 1;
                hist.record(elapsed.as_micros() as u64).unwrap();
            }
            Err(e) => {
                // Check if the error is one of our domain errors
                if let Some(shot_err) = e.downcast_ref::<ShotError>() {
                    match shot_err {
                        ShotError::Timeout => rejections.timeouts += 1,
                        ShotError::Overloaded => rejections.sheds += 1,
                        ShotError::Inner(_) => rejections.inner += 1,
                    }
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
        println!("P50 (Success):   {}µs", hist.value_at_quantile(0.5));
        println!("P99 (Success):   {}µs", hist.value_at_quantile(0.99));
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
async fn main() {
    let limit = 1_000;
    let period = Duration::from_secs(1);
    let total_reqs = 5_000;

    // 1.a. Managed Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(NonZeroUsize::new(limit).unwrap(), period));
    let fixed_svc = ManagedRateLimitLayer::new(fixed, period).layer(service_fn(mock_db_call));
    run_load_test("Managed Fixed Window", fixed_svc, total_reqs).await;

    // 1.b. Raw Fixed Window Stress
    let fixed = Arc::new(FixedWindow::new(NonZeroUsize::new(limit).unwrap(), period));
    let fixed_svc = RateLimitLayer::new(fixed).layer(service_fn(mock_db_call));
    run_load_test("Raw Fixed Window", fixed_svc, total_reqs).await;

    // 2.a. Managed Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(
        NonZeroUsize::new(limit).unwrap(),
        period,
    ));
    let sliding_svc = ManagedRateLimitLayer::new(sliding, period).layer(service_fn(mock_db_call));
    run_load_test("Managed Sliding Window", sliding_svc, total_reqs).await;

    // 2.b. Raw Sliding Window Stress
    let sliding = Arc::new(SlidingWindow::new(
        NonZeroUsize::new(limit).unwrap(),
        period,
    ));
    let sliding_svc = RateLimitLayer::new(sliding).layer(service_fn(mock_db_call));
    run_load_test("Raw Sliding Window", sliding_svc, total_reqs).await;

    // 3.a. Token Bucket Stress
    // (Assuming TokenBucket::new(capacity, refill_rate_per_sec))
    let bucket = Arc::new(TokenBucket::new(
        NonZeroUsize::new(limit).unwrap(),
        limit,
        period,
    ));
    let bucket_svc = ManagedRateLimitLayer::new(bucket, period).layer(service_fn(mock_db_call));
    run_load_test("Managed Token Bucket", bucket_svc, total_reqs).await;

    // 3.b. Token Bucket Stress
    // (Assuming TokenBucket::new(capacity, refill_rate_per_sec))
    let bucket = Arc::new(TokenBucket::new(
        NonZeroUsize::new(limit).unwrap(),
        limit,
        period,
    ));
    let bucket_svc = RateLimitLayer::new(bucket).layer(service_fn(mock_db_call));
    run_load_test("Raw Token Bucket", bucket_svc, total_reqs).await;

    // 4.a. Tower Built-in (Buffered to allow the 5_000 reqs to queue)
    let tower_svc = ServiceBuilder::new()
        .buffer(1_000) // Buffer size
        .rate_limit(limit as u64, period)
        .service(service_fn(mock_db_call));
    run_load_test("Buffered Tower RateLimit", tower_svc, total_reqs).await;
}
