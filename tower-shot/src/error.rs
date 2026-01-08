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

    /// The request was rejected due to rate limiting.
    ///
    /// The duration indicates when the client should retry.
    /// When the `axum` feature is enabled, this converts to `429 Too Many Requests`
    /// with a `Retry-After` header.
    #[error("Rate limit exceeded; retry after {retry_after:?}")]
    RateLimited {
        /// The duration to wait before retrying.
        retry_after: std::time::Duration,
    },

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

        let (status, msg, headers) = match self {
            Self::Overloaded => (StatusCode::SERVICE_UNAVAILABLE, self.to_string(), None),
            Self::Timeout => (StatusCode::REQUEST_TIMEOUT, self.to_string(), None),
            Self::RateLimited { retry_after } => {
                let secs = retry_after.as_secs().max(1);
                let val = axum::http::HeaderValue::from(secs);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    self.to_string(),
                    Some((axum::http::header::RETRY_AFTER, val)),
                )
            }
            Self::Inner(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string(), None),
        };

        let mut response = (status, msg).into_response();
        if let Some((name, value)) = headers {
            response.headers_mut().insert(name, value);
        }
        response
    }
}
