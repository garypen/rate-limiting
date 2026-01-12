use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::metrics::Counter;
use opentelemetry::metrics::Meter;
use pin_project_lite::pin_project;
use tokio::time::Instant;
use tokio::time::Sleep;
use tokio::time::Timeout;
use tokio::time::sleep;
use tokio::time::timeout;
use tower::BoxError;
use tower::Service;

use shot_limit::Reason;
use shot_limit::Strategy;

use crate::error::ShotError;

#[derive(Clone, Debug)]
struct RateLimitServiceMetrics {
    early_wake: Counter<u64>,
}

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
    timeout: Option<Duration>,
    wait_start: Option<Instant>,
    meter: Meter,
    instruments: RateLimitServiceMetrics,
}

pin_project! {
    /// A future that wraps the inner service future with a timeout.
    pub struct ResponseFuture<F> {
        #[pin]
        inner: Timeout<F>,
    }
}

impl<F, T, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<T, E>>,
    E: From<BoxError>,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.poll(cx) {
            Poll::Ready(Ok(res)) => Poll::Ready(res),
            Poll::Ready(Err(_)) => Poll::Ready(Err(E::from(Box::new(ShotError::Timeout)))),
            Poll::Pending => Poll::Pending,
        }
    }
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
            timeout: self.timeout,
            wait_start: None,
            meter: self.meter.clone(),
            instruments: self.instruments.clone(),
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
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // 1. If we are currently sleeping, check if we're done
        if let Some(ref mut fut) = self.sleep {
            match fut.as_mut().poll(cx) {
                Poll::Ready(_) => {
                    self.sleep = None;
                    // We woke up.
                    // If we have a timeout, check if we exceeded it during sleep.
                    if let Some(timeout) = self.timeout
                        && let Some(start) = self.wait_start
                        && start.elapsed() >= timeout
                    {
                        self.wait_start = None;
                        return Poll::Ready(Err(Box::new(ShotError::Timeout)));
                    }
                }
                Poll::Pending => {
                    // Early Wake
                    let strategy = format!("{:?}", self.limiter);
                    self.instruments
                        .early_wake
                        .add(1, &[KeyValue::new("early_wake", strategy)]);
                    return Poll::Pending;
                }
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
            // Check timeout before processing
            if let Some(timeout) = self.timeout {
                let start = *self.wait_start.get_or_insert(Instant::now());
                if start.elapsed() >= timeout {
                    self.wait_start = None;
                    return Poll::Ready(Err(Box::new(ShotError::Timeout)));
                }
            }

            match self.limiter.process() {
                ControlFlow::Continue(_) => {
                    self.permit_acquired = true;
                }
                ControlFlow::Break(reason) => {
                    let Reason::Overloaded { retry_after } = reason;

                    if self.fail_fast {
                        return Poll::Ready(Err(Box::new(ShotError::RateLimited { retry_after })));
                    } else {
                        // If this is the first time we are blocking, record start time
                        let start = *self.wait_start.get_or_insert(Instant::now());

                        let sleep_duration = if let Some(timeout) = self.timeout {
                            let elapsed = start.elapsed();
                            let remaining = timeout.saturating_sub(elapsed);
                            if remaining.is_zero() {
                                self.wait_start = None;
                                return Poll::Ready(Err(Box::new(ShotError::Timeout)));
                            }
                            std::cmp::min(retry_after, remaining)
                        } else {
                            retry_after
                        };

                        let mut sleep_fut = Box::pin(sleep(sleep_duration));
                        match sleep_fut.as_mut().poll(cx) {
                            Poll::Pending => {
                                self.sleep = Some(sleep_fut);
                                return Poll::Pending;
                            }
                            Poll::Ready(_) => {
                                // Immediate wakeup (sleep(0) or similar)
                                // If due to timeout expiry
                                if let Some(timeout) = self.timeout
                                    && start.elapsed() >= timeout
                                {
                                    self.wait_start = None;
                                    return Poll::Ready(Err(Box::new(ShotError::Timeout)));
                                }

                                cx.waker().wake_by_ref();
                                return Poll::Pending;
                            }
                        }
                    }
                }
            }
        }

        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.permit_acquired = false;
        let start = self.wait_start.take();
        let timeout_duration = match (self.timeout, start) {
            (Some(t), Some(s)) => t.saturating_sub(s.elapsed()),
            (Some(t), None) => t,
            (None, _) => Duration::from_secs(3600 * 24 * 365), // Effective infinity
        };

        ResponseFuture {
            inner: timeout(timeout_duration, self.inner.call(req)),
        }
    }
}

impl<L, S> RateLimitService<L, S>
where
    L: ?Sized,
{
    pub fn new(inner: S, limiter: Arc<L>) -> Self {
        let meter = global::meter("rate_limit_service");
        let instruments = RateLimitServiceMetrics {
            early_wake: meter.u64_counter("early_wake").build(),
        };

        Self {
            inner,
            limiter,
            sleep: None,
            permit_acquired: false,
            fail_fast: false,
            timeout: None,
            wait_start: None,
            meter: global::meter("rate_limit_service"),
            instruments,
        }
    }

    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}
