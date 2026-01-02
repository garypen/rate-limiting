use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use tokio::time::Sleep;
use tokio::time::sleep;
use tower::BoxError;
use tower::Service;

use shot_limit::Reason;
use shot_limit::Strategy;

pub struct RateLimitService<L, S> {
    inner: S,
    limiter: Arc<L>,
    sleep: Option<Pin<Box<Sleep>>>,
}

// Manually implement Clone because Pin<Box<Sleep>> cannot be cloned
impl<L, S> Clone for RateLimitService<L, S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limiter: Arc::clone(&self.limiter),
            // We start with a fresh sleep state for the new clone
            sleep: None,
        }
    }
}

impl<L, S, Req> Service<Req> for RateLimitService<L, S>
where
    L: Strategy + Send + Sync + 'static,
    S: Service<Req, Error = BoxError>,
    Req: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // 1. If we are currently sleeping, check if we're done
        if let Some(ref mut fut) = self.sleep {
            match fut.as_mut().poll(cx) {
                Poll::Ready(_) => self.sleep = None,
                Poll::Pending => return Poll::Pending,
            }
        }
        // 2. Check the strategy
        match self.limiter.process() {
            ControlFlow::Continue(_) => self.inner.poll_ready(cx),
            ControlFlow::Break(reason) => {
                let Reason::Overloaded { retry_after } = reason;

                let delay = retry_after + Duration::from_millis(1);
                let mut sleep_fut = Box::pin(sleep(delay));

                // Important: Poll it to register the waker from 'cx'
                match sleep_fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        self.sleep = Some(sleep_fut);
                        Poll::Pending
                    }
                    Poll::Ready(_) => {
                        // If it's already ready (rare but possible),
                        // recurse or simply notify the caller to try again immediately.
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.inner.call(req)
    }
}

impl<L, S> RateLimitService<L, S> {
    pub fn new(inner: S, limiter: Arc<L>) -> Self {
        Self {
            inner,
            limiter,
            sleep: None,
        }
    }
}
