use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tower::Layer;
use tower::Service;
use tower::ServiceExt;

use shot_limit::TokenBucket;
use tower_shot::ManagedLoadShedRateLimitLayer;
use tower_shot::ManagedRetryRateLimitLayer;
use tower_shot::ShotError;

#[tokio::main]
async fn main() {
    // 1. Setup Strategy: 10 tokens, refills 1 every 100ms
    let limit = 10.try_into().unwrap();
    let refill_amount = 1.try_into().unwrap();
    let interval = Duration::from_millis(100);
    let bucket = Arc::new(TokenBucket::new(limit, refill_amount, interval));

    // 2. Setup budget
    let max_wait = Duration::from_millis(500);

    // 3. Define a "Work" service
    let service = tower::service_fn(|i: usize| async move {
        // Simulate a tiny bit of processing time
        sleep(Duration::from_millis(1)).await;
        Ok::<_, tower::BoxError>(format!("Request {i:03} Successful"))
    });

    println!("🚀 Starting Managed Layers Stress Test...");
    println!("Strategy: TokenBucket (Cap: 10, Refill: 1/100ms)");
    println!("Budget: 500ms wait\n");

    println!("---\n--- Testing ManagedRetryRateLimitLayer (Maximizes throughput) ---");
    run_stress(ManagedRetryRateLimitLayer::new(bucket.clone(), max_wait).layer(service.clone()))
        .await;

    println!("\n--- Testing ManagedLoadShedRateLimitLayer (Prioritizes latency) ---");
    run_stress(ManagedLoadShedRateLimitLayer::new(bucket, max_wait).layer(service)).await;

    println!("\n🏁 Stress test complete.");
}

async fn run_stress<S>(svc: S)
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
