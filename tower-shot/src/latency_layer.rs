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

/// A high-performance, non-blocking rate limiting stack that prioritizes latency.
///
/// This layer uses a "Shed-First" architecture. Instead of queuing requests
/// in memory, it immediately rejects excess traffic if the rate limit is reached.
pub struct ManagedLatencyLayer<L, Req>
where
    L: ?Sized,
{
    limiter: Arc<L>,
    max_wait: Duration,
    _phantom: PhantomData<fn(Req)>,
}

impl<L, Req> Clone for ManagedLatencyLayer<L, Req>
where
    L: ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
            max_wait: self.max_wait,
            _phantom: PhantomData,
        }
    }
}

impl<S, L, Req> Layer<S> for ManagedLatencyLayer<L, Req>
where
    L: Strategy + ?Sized + Send + Sync + 'static,
    S: Service<Req, Error = BoxError> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    Req: Send + 'static,
{
    type Service = BoxCloneSyncService<Req, S::Response, BoxError>;

    fn layer(&self, inner: S) -> Self::Service {
        let rl = RateLimitService::new(inner, self.limiter.clone());

        // Timeout is outer to ensure a hard deadline on the entire process.
        // LoadShed is used to provide backpressure when the inner service is busy.
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
            } else if let Some(shot_err) = err.downcast_ref::<ShotError>() {
                // Propagate ShotError as is
                BoxError::from(shot_err.clone())
            } else {
                // Wrap any other inner service errors
                Box::from(ShotError::Inner(err.to_string()))
            }
        });

        BoxCloneSyncService::new(mapped_svc)
    }
}

impl<L, Req> ManagedLatencyLayer<L, Req>
where
    L: Strategy + ?Sized,
{
    pub fn new(limiter: Arc<L>, max_wait: Duration) -> Self {
        Self {
            limiter,
            max_wait,
            _phantom: PhantomData,
        }
    }
}
