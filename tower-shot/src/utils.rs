use std::sync::Arc;
use std::time::Duration;

use tower::BoxError;
use tower::Service;
use tower::ServiceBuilder;
use tower::layer::util::Stack;
use tower::util::BoxCloneSyncService;

use shot_limit::Strategy;

use crate::RateLimitLayer;

/// Limit service time with a single unified timeout
pub fn make_timeout_svc<S, V, Req, Resp>(
    strategy: Arc<S>,
    timeout: Duration,
    svc: V,
) -> BoxCloneSyncService<Req, Resp, BoxError>
where
    S: Strategy + Send + Sync + 'static,
    Req: Send + 'static,
    V: Service<Req, Response = Resp, Error = BoxError> + Clone + Send + Sync + 'static,
    <V as Service<Req>>::Future: Send,
{
    BoxCloneSyncService::new(
        ServiceBuilder::new()
            /*
                .map_err(|e: BoxError| {
                    if e.is::<Overloaded>() {
                        Box::new(ShotError::Overloaded)
                    } else {
                        e
                    }
                })
            */
            .layer(RateLimitLayer::new(strategy).with_timeout(timeout))
            .service(svc),
    )
}

/// Limit service time and Shed Load with a single unified timeout
pub fn make_latency_svc<S, V, Req, Resp>(
    strategy: Arc<S>,
    timeout: Duration,
    svc: V,
) -> BoxCloneSyncService<Req, Resp, BoxError>
where
    S: Strategy + Send + Sync + 'static,
    Req: Send + 'static,
    V: Service<Req, Response = Resp, Error = BoxError> + Clone + Send + Sync + 'static,
    <V as Service<Req>>::Future: Send,
{
    BoxCloneSyncService::new(
        ServiceBuilder::new()
            /*
                .map_err(|e: BoxError| {
                    if e.is::<Elapsed>() {
                        Box::new(ShotError::Timeout)
                    } else if e.is::<Overloaded>() {
                        panic!("I don't think this can happen");
                        // Box::new(ShotError::Overloaded)
                    } else {
                        e
                    }
                })
            */
            // .timeout(timeout)
            // .load_shed()
            .layer(
                RateLimitLayer::new(strategy)
                    .with_fail_fast(true)
                    .with_timeout(timeout),
            )
            .service(svc),
    )
}

/// Service Builder Extension with additional useful functions for tower::ServiceBuilder.
pub trait ServiceBuilderExt<L> {
    /// Add a high throughput layer
    fn throughput_rate_limit(
        self,
        limiter: Arc<dyn Strategy + Send + Sync + 'static>,
        timeout: Duration,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy + Send + Sync + 'static>, L>>;

    /// Add a low latency layer
    fn latency_rate_limit(
        self,
        limiter: Arc<dyn Strategy + Send + Sync + 'static>,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy + Send + Sync + 'static>, L>>;
}

impl<L> ServiceBuilderExt<L> for ServiceBuilder<L> {
    fn throughput_rate_limit(
        self,
        limiter: Arc<dyn Strategy + Send + Sync + 'static>,
        timeout: Duration,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy + Send + Sync + 'static>, L>> {
        self.layer(RateLimitLayer::new(limiter).with_timeout(timeout))
    }

    fn latency_rate_limit(
        self,
        limiter: Arc<dyn Strategy + Send + Sync + 'static>,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy + Send + Sync + 'static>, L>> {
        self.layer(RateLimitLayer::new(limiter).with_fail_fast(true))
    }
}
