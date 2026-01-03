use std::sync::Arc;

use shot_limit::Strategy;
use tower::Layer;

use crate::service::RateLimitService;

/// Applies Rate Limit to requests.
#[derive(Debug)]
pub struct RateLimitLayer<L>
where
    L: ?Sized,
{
    limiter: Arc<L>,
}

impl<L> Clone for RateLimitLayer<L>
where
    L: ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            limiter: Arc::clone(&self.limiter),
        }
    }
}

impl<L> RateLimitLayer<L>
where
    L: Strategy + ?Sized + Send + Sync + 'static,
{
    /// Create a RateLimitLayer
    pub fn new(limiter: Arc<L>) -> Self {
        RateLimitLayer { limiter }
    }
}

impl<L, S> Layer<S> for RateLimitLayer<L>
where
    L: ?Sized,
{
    type Service = RateLimitService<L, S>;

    fn layer(&self, service: S) -> Self::Service {
        RateLimitService::new(service, self.limiter.clone())
    }
}
