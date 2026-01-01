use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use rate_limiting_lib::Strategy;
use tower::BoxError;
use tower::Layer;
use tower::Service;
use tower::buffer::Buffer;
use tower::util::BoxService;

use crate::RateLimitService;

// A helper to represent the erased future type Buffer uses internally
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub struct BufferedRateLimitLayer<L, Req> {
    limiter: Arc<L>,
    buffer_size: usize,
    _phantom: PhantomData<fn(Req)>,
}

impl<S, L, Req> Layer<S> for BufferedRateLimitLayer<L, Req>
where
    L: Strategy + Send + Sync + 'static,
    S: Service<Req, Error = BoxError> + Send + 'static,
    S::Future: Send + 'static,
    Req: Send + 'static,
{
    // Fix: We must define the Service type to match Tower's internal
    // Buffer<Request, Future> layout exactly.
    type Service = Buffer<Req, BoxFuture<Result<S::Response, BoxError>>>;

    fn layer(&self, inner: S) -> Self::Service {
        let rl = RateLimitService::new(inner, self.limiter.clone());

        // 1. Erase the complex RateLimitService type into a BoxService.
        // This turns S::Future into Pin<Box<dyn Future...>>
        let boxed_rl = BoxService::new(rl);

        // 2. Now Buffer::new is receiving a Pin<Box<...>> future,
        // which matches our Self::Service definition exactly.
        Buffer::new(boxed_rl, self.buffer_size)
    }
}

impl<L: Strategy, Req> BufferedRateLimitLayer<L, Req> {
    pub fn new(limiter: L, capacity: usize) -> Self {
        let buffer_size = capacity + (capacity / 4).max(10);

        Self {
            limiter: Arc::new(limiter),
            buffer_size,
            _phantom: PhantomData,
        }
    }
}
