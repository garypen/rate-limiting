use axum::{
    Router, error_handling::HandleErrorLayer, extract::Request, http::StatusCode,
    response::IntoResponse, routing::get,
};
use shot_limit::TokenBucket;
use std::sync::Arc;
use std::time::Duration;
use tower::BoxError;
use tower::ServiceBuilder;
use tower_shot::{ManagedRateLimitLayer, ShotError};

#[tokio::main]
async fn main() {
    // 1. Setup Strategy
    let limit = 10.try_into().unwrap();
    let strategy = Arc::new(TokenBucket::new(limit, 10, Duration::from_secs(1)));

    // 2. Setup Managed Layer
    let managed_layer =
        ManagedRateLimitLayer::<_, Request>::new(strategy, Duration::from_millis(500));
    // 3. Build the Router
    let app = Router::new()
        .route("/", get(|| async { "Hello, Shot!" }))
        .layer(
            ServiceBuilder::new()
                // 1. The outermost layer: catches BoxError and returns Response
                .layer(HandleErrorLayer::new(handle_shot_error))
                // 2. The middle layer: introduces BoxError
                .layer(managed_layer)
                // 3. The secret sauce: converts the Route's Infallible to BoxError
                // so that ManagedRateLimitLayer is happy wrapping it.
                .map_err(BoxError::from),
        );

    // 4. Serve
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!("📡 Listening on http://127.0.0.1:3000");

    // This will now compile because the ServiceBuilder stack is Infallible
    axum::serve(listener, app).await.unwrap();
}

/// The signature must match BoxError -> IntoResponse
async fn handle_shot_error(err: tower::BoxError) -> impl IntoResponse {
    if let Some(shot_err) = err.downcast_ref::<ShotError>() {
        shot_err.clone().into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal Service Error").into_response()
    }
}
