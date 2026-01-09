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
    fail_fast: bool,
}

impl<L> Clone for RateLimitLayer<L>
where
    L: ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            limiter: Arc::clone(&self.limiter),
            fail_fast: self.fail_fast,
        }
    }
}

impl<L> RateLimitLayer<L>
where
    L: Strategy + ?Sized + Send + Sync + 'static,
{
    /// Create a RateLimitLayer
    pub fn new(limiter: Arc<L>) -> Self {
        RateLimitLayer {
            limiter,
            fail_fast: false,
        }
    }

    /// Set whether the service should fail immediately when overloaded.
    ///
    /// If `true`, the service will return `ShotError::RateLimited` immediately
    /// instead of waiting for the rate limit to reset.
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }
}

impl<L, S> Layer<S> for RateLimitLayer<L>
where
    L: ?Sized,
{
    type Service = RateLimitService<L, S>;

    fn layer(&self, service: S) -> Self::Service {
        RateLimitService::new(service, self.limiter.clone()).with_fail_fast(self.fail_fast)
    }
}