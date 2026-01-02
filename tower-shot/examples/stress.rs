use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tower::Layer;
use tower::Service;
use tower::ServiceExt;

use shot_limit::TokenBucket;
use tower_shot::ManagedRateLimitLayer;
use tower_shot::ShotError;

#[tokio::main]
async fn main() {
    // 1. Setup Strategy: 100 tokens, refills 10 every 100ms
    let limit = 100.try_into().unwrap();
    let refill_amount = 10.try_into().unwrap();
    let interval = Duration::from_millis(100);
    let bucket = Arc::new(TokenBucket::new(limit, refill_amount, interval));

    // 2. Setup Managed Layer: 50ms total wait budget
    // This is tight! It ensures we see Timeouts quickly.
    let max_wait = Duration::from_millis(50);
    let layer = ManagedRateLimitLayer::new(bucket, max_wait);

    // 3. Define a "Work" service
    let service = tower::service_fn(|_| async {
        // Simulate a tiny bit of processing time
        sleep(Duration::from_millis(1)).await;
        Ok::<&str, tower::BoxError>("Request Successful")
    });

    let managed_service = layer.layer(service);

    println!("🚀 Starting Stress Test...");
    println!("Strategy: TokenBucket (Cap: 100, Refill: 10/100ms)");
    println!("Managed Budget: 50ms wait\n");

    // 4. Fire 200 requests instantly
    let mut tasks = Vec::new();
    for i in 0..200 {
        let mut svc = managed_service.clone();
        tasks.push(tokio::spawn(async move {
            // We use .ready() because ManagedRateLimitLayer handles the load shedding
            match svc.ready().await {
                Ok(ready_svc) => match ready_svc.call(()).await {
                    Ok(resp) => println!("[{i:03}] ✅ {resp}"),
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

    println!("\n🏁 Stress test complete.");
}
