use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use tower::BoxError;
use tower::Service;
use tower::ServiceExt;

use crate::ShotError;

pub(crate) struct RateLimitRetryService<S> {
    pub(crate) inner: S,
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
                if let Some(shot_err) = err.downcast_ref::<ShotError>()
                    && let ShotError::RateLimited { .. } = shot_err
                {
                    // Pretend we are ready, so call() gets invoked.
                    // We will handle the retry logic (and sleep) inside call().
                    return Poll::Ready(Ok(()));
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
                        if let Some(shot_err) = err.downcast_ref::<ShotError>()
                            && let ShotError::RateLimited { retry_after } = shot_err
                        {
                            // Wait before retrying
                            tokio::time::sleep(*retry_after).await;
                            continue;
                        }
                        // Propagate other errors
                        return Err(err);
                    }
                }
            }
        })
    }
}
