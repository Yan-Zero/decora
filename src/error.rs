//! 只对外返回固定说明，绝不回显上游响应、URL、凭据或用户输入。
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// OpenAI 风格错误封装；message 必须是程序内固定字符串。
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
    pub retry_after: Option<u64>,
}
impl ApiError {
    /// 构造不包含敏感信息的错误。
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            retry_after: None,
        }
    }
    /// 请求格式或字段范围错误。
    pub fn invalid() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "请求字段、类型或范围不符合 Decisions 协议。",
        )
    }
    /// 协议存在该能力，但当前网关或候选不能无损支持。
    pub fn unsupported(code: &'static str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            code,
            "当前候选无法无损支持该请求；请查看能力矩阵。",
        )
    }
    /// 上游响应违反合同；不重试或切换模型。
    pub fn protocol() -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            "upstream_protocol_error",
            "上游响应不符合已核实的协议。",
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
