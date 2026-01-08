use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use shot_limit::Strategy;
use tower::BoxError;
use tower::Layer;
use tower::Service;
use tower::ServiceExt;
use tower::util::BoxCloneSyncService;

use crate::RateLimitService;
use crate::ShotError;

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
            .timeout(self.max_wait)
            .layer(RateLimitRetryLayer)
            .service(rl);

        // Map the mixed errors into ShotError
        let mapped_svc = tower::util::MapErr::new(svc, |err: BoxError| {
            if err.is::<tower::timeout::error::Elapsed>() {
                BoxError::from(ShotError::Timeout)
            } else if let Some(shot_err) = err.downcast_ref::<ShotError>() {
                // Propagate ShotError as is
                BoxError::from(shot_err.clone())
            } else {
                // Wrap any other inner service errors
                Box::from(ShotError::Inner(err.to_string()))
            }
        });

        BoxCloneSyncService::new(mapped_svc)
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

/// A high-performance, non-blocking rate limiting stack that prioritizes latency.
///
/// This layer uses a "Shed-First" architecture. Instead of queuing requests
/// in memory, it immediately rejects excess traffic if the rate limit is reached.
pub struct ManagedLatencyLayer<L, Req>
where
    L: ?Sized,
{
    limiter: Arc<L>,
    max_wait: Duration,
    _phantom: PhantomData<fn(Req)>,
}

impl<L, Req> Clone for ManagedLatencyLayer<L, Req>
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

impl<S, L, Req> Layer<S> for ManagedLatencyLayer<L, Req>
where
    L: Strategy + ?Sized + Send + Sync + 'static,
    S: Service<Req, Error = BoxError> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    Req: Send + 'static,
{
    type Service = BoxCloneSyncService<Req, S::Response, BoxError>;

    fn layer(&self, inner: S) -> Self::Service {
        let rl = RateLimitService::new(inner, self.limiter.clone());

        // Timeout is outer to ensure a hard deadline on the entire process.
        // LoadShed is used to provide backpressure when the inner service is busy.
        let svc = tower::ServiceBuilder::new()
            .timeout(self.max_wait)
            .load_shed()
            .service(rl);

        // Map the mixed errors into ShotError
        let mapped_svc = tower::util::MapErr::new(svc, |err: BoxError| {
            if err.is::<tower::timeout::error::Elapsed>() {
                BoxError::from(ShotError::Timeout)
            } else if err.is::<tower::load_shed::error::Overloaded>() {
                BoxError::from(ShotError::Overloaded)
            } else if let Some(shot_err) = err.downcast_ref::<ShotError>() {
                // Propagate ShotError as is
                BoxError::from(shot_err.clone())
            } else {
                // Wrap any other inner service errors
                Box::from(ShotError::Inner(err.to_string()))
            }
        });

        BoxCloneSyncService::new(mapped_svc)
    }
}

impl<L, Req> ManagedLatencyLayer<L, Req>
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

#[derive(Clone)]
struct RateLimitRetryLayer;

impl<S> Layer<S> for RateLimitRetryLayer {
    type Service = RateLimitRetryService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitRetryService { inner }
    }
}

struct RateLimitRetryService<S> {
    inner: S,
}

impl<S: Clone> Clone for RateLimitRetryService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S, Req> Service<Req> for RateLimitRetryService<S>
where
    S: Service<Req, Error = BoxError> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    Req: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    // We return a BoxFuture because we are creating an async block
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.inner.poll_ready(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(err)) => {
                // Check if it's a RateLimited error
                if let Some(shot_err) = err.downcast_ref::<ShotError>() {
                    if let ShotError::RateLimited { .. } = shot_err {
                        // Pretend we are ready, so call() gets invoked.
                        // We will handle the retry logic (and sleep) inside call().
                        return Poll::Ready(Ok(()));
                    }
                }
                Poll::Ready(Err(err))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            loop {
                match inner.ready().await {
                    Ok(svc) => {
                        return svc.call(req).await;
                    }
                    Err(err) => {
                        if let Some(shot_err) = err.downcast_ref::<ShotError>() {
                            if let ShotError::RateLimited { retry_after } = shot_err {
                                // Wait before retrying
                                tokio::time::sleep(*retry_after).await;
                                continue;
                            }
                        }
                        // Propagate other errors
                        return Err(err);
                    }
                }
            }
        })
    }
}
