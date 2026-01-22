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

/// Create a "Throughput" optimized service.
///
/// This configuration maximizes success rate by allowing requests to wait for a permit,
/// up to the specified `timeout`.
///
/// **Behavior:**
/// 1. If a permit is available, the request proceeds.
/// 2. If the limit is reached, the request **waits** (retries) until a permit is available.
/// 3. If the total time (waiting + processing) exceeds `timeout`, `ShotError::Timeout` is returned.
pub fn make_timeout_svc<S, V, Req, Resp>(
    strategy: Arc<S>,
    timeout: Duration,
    svc: V,
) -> BoxCloneSyncService<Req, Resp, BoxError>
where
    S: Strategy + Send + Sync + 'static,
    Req: Send + 'static,
    V: Service<Req, Response = Resp, Error = BoxError> + Clone + Send + Sync + 'static,
    <V as Service<Req>>::Future: Send + Sync,
{
    ServiceBuilder::new()
        .boxed_clone_sync()
        .layer(RateLimitLayer::new(strategy).with_timeout(timeout))
        .service(svc)
}

/// Create a "Latency" optimized service.
///
/// This configuration prioritizes low latency and system stability ("Fail Fast").
///
/// **Behavior:**
/// 1. If a permit is available, the request proceeds.
/// 2. If the limit is reached, the request is **immediately rejected** with `ShotError::Overloaded`.
/// 3. The `timeout` is applied only to the *execution* of the inner service (since there is no waiting for permits).
pub fn make_latency_svc<S, V, Req, Resp>(
    strategy: Arc<S>,
    timeout: Duration,
    svc: V,
) -> BoxCloneSyncService<Req, Resp, BoxError>
where
    S: Strategy + Send + Sync + 'static,
    Req: Send + 'static,
    V: Service<Req, Response = Resp, Error = BoxError> + Clone + Send + Sync + 'static,
    <V as Service<Req>>::Future: Send + Sync,
{
    ServiceBuilder::new()
        .boxed_clone_sync()
        .map_err(map_overloaded)
        .load_shed()
        .layer(
            RateLimitLayer::new(strategy)
                .with_fail_fast(true)
                .with_timeout(timeout),
        )
        .service(svc)
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
    /// Add a high throughput layer (see [`make_timeout_svc`])
    fn throughput_rate_limit(
        self,
        limiter: Arc<dyn Strategy>,
        timeout: Duration,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy>, L>>;

    /// Add a load shedding low latency layer (see [`make_latency_svc`])
    fn latency_rate_limit(
        self,
        limiter: Arc<dyn Strategy>,
        timeout: Duration,
    ) -> ServiceBuilder<
        Stack<
            RateLimitLayer<dyn Strategy>,
            Stack<LoadShedLayer, Stack<MapErrLayer<fn(BoxError) -> BoxError>, L>>,
        >,
    >;
}

impl<L> ServiceBuilderExt<L> for ServiceBuilder<L> {
    fn throughput_rate_limit(
        self,
        limiter: Arc<dyn Strategy>,
        timeout: Duration,
    ) -> ServiceBuilder<Stack<RateLimitLayer<dyn Strategy>, L>> {
        self.layer(RateLimitLayer::new(limiter).with_timeout(timeout))
    }

    fn latency_rate_limit(
        self,
        limiter: Arc<dyn Strategy>,
        timeout: Duration,
    ) -> ServiceBuilder<
        Stack<
            RateLimitLayer<dyn Strategy>,
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
