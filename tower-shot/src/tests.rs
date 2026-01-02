use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use shot_limit::FixedWindow;
use shot_limit::Reason;
use shot_limit::SlidingWindow;
use shot_limit::Strategy;
use shot_limit::TokenBucket;
use tower::BoxError;
use tower::Layer;
use tower::Service;
use tower::ServiceBuilder;
use tower::ServiceExt;

use super::*;

use futures::future::Ready;
use futures::future::ready;

#[derive(Clone)]
struct MockService {
    pub count: Arc<AtomicUsize>,
}

impl Service<()> for MockService {
    type Response = ();
    type Error = BoxError;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: ()) -> Self::Future {
        self.count.fetch_add(1, Ordering::SeqCst);
        ready(Ok(()))
    }
}

// A mock strategy that blocks exactly once, then allows everything
#[derive(Debug)]
struct InstantRecoveryStrategy {
    already_blocked: AtomicBool,
}

impl Strategy for InstantRecoveryStrategy {
    fn process(&self) -> ControlFlow<Reason> {
        if self.already_blocked.swap(true, Ordering::SeqCst) {
            ControlFlow::Continue(())
        } else {
            // Hint an immediate recovery
            ControlFlow::Break(Reason::Overloaded {
                retry_after: Duration::from_nanos(0),
            })
        }
    }
}

macro_rules! test_limiter_service {
    ($name:ident, $strategy_type:ty, $strategy_init:expr) => {
        #[cfg(test)]
        mod $name {
            use super::*;
            use tokio::time::advance;
            use tokio::time::pause;

            #[tokio::test]
            async fn test_poll_ready_backpressure() {
                pause();

                let capacity = NonZeroUsize::new(2).unwrap();
                let interval = Duration::from_millis(100);

                // FIX E0618: Wrap $strategy_init in parentheses
                let strategy = ($strategy_init)(capacity, interval);

                let mock = MockService {
                    count: Arc::new(AtomicUsize::new(0)),
                };
                let mut service = RateLimitService::new(mock, Arc::new(strategy));

                // FIX E0282: Use Turbofish with ServiceExt to force type inference for ()
                let _ = service.ready().await.unwrap();
                service.call(()).await.unwrap();

                let _ = service.ready().await.unwrap();
                service.call(()).await.unwrap();

                // 2. This poll must stay Pending
                // We use ready() here which should be the generic future
                let mut ready_fut = service.ready();
                tokio::select! {
                    _ = &mut ready_fut => panic!("Should be throttled!"),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }

                // 3. Advance time
                advance(Duration::from_millis(110)).await;

                // 4. Now it should succeed
                // Ensure we finish the future we started
                ready_fut.await.expect("Should recover");
                service.call(()).await.unwrap();
            }
        }
    };
}

// --- Applying the Macro to your 3 Strategies ---

test_limiter_service!(fixed_window_tests, FixedWindow, FixedWindow::new);

test_limiter_service!(sliding_window_tests, SlidingWindow, SlidingWindow::new);

test_limiter_service!(
    token_bucket_tests,
    TokenBucket,
    |cap, int| TokenBucket::new(cap, 1, int)
);

#[tokio::test]
async fn test_layer_integration() {
    let limiter = SlidingWindow::new(NonZeroUsize::new(100).unwrap(), Duration::from_secs(1));

    let mut service = tower::ServiceBuilder::new()
        .layer(RateLimitLayer::new(Arc::new(limiter)))
        .service(MockService {
            count: Arc::new(AtomicUsize::new(0)),
        });

    // Verify it handles a basic request
    service.ready().await.unwrap().call(()).await.unwrap();
}

#[tokio::test]
async fn test_shared_state_across_clones() {
    let rl = FixedWindow::new(NonZeroUsize::new(1).unwrap(), Duration::from_secs(10));
    let layer = RateLimitLayer::new(Arc::new(rl));

    let mut svc1 = layer.layer(MockService {
        count: Arc::new(AtomicUsize::new(0)),
    });
    let mut svc2 = layer.layer(MockService {
        count: Arc::new(AtomicUsize::new(0)),
    });

    svc1.ready().await.unwrap().call(()).await.unwrap();

    // svc2 should now be throttled because svc1 used the token
    assert!(futures::poll!(svc2.ready()).is_pending());
}

