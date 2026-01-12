use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tower::Service;
use tower::ServiceExt;

use shot_limit::TokenBucket;
use tower_shot::ShotError;
use tower_shot::make_latency_svc;
use tower_shot::make_timeout_svc;

#[tokio::main]
async fn main() {
    // 1. Setup Strategy: 10 tokens, refills 1 every 100ms
    let limit = 10.try_into().unwrap();
    let refill_amount = 1.try_into().unwrap();
    let interval = Duration::from_millis(100);
    let bucket = Arc::new(TokenBucket::new(limit, refill_amount, interval));

    // 2. Setup budget
    let total_timeout = Duration::from_millis(500);

    // 3. Define a "Work" service
    let service = tower::service_fn(|i: usize| async move {
        // Simulate a tiny bit of processing time
        sleep(Duration::from_millis(1)).await;
        Ok::<_, tower::BoxError>(format!("Request {i:03} Successful"))
    });

    println!("🚀 Starting Managed Layers Stress Test...");
    println!("Strategy: TokenBucket (Cap: 10, Refill: 1/100ms)");
    println!("Budget: 500ms total timeout\n");

    println!("---\n--- Testing Managed Throughput (Timeout Service) ---");
    run_stress(make_timeout_svc(bucket.clone(), total_timeout, service.clone())).await;

    println!("\n--- Testing Managed Latency (Latency Service) ---");
    run_stress(make_latency_svc(bucket, total_timeout, service)).await;

    println!("\n🏁 Stress test complete.");
}

async fn run_stress<S>(
    svc: S,
)
where
    S: Service<usize, Response = String, Error = tower::BoxError> + Clone + Send + 'static,
    S::Future: Send,
{
    let mut tasks = Vec::new();
    for i in 0..50 {
        let mut local_svc = svc.clone();
        tasks.push(tokio::spawn(async move {
            match local_svc.ready().await {
                Ok(ready_svc) => match ready_svc.call(i).await {
                    Ok(resp) => println!("✅ {}", resp),
                    Err(e) => {
                        if let Some(shot_err) = e.downcast_ref::<ShotError>() {
                            println!("[{i:03}] ❌ Shot Rejected: {shot_err}");
                        } else {
                            println!("[{i:03}] 💥 Unexpected Error: {e}");
                        }
                    }
                },
                Err(e) => println!("[{i:03}] ⚠️ Service Unavailable: {e}"),
            }
        }));
    }

    // Wait for all requests to finish
    for task in tasks {
        let _ = task.await;
    }
}