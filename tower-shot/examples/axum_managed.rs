//! Basic Axum example for Configurable Rate Limiting
//!
//! You can configure your router to choose between:
//!  - strategies
//!    - FixedWindow
//!    - SlidingWindow
//!    - TokenBucket
//!  - RateLimiting Style
//!    - Standard (pure rate limiting)
//!    - Retry (ManagedRetryRateLimitLayer - builtin timeout and retries)
//!    - LoadShed (ManagedLoadShedRateLimitLayer - builtin timeout and load-shedding)
//!
//! By default, you get a router which:
//!  - Ensures that no request takes > 500ms
//!  - Uses TokenBucket with capacity requests to 10/second
//!  - Refills Tokens with 10 new tokens/second.
//!
//! We can run `hey` (or your load tool of choice) to verify
//! that this is working correctly as follows:
//!
//! -n: Total requests (100)
//! -c: 1 worker (to ensure the -q limit is global)
//! -q: 20 requests per second
//!
//! We are overloading our limiter to verify enforcement. We
//! are asking it to perform double the requests we support.
//!
//! ```bash
//! hey -n 100 -c 1 -q 20 http://localhost:3000/
//! ```
//!
//! You should see something like 50% 200 and 50% 503 (unavailable) or 408 (timeout).
//!
//! Notes:
//! - The timeout is set as a failsafe to enforce SLAs in case an
//!   inner service is taking too long to complete.
//! - Request limiting is not an exact science, so you'll see some
//!   small variation around 50%.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use clap::Parser;
use clap::ValueEnum;
use shot_limit::FixedWindow;
use shot_limit::Gcra;
use shot_limit::SlidingWindow;
use shot_limit::Strategy;
use shot_limit::TokenBucket;
use tower::BoxError;
use tower::ServiceBuilder;
use tower_shot::ManagedLoadShedRateLimitLayer;
use tower_shot::ManagedRetryRateLimitLayer;
use tower_shot::RateLimitLayer;
use tower_shot::ShotError;

#[derive(ValueEnum, Clone, Debug)]
enum LimiterType {
    Standard,
    Retry,
    LoadShed,
}

#[derive(ValueEnum, Clone, Debug)]
enum StrategyType {
    Fixed,
    Gcra,
    Sliding,
    Bucket,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The rate limiting algorithm to use
    #[arg(short, long, value_enum, default_value_t = StrategyType::Bucket)]
    strategy: StrategyType,

    /// The type of tower layer to apply
    #[arg(short, long, value_enum, default_value_t = LimiterType::Retry)]
    limiter: LimiterType,

    /// Requests per second allowed
    #[arg(short, long, default_value_t = 10)]
    capacity: usize,

    /// Increment tokens per period
    #[arg(short, long, default_value_t = 10)]
    increment: usize,

    /// The period over which capacity is replenished (e.g., "1s", "500ms", "1min")
    #[arg(short, long, value_parser = humantime::parse_duration, default_value = "1s")]
    period: Duration,

    /// Maximum time a request can wait for a permit (e.g., "500ms")
    #[arg(short, long, value_parser = humantime::parse_duration, default_value = "500ms")]
    timeout: Duration,

    /// The address to listen on
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    addr: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let args = Args::parse();

    let capacity = args.capacity.try_into()?;
    let increment = args.increment.try_into()?;

    // Ensure the Arc is created as the trait object type immediately
    let strategy: Arc<dyn Strategy + Send + Sync> = match args.strategy {
        StrategyType::Fixed => Arc::new(FixedWindow::new(capacity, args.period)),
        StrategyType::Gcra => Arc::new(Gcra::new(capacity, args.period)),
        StrategyType::Sliding => Arc::new(SlidingWindow::new(capacity, args.period)),
        StrategyType::Bucket => Arc::new(TokenBucket::new(capacity, increment, args.period)),
    };

    println!(
        "📡 Strategy: {:?} | Limiter: {:?} | Capacity: {} per {:?} | Timeout: {:?}",
        args.strategy, args.limiter, args.capacity, args.period, args.timeout
    );

    let mut app = Router::new().route("/", get(|| async { "Hello, Shot!" }));

    app = match args.limiter {
        LimiterType::Standard => {
            let layer = RateLimitLayer::new(strategy);
            app.layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(handle_shot_error))
                    // Since we aren't using ManagedRateLimitLayer, we
                    // specify load shedding and timeout manually.
                    .timeout(args.timeout)
                    .load_shed()
                    .layer(layer)
                    .map_err(BoxError::from),
            )
        }
        LimiterType::Retry => {
            let layer = ManagedRetryRateLimitLayer::new(strategy, args.timeout);
            app.layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(handle_shot_error))
                    .layer(layer)
                    .map_err(BoxError::from),
            )
        }
        LimiterType::LoadShed => {
            let layer = ManagedLoadShedRateLimitLayer::new(strategy, args.timeout);
            app.layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(handle_shot_error))
                    .layer(layer)
                    .map_err(BoxError::from),
            )
        }
    };

    let listener = tokio::net::TcpListener::bind(args.addr).await?;

    println!(
        "📡 Listening on http://{}", 
        listener.local_addr()?.to_string()
    );

    axum::serve(listener, app).await?;

    Ok(())
}

/// The signature must match BoxError -> IntoResponse
async fn handle_shot_error(err: BoxError) -> impl IntoResponse {
    if let Ok(shot_err) = err.downcast::<ShotError>() {
        shot_err.into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal Service Error").into_response()
    }
}