use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use rate_limiting_lib::Strategy;
use tower::buffer::Buffer;
use tower::load_shed::LoadShed;
use tower::timeout::Timeout;
use tower::util::BoxCloneService;
use tower::{BoxError, Layer, Service};

use crate::RateLimitService;

pub struct ManagedRateLimitLayer<L, Req> {
    limiter: Arc<L>,
    buffer_size: usize,
    max_wait: Duration, // Required here for the "Managed" experience
    _phantom: PhantomData<fn(Req)>,
}

impl<S, L, Req> Layer<S> for ManagedRateLimitLayer<L, Req>
where
    L: Strategy + Send + Sync + 'static,
    S: Service<Req, Error = BoxError> + Send + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    Req: Send + 'static,
{
    type Service = BoxCloneService<Req, S::Response, BoxError>;

    fn layer(&self, inner: S) -> Self::Service {
        let rl = RateLimitService::new(inner, self.limiter.clone());

        // The "Batteries-Included" Stack
        let buffered = Buffer::new(rl, self.buffer_size);
        let loadshed = LoadShed::new(buffered);
        let timeout = Timeout::new(loadshed, self.max_wait);

        BoxCloneService::new(timeout)
    }
}

impl<L: Strategy, Req> ManagedRateLimitLayer<L, Req> {
    pub fn new(limiter: L, capacity: usize, max_wait: Duration) -> Self {
        let buffer_size = capacity + (capacity / 4).max(10);

        Self {
            limiter: Arc::new(limiter),
            buffer_size,
            max_wait,
            _phantom: PhantomData,
        }
    }
}
