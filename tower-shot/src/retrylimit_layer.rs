use tower::Layer;

use crate::retrylimit_service::RateLimitRetryService;

#[derive(Clone)]
pub(crate) struct RateLimitRetryLayer;

impl<S> Layer<S> for RateLimitRetryLayer {
    type Service = RateLimitRetryService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitRetryService { inner }
    }
}
