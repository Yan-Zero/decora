//! HTTP 服务、连接复用、带能力过滤的加权优先级路由和有界故障回退。
use crate::{
    adapter::compatible,
    config::{Backend, Config, resolve_secret},
    error::ApiError,
    protocol::{DecisionRequest, DecisionResponse},
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rand::Rng;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

struct RuntimeBackend {
    config: Backend,
    key: String,
}
#[derive(Clone, Copy)]
struct Deadline(Instant);
/// 可廉价 clone 的服务状态；所有请求共用一个 reqwest 连接池。
#[derive(Clone)]
pub struct Gateway {
    inner: Arc<Inner>,
}
struct Inner {
    config: Config,
    backends: BTreeMap<String, RuntimeBackend>,
    bearer: Option<String>,
    client: reqwest::Client,
    slots: Arc<Semaphore>,
}
impl Gateway {
    /// 使用环境变量解析配置，构造启用 TLS 校验且禁止 redirect 的连接池。
    pub fn new(config: Config) -> Result<Self, String> {
        Self::with_env(config, |name| std::env::var(name).ok())
    }
    /// 可注入凭据解析器；离线测试无需真实密钥或修改全局环境。
    pub fn with_env(
        config: Config,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        config.validate()?;
        let bearer = config
            .bearer_key_env
            .as_ref()
            .map(|name| resolve_secret(name, &lookup))
            .transpose()?;
        let mut backends = BTreeMap::new();
        for (id, backend) in &config.backends {
            let key = resolve_secret(&backend.api_key_env, &lookup)?;
            backends.insert(
                id.clone(),
                RuntimeBackend {
                    config: backend.clone(),
                    key,
                },
            );
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .user_agent("Decora/0.1")
            .build()
            .map_err(|_| "HTTP TLS 客户端初始化失败".to_owned())?;
        let slots = Arc::new(Semaphore::new(config.max_in_flight));
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                backends,
                bearer,
                client,
                slots,
            }),
        })
    }
    /// 构造仅含 Decisions 和最小健康检查的 Router；鉴权发生在 body 读取之前。
    pub fn router(&self) -> Router {
        let decisions = Router::new()
            .route("/v1/decisions", post(create))
            .route_layer(middleware::from_fn_with_state(self.clone(), guard));
        Router::new()
            .merge(decisions)
            .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
            .fallback(|| async {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", "接口不存在。")
            })
            .layer(DefaultBodyLimit::max(self.inner.config.max_body_bytes))
            .with_state(self.clone())
    }
    /// 实际监听地址；默认 127.0.0.1:8787。
    pub fn listen(&self) -> std::net::SocketAddr {
        self.inner.config.listen
    }
    async fn execute(
        &self,
        request: DecisionRequest,
        need: crate::protocol::Requirements,
        deadline: Deadline,
    ) -> Result<DecisionResponse, ApiError> {
        let ids = self
            .inner
            .config
            .models
            .get(&request.model)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "model_not_found",
                    "公开模型 alias 不存在。",
                )
            })?;
        let mut compatible_ids = Vec::new();
        let mut usage_blocked = false;
        for id in ids {
            match compatible(&self.inner.backends[id].config, &request, &need) {
                Ok(()) => compatible_ids.push(id.as_str()),
                Err(e) => {
                    usage_blocked |= e.code == "unsupported_usage";
                }
            }
        }
        if compatible_ids.is_empty() {
            return Err(ApiError::unsupported(if usage_blocked {
                "unsupported_usage"
            } else {
                "unsupported_capability"
            }));
        }
        let order = weighted_order(compatible_ids, &self.inner.backends);
        let mut last = ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "upstream_unavailable",
            "兼容上游暂时不可用。",
        );
        for attempt in 0..self.inner.config.max_attempts {
            let Some(remaining) = deadline.0.checked_duration_since(Instant::now()) else {
                return Err(timeout_error());
            };
            let backend = &self.inner.backends[order[attempt % order.len()]];
            let body = request.openai_body(&backend.config.model);
            let timeout =
                Duration::from_millis(self.inner.config.attempt_timeout_ms).min(remaining);
            let result = self
                .inner
                .client
                .post(
                    backend
                        .config
                        .endpoint
                        .as_deref()
                        .unwrap_or(backend.config.kind.endpoint()),
                )
                .bearer_auth(&backend.key)
                .json(&body)
                .timeout(timeout)
                .send()
                .await;
            let wait;
            match result {
                Ok(mut response) => {
                    let status = response.status();
                    if status.is_success() {
                        let mut bytes = Vec::new();
                        if response
                            .content_length()
                            .is_some_and(|n| n > self.inner.config.max_response_bytes as u64)
                        {
                            return Err(ApiError::protocol());
                        }
                        loop {
                            match response.chunk().await {
                                Ok(Some(chunk)) => {
                                    if bytes.len() + chunk.len()
                                        > self.inner.config.max_response_bytes
                                    {
                                        return Err(ApiError::protocol());
                                    }
                                    bytes.extend_from_slice(&chunk);
                                }
                                Ok(None) => break,
                                Err(_) => {
                                    return Err(ApiError::new(
                                        StatusCode::BAD_GATEWAY,
                                        "upstream_body_error",
                                        "上游响应体传输失败；未自动重放请求。",
                                    ));
                                }
                            }
                        }
                        let value: Value =
                            serde_json::from_slice(&bytes).map_err(|_| ApiError::protocol())?;
                        return DecisionResponse::parse(value, &request);
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS {
                        let seconds = retry_after(
                            response
                                .headers()
                                .get(header::RETRY_AFTER)
                                .and_then(|h| h.to_str().ok()),
                            SystemTime::now(),
                        )
                        .unwrap_or(1);
                        last = ApiError::new(
                            StatusCode::TOO_MANY_REQUESTS,
                            "rate_limit_exceeded",
                            "兼容上游限流，请稍后重试。",
                        );
                        last.retry_after = Some(seconds);
                        wait = Duration::from_secs(seconds);
                    } else if [500, 502, 503, 504].contains(&status.as_u16()) {
                        last = ApiError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "upstream_unavailable",
                            "兼容上游暂时不可用。",
                        );
                        wait = backoff(attempt);
                    } else if status.is_client_error() {
                        // 401/403 是服务端配置问题，不误导调用者更换自己的 API key。
                        return Err(ApiError::new(
                            if [401, 403].contains(&status.as_u16()) {
                                StatusCode::BAD_GATEWAY
                            } else {
                                StatusCode::BAD_REQUEST
                            },
                            "upstream_client_error",
                            "上游拒绝请求；未切换模型，请检查协议或服务端配置。",
                        ));
                    } else {
                        return Err(ApiError::new(
                            StatusCode::BAD_GATEWAY,
                            "upstream_http_error",
                            "上游返回不支持的 HTTP 状态；重定向已禁用。",
                        ));
                    }
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    last = if e.is_timeout() {
                        timeout_error()
                    } else {
                        ApiError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "upstream_unavailable",
                            "兼容上游连接失败。",
                        )
                    };
                    wait = backoff(attempt);
                }
                Err(_) => {
                    return Err(ApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "upstream_transport_error",
                        "上游传输失败；未自动重放请求。",
                    ));
                }
            }
            tracing::warn!(
                attempt = attempt + 1,
                error_code = last.code,
                "上游暂时失败"
            );
            if attempt + 1 == self.inner.config.max_attempts {
                break;
            }
            if wait >= deadline.0.saturating_duration_since(Instant::now()) {
                return Err(last);
            }
            tokio::time::sleep(wait).await;
        }
        Err(last)
    }
}
fn backoff(attempt: usize) -> Duration {
    Duration::from_millis((100u64 << attempt.min(4)) + rand::rng().random_range(0..=50))
}
fn timeout_error() -> ApiError {
    ApiError::new(
        StatusCode::GATEWAY_TIMEOUT,
        "upstream_timeout",
        "请求超过网关时间预算。",
    )
}
fn weighted_order<'a>(
    mut ids: Vec<&'a str>,
    backends: &BTreeMap<String, RuntimeBackend>,
) -> Vec<&'a str> {
    let mut ordered = Vec::with_capacity(ids.len());
    let mut rng = rand::rng();
    while !ids.is_empty() {
        let priority = ids
            .iter()
            .map(|id| backends[*id].config.priority)
            .min()
            .unwrap();
        let total: u64 = ids
            .iter()
            .filter(|id| backends[**id].config.priority == priority)
            .map(|id| u64::from(backends[*id].config.weight))
            .sum();
        let mut pick = rng.random_range(0..total);
        let index = ids
            .iter()
            .position(|id| {
                let b = &backends[*id].config;
                if b.priority != priority {
                    return false;
                }
                if pick < u64::from(b.weight) {
                    true
                } else {
                    pick -= u64::from(b.weight);
                    false
                }
            })
            .unwrap();
        ordered.push(ids.remove(index));
    }
    ordered
}
/// 解析 Retry-After 的秒数或 HTTP 日期；过期日期为 0，不截短未来等待时间。
pub fn retry_after(value: Option<&str>, now: SystemTime) -> Option<u64> {
    let value = value?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let duration = date.duration_since(now).unwrap_or_default();
    Some(
        duration
            .as_secs()
            .saturating_add(u64::from(duration.subsec_nanos() != 0)),
    )
}
async fn guard(
    State(state): State<Gateway>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(key) = &state.inner.bearer {
        let headers = request.headers();
        let provided = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        if headers.get_all(header::AUTHORIZATION).iter().count() != 1
            || !provided.is_some_and(|v| bool::from(v.as_bytes().ct_eq(key.as_bytes())))
        {
            let mut response = ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "需要有效的 Bearer 凭据。",
            )
            .into_response();
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, "Bearer".parse().unwrap());
            return response;
        }
    }
    let Ok(_permit) = state.inner.slots.clone().try_acquire_owned() else {
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_busy",
            "网关并发已满，请稍后重试。",
        )
        .into_response();
    };
    let deadline =
        Deadline(Instant::now() + Duration::from_millis(state.inner.config.request_timeout_ms));
    request.extensions_mut().insert(deadline);
    match tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline.0),
        next.run(request),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => timeout_error().into_response(),
    }
}
async fn create(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<DecisionResponse>, ApiError> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        != Some("application/json")
    {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_content_type",
            "请求必须使用 application/json。",
        ));
    }
    let bytes =
        body.map_err(|e| ApiError::new(e.status(), "invalid_body", "请求体超过限制或读取失败。"))?;
    let value = serde_json::from_slice(&bytes).map_err(|_| ApiError::invalid())?;
    let (request, need) = DecisionRequest::parse(value, state.inner.config.max_images)?;
    state.execute(request, need, deadline).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn backend(priority: u32, weight: u32) -> RuntimeBackend {
        RuntimeBackend {
            config: serde_json::from_value(json!({"kind":"openai", "model":"gpt-6-luna", "api_key_env":"KEY", "priority":priority, "weight":weight, "capabilities":{}})).unwrap(),
            key: "offline".into(),
        }
    }
    #[test]
    fn weights_choose_first_candidate_and_priority_groups_do_not_mix() {
        let backends = BTreeMap::from([
            ("heavy".into(), backend(0, 9)),
            ("light".into(), backend(0, 1)),
            ("fallback".into(), backend(1, 100)),
        ]);
        let mut heavy_first = 0;
        for _ in 0..10_000 {
            let order = weighted_order(vec!["fallback", "light", "heavy"], &backends);
            assert_eq!(order.len(), 3);
            assert_eq!(order[2], "fallback");
            assert_ne!(order[0], order[1]);
            heavy_first += usize::from(order[0] == "heavy");
        }
        // 极宽统计界限，避免把采样噪声写成脆弱测试。
        assert!((8500..9500).contains(&heavy_first));
    }
}
