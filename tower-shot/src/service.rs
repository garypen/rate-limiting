use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use tokio::time::Sleep;
use tokio::time::sleep;
use tower::BoxError;
use tower::Service;

use shot_limit::Reason;
use shot_limit::Strategy;

use crate::error::ShotError;

#[derive(Debug)]
pub struct RateLimitService<L, S>
where
    L: ?Sized,
{
    inner: S,
    limiter: Arc<L>,
    sleep: Option<Pin<Box<Sleep>>>,
    permit_acquired: bool,
    fail_fast: bool,
}

// Manually implement Clone because Pin<Box<Sleep>> cannot be cloned
impl<L, S> Clone for RateLimitService<L, S>
where
    L: ?Sized,
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limiter: Arc::clone(&self.limiter),
            // We start with a fresh sleep state for the new clone
            sleep: None,
            permit_acquired: false,
            fail_fast: self.fail_fast,
        }
    }
}

impl<L, S, Req> Service<Req> for RateLimitService<L, S>
where
    L: Strategy + ?Sized + Send + Sync + 'static,
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

        // 2. Check inner service readiness FIRST to avoid over-consuming tokens
        match self.inner.poll_ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }

        // 3. Check the strategy if we don't have a permit yet
        if !self.permit_acquired {
            match self.limiter.process() {
                ControlFlow::Continue(_) => {
                    self.permit_acquired = true;
                }
                ControlFlow::Break(reason) => {
                    let Reason::Overloaded { retry_after } = reason;

                    if self.fail_fast {
                        return Poll::Ready(Err(Box::new(ShotError::RateLimited { retry_after })));
                    }

                    let mut sleep_fut = Box::pin(sleep(retry_after));
                    match sleep_fut.as_mut().poll(cx) {
                        Poll::Pending => {
                            self.sleep = Some(sleep_fut);
                            return Poll::Pending;
                        }
                        Poll::Ready(_) => {
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        }
                    }
                }
            }
        }

        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.permit_acquired = false;
        self.inner.call(req)
    }
}

impl<L, S> RateLimitService<L, S>
where
    L: ?Sized,
{
    pub fn new(inner: S, limiter: Arc<L>) -> Self {
        Self {
            inner,
            limiter,
            sleep: None,
            permit_acquired: false,
            fail_fast: false,
        }
    }

    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }
}
