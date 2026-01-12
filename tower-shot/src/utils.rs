use std::sync::Arc;
use std::time::Duration;

use tower::BoxError;
use tower::Service;
use tower::ServiceBuilder;
use tower::layer::util::Stack;
use tower::load_shed::LoadShedLayer;
use tower::load_shed::error::Overloaded;
use tower::util::BoxCloneSyncService;
use tower::util::MapErrLayer;

use shot_limit::Strategy;

use crate::RateLimitLayer;
use crate::ShotError;

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
            .layer(RateLimitLayer::new(strategy).with_timeout(timeout))
            .service(svc),
    )
}

/// Limit service time with a single unified timeout and shed load
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
            .map_err(map_overloaded)
            .load_shed()
            .layer(
                RateLimitLayer::new(strategy)
                    .with_fail_fast(true)
                    .with_timeout(timeout),
            )
            .service(svc),
    )
}

fn map_overloaded(e: BoxError) -> BoxError {
    if e.is::<Overloaded>() {
        Box::new(ShotError::Overloaded)
    } else {
        e
    }
}

/// Service Builder Extension with additional useful functions for tower::ServiceBuilder.
pub trait ServiceBuilderExt<L> {
    /// Add a high throughput layer
    fn throughput_rate_limit(
        self,
        limiter: Arc<dyn Strategy + Send + Sync + 'static>,
        timeout: Duration,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy + Send + Sync + 'static>, L>>;

    /// Add a load shedding low latency layer
    fn latency_rate_limit(
        self,
        limiter: Arc<dyn Strategy + Send + Sync + 'static>,
        timeout: Duration,
    ) -> ServiceBuilder<
        Stack<
            RateLimitLayer<dyn Strategy + Send + Sync>,
            Stack<LoadShedLayer, Stack<MapErrLayer<fn(BoxError) -> BoxError>, L>>,
        >,
    >;
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
        timeout: Duration,
    ) -> ServiceBuilder<
        Stack<
            RateLimitLayer<dyn Strategy + Send + Sync>,
            Stack<LoadShedLayer, Stack<MapErrLayer<fn(BoxError) -> BoxError>, L>>,
        >,
    > {
        self.map_err(map_overloaded as fn(BoxError) -> BoxError)
            .load_shed()
            .layer(
                RateLimitLayer::new(limiter)
                    .with_fail_fast(true)
                    .with_timeout(timeout),
            )
    }
}
