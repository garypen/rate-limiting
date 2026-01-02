use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use shot_limit::Strategy;
use tower::BoxError;
use tower::Layer;
use tower::Service;
use tower::util::BoxCloneSyncService;

use crate::RateLimitService;
use crate::ShotError;

/// A high-performance, non-blocking rate limiting stack.
///
/// This layer uses a "Shed-First" architecture. Instead of queuing requests
/// in memory (which increases latency and risk of OOM), it immediately
/// rejects excess traffic.
///
/// ### Error Responsibilities:
/// - **LoadShedding (`ShotError::Overloaded`)**: Occurs when the rate limit
///   is reached. This happens at the `poll_ready` stage and is near-instant.
/// - **Timeout (`ShotError::Timeout`)**: Occurs if the *inner service* ///   takes too long to respond (e.g., a slow database query).
///
/// This separation ensures that rate-limit rejections never suffer from
/// "buffer bloat" tail latencies.
pub struct ManagedRateLimitLayer<L, Req> {
    limiter: Arc<L>,
    max_wait: Duration, // Required here for the "Managed" experience
    _phantom: PhantomData<fn(Req)>,
}

// Note: Deriving Clone causes issues when using the layer with Axum.
// We'll just implemented it explicitly.
impl<L, Req> Clone for ManagedRateLimitLayer<L, Req> {
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
            max_wait: self.max_wait,
            _phantom: PhantomData,
        }
    }
}

impl<S, L, Req> Layer<S> for ManagedRateLimitLayer<L, Req>
where
    L: Strategy + Send + Sync + 'static,
    S: Service<Req, Error = BoxError> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    Req: Send + 'static,
{
    type Service = BoxCloneSyncService<Req, S::Response, BoxError>;

    fn layer(&self, inner: S) -> Self::Service {
        let rl = RateLimitService::new(inner, self.limiter.clone());

        // Timeout is outer to ensure a hard deadline on the entire process.
        let svc = tower::ServiceBuilder::new()
            .timeout(self.max_wait)
            .load_shed()
            .service(rl);

        // Map the mixed errors into ShotError
        let mapped_svc = tower::util::MapErr::new(svc, |err: BoxError| {
            if err.is::<tower::timeout::error::Elapsed>() {
                BoxError::from(ShotError::Timeout)
            } else if err.is::<tower::load_shed::error::Overloaded>() {
                BoxError::from(ShotError::Overloaded)
            } else if err.is::<ShotError>() {
                err
            } else {
                // Wrap any other inner service errors
                Box::from(ShotError::Inner(err.to_string()))
            }
        });

        BoxCloneSyncService::new(mapped_svc)
    }
}

impl<L: Strategy, Req> ManagedRateLimitLayer<L, Req> {
    pub fn new(limiter: Arc<L>, max_wait: Duration) -> Self {
        Self {
            limiter,
            max_wait,
            _phantom: PhantomData,
        }
    }
}
