use std::sync::Arc;

use shot_limit::Strategy;
use tower::Layer;

use crate::service::RateLimitService;

/// Applies GraphQL processing to requests via the supplied inner service.
#[derive(Clone, Debug)]
pub struct RateLimitLayer<L> {
    limiter: Arc<L>,
}

impl<L> RateLimitLayer<L>
where
    L: Strategy + Send + Sync + 'static,
{
    /// Create a RateLimitLayer
    pub fn new(limiter: Arc<L>) -> Self {
        RateLimitLayer { limiter }
    }
}

impl<L, S> Layer<S> for RateLimitLayer<L> {
    type Service = RateLimitService<L, S>;

    fn layer(&self, service: S) -> Self::Service {
        RateLimitService::new(service, self.limiter.clone())
    }
}
