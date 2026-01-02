/// Errors produced by the Tower Shot middleware stack.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ShotError {
    /// The request was queued but exceeded the maximum allowed wait time.
    ///
    /// When the `axum` feature is enabled, this converts to `408 Request Timeout`.
    #[error("Request timed out waiting for rate limit capacity")]
    Timeout,

    /// The service is currently at peak capacity and cannot queue more requests.
    ///
    /// This is triggered by the Load Shedding layer to protect system resources.
    /// When the `axum` feature is enabled, this converts to `503 Service Unavailable`.
    #[error("Service is overloaded; request shed")]
    Overloaded,

    /// An unexpected error occurred in the inner service.
    ///
    /// The string contains the `Display` representation of the inner error.
    /// When the `axum` feature is enabled, this converts to `500 Internal Server Error`.
    #[error("Internal service error: {0}")]
    Inner(String),
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for ShotError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;

        let status = match self {
            Self::Overloaded => StatusCode::SERVICE_UNAVAILABLE,
            Self::Timeout => StatusCode::REQUEST_TIMEOUT,
            Self::Inner(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (status, self.to_string()).into_response()
    }
}