#[tokio::test]
async fn test_concurrent_hammer() {
    let capacity = 50;
    let strategy = Arc::new(SlidingWindow::new(
        NonZeroUsize::new(capacity).unwrap(),
        Duration::from_millis(100),
    ));

    let mock_count = Arc::new(AtomicUsize::new(0));
    let mock = MockService {
        count: mock_count.clone(), // Keep a reference to check later
    };

    let service = RateLimitService::new(mock, strategy);
    let service = tower::buffer::Buffer::new(service, 100);

    let mut handles = vec![];
    for _ in 0..100 {
        let mut svc = service.clone();
        handles.push(tokio::spawn(async move {
            // This will block until the service is ready
            let _ = svc.ready().await.expect("Service should stay healthy");
            svc.call(()).await
        }));
    }

    // Wait for all tasks to complete their attempts
    // Note: Since we don't advance time, 50 will succeed and 50 will stay Pending
    // unless we use a timeout or a specific test structure.

    // ADJUSTMENT: We only expect 50 to resolve.
    // If we await all 100, the test will HANG because 50 are stuck in Pending!
    let mut completed = 0;
    let timeout = tokio::time::sleep(Duration::from_millis(50));
    tokio::pin!(timeout);

    for h in handles {
        tokio::select! {
            res = h => {
                res.expect("Task panicked").expect("Call failed");
                completed += 1;
            }
            _ = &mut timeout => {
                // Timeout reached, stop waiting for more tasks
                break;
            }
        }
    }

    // ASSERTION: Only exactly 50 requests should have been allowed through
    assert_eq!(
        mock_count.load(Ordering::SeqCst),
        capacity,
        "Limiter allowed more/less than capacity under pressure"
    );
    assert_eq!(
        completed, capacity,
        "Tasks completed count doesn't match allowed capacity"
    );
}

#[tokio::test]
async fn test_immediate_recovery() {
    let strategy = Arc::new(InstantRecoveryStrategy {
        already_blocked: AtomicBool::new(false),
    });
    let mock = MockService {
        count: Arc::new(AtomicUsize::new(0)),
    };
    let mut service = RateLimitService::new(mock, strategy);

    // 1. First call to poll_ready will see the 'Break' with 0ms hint.
    // The service should register a sleep(0) and return Pending.
    let ready_fut = service.ready();

    // We yield to the executor to let the 0ms timer fire immediately
    tokio::task::yield_now().await;

    // 2. The service should wake up immediately and be ready now
    ready_fut
        .await
        .expect("Should recover immediately from 0ms hint");
    service.call(()).await.unwrap();
}

#[tokio::test]
async fn test_managed_layer_cloning_concurrency() {
    let capacity = 5;
    let limiter = FixedWindow::new(
        NonZeroUsize::new(capacity).unwrap(),
        Duration::from_secs(60),
    );

    // Create the Managed Layer (Wait up to 100ms before failing)
    let layer = ManagedRateLimitLayer::new(Arc::new(limiter), Duration::from_millis(100));

    let mock_count = Arc::new(AtomicUsize::new(0));
    let service = ServiceBuilder::new().layer(layer).service(MockService {
        count: mock_count.clone(),
    });

    let mut handles = vec![];

    // Fire 20 requests from 20 different clones
    for _ in 0..20 {
        let mut cloned_svc = service.clone(); // Testing BoxCloneService here
        handles.push(tokio::spawn(async move {
            // Wait for readiness (Buffer worker handles this)
            let ready_svc = cloned_svc.ready().await?;
            ready_svc.call(()).await
        }));
    }

    let mut success = 0;
    let mut failure = 0;

    for h in handles {
        match h.await.unwrap() {
            Ok(_) => success += 1,
            Err(_) => failure += 1,
        }
    }

    // ASSERTIONS
    assert_eq!(success, capacity, "Should have exactly 5 successes");
    assert_eq!(
        failure, 15,
        "Remaining 15 should have timed out or been rejected"
    );
    assert_eq!(
        mock_count.load(Ordering::SeqCst),
        capacity,
        "Inner service should only see 5 hits"
    );
}
