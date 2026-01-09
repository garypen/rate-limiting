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
use crate::retrylimit_layer::RateLimitRetryLayer;

/// A high-performance, non-blocking rate limiting stack that uses retries.
///
/// This layer uses a retry mechanism. If the rate limit is reached, it will
/// wait for the required duration and then retry the request, up to a
/// maximum timeout.
pub struct ManagedThroughputLayer<L, Req>
where
    L: ?Sized,
{
    limiter: Arc<L>,
    max_wait: Duration,
    _phantom: PhantomData<fn(Req)>,
}

impl<L, Req> Clone for ManagedThroughputLayer<L, Req>
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

impl<S, L, Req> Layer<S> for ManagedThroughputLayer<L, Req>
where
    L: Strategy + ?Sized + Send + Sync + 'static,
    S: Service<Req, Error = BoxError> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    Req: Send + 'static,
{
    type Service = BoxCloneSyncService<Req, S::Response, BoxError>;

    fn layer(&self, inner: S) -> Self::Service {
        // Enable fail_fast so RateLimitService returns error immediately
        let rl = RateLimitService::new(inner, self.limiter.clone()).with_fail_fast(true);

        // Timeout is outer to ensure a hard deadline on the entire process.
        // RateLimitRetryLayer handles the retries when RateLimited.
        let svc = tower::ServiceBuilder::new()
            .map_err(|err: BoxError| {
                if err.is::<tower::timeout::error::Elapsed>() {
                    BoxError::from(ShotError::Timeout)
                } else if let Some(shot_err) = err.downcast_ref::<ShotError>() {
                    // Propagate ShotError as is
                    BoxError::from(shot_err.clone())
                } else {
                    // Wrap any other inner service errors
                    Box::from(ShotError::Inner(err.to_string()))
                }
            })
            .timeout(self.max_wait)
            .layer(RateLimitRetryLayer)
            .service(rl);

        BoxCloneSyncService::new(svc)
    }
}

impl<L, Req> ManagedThroughputLayer<L, Req>
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
