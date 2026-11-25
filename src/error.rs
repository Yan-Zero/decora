//! Return only fixed public messages; never echo upstream responses, URLs, credentials, or user input.
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// OpenAI-style error wrapper; message must be a fixed string defined in the program.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
    pub retry_after: Option<u64>,
}
impl ApiError {
    /// Construct an error that contains no sensitive information.
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            retry_after: None,
        }
    }
    /// The request format is invalid or a field is out of range.
    pub fn invalid() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Request fields, types or ranges do not match the Decisions protocol.",
        )
    }
    /// The protocol defines this capability, but the gateway or any candidate cannot support it losslessly.
    pub fn unsupported(code: &'static str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            code,
            "No configured upstream can serve this request losslessly; see the capability matrix.",
        )
    }
    /// The upstream response violated the contract; do not retry or switch models.
    pub fn protocol() -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            "upstream_protocol_error",
            "Upstream response violated the verified protocol.",
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let kind = if self.status == StatusCode::UNAUTHORIZED {
            "authentication_error"
        } else if self.status.is_client_error() {
            "invalid_request_error"
        } else {
            "api_error"
        };
        let mut response = (self.status, Json(json!({"error": {"message": self.message, "type": kind, "param": null, "code": self.code}}))).into_response();
        if let Some(seconds) = self.retry_after {
            response.headers_mut().insert(
                "retry-after",
                seconds.to_string().parse().expect("integer header"),
            );
        }
        response
    }
}
