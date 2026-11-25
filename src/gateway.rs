//! HTTP service, connection reuse, capability-filtered weighted-priority routing, and bounded failure fallback.
use crate::{
    adapter::{compatible_for_dialect, native_body_for, native_response},
    config::{Backend, BackendKind, Config, resolve_secret},
    dialect::Dialect,
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
/// Cheaply cloneable service state; all requests share one reqwest connection pool.
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
    /// Resolve configuration using environment variables and build a connection pool that verifies TLS and disables redirects.
    pub fn new(config: Config) -> Result<Self, String> {
        Self::with_env(config, |name| std::env::var(name).ok())
    }
    /// Accept an injectable credential lookup; offline tests need neither real keys nor changes to the global environment.
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
            .map_err(|_| "failed to initialise the HTTP TLS client".to_owned())?;
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
    /// Build a Router with multi-dialect Decisions endpoints and a minimal health check; authentication occurs before the body is read.
    pub fn router(&self) -> Router {
        let decisions = Router::new()
            .route(
                "/v1/decisions",
                post(|state, headers, deadline, body| {
                    create(Dialect::Openai, state, headers, deadline, body)
                }),
            )
            .route(
                "/v1beta/decisions",
                post(|state, headers, deadline, body| {
                    create(Dialect::Gemini, state, headers, deadline, body)
                }),
            )
            .route(
                "/v0/decisions",
                post(|state, headers, deadline, body| {
                    create(Dialect::Typesafe, state, headers, deadline, body)
                }),
            )
            .route(
                "/api/v1/decisions",
                post(|state, headers, deadline, body| {
                    create(Dialect::Openrouter, state, headers, deadline, body)
                }),
            )
            .route_layer(middleware::from_fn_with_state(self.clone(), guard));
        Router::new()
            .merge(decisions)
            .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
            .fallback(|| async {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", "Endpoint not found.")
            })
            .layer(DefaultBodyLimit::max(self.inner.config.max_body_bytes))
            .with_state(self.clone())
    }
    /// Actual listening address; defaults to 127.0.0.1:8787.
    pub fn listen(&self) -> std::net::SocketAddr {
        self.inner.config.listen
    }
    async fn execute(
        &self,
        dialect: Dialect,
        raw_request: Value,
        request: DecisionRequest,
        need: crate::protocol::Requirements,
        deadline: Deadline,
    ) -> Result<Value, ApiError> {
        let ids = self
            .inner
            .config
            .models
            .get(&request.model)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "model_not_found",
                    "Unknown public model alias.",
                )
            })?;
        let mut compatible_ids = Vec::new();
        let mut usage_blocked = false;
        let mut incompatible_code = "unsupported_capability";
        for id in ids {
            match compatible_for_dialect(
                dialect,
                &self.inner.backends[id].config,
                &request,
                &need,
                self.inner.config.usage_policy,
            ) {
                Ok(()) => compatible_ids.push(id.as_str()),
                Err(e) => {
                    usage_blocked |= e.code == "unsupported_usage";
                    if e.code != "unsupported_capability" {
                        incompatible_code = e.code;
                    }
                }
            }
        }
        if compatible_ids.is_empty() {
            return Err(ApiError::unsupported(if usage_blocked {
                "unsupported_usage"
            } else {
                incompatible_code
            }));
        }
        let order = weighted_order(compatible_ids, &self.inner.backends);
        let mut last = ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "upstream_unavailable",
            "No compatible upstream is available right now.",
        );
        for attempt in 0..self.inner.config.max_attempts {
            let Some(remaining) = deadline.0.checked_duration_since(Instant::now()) else {
                return Err(timeout_error());
            };
            let backend = &self.inner.backends[order[attempt % order.len()]];
            let passthrough = dialect.matches_backend(backend.config.kind);
            let body = if passthrough {
                dialect.passthrough_body(&raw_request, &request, &backend.config.model)
            } else {
                match backend.config.kind {
                    BackendKind::Openai => request.openai_body(&backend.config.model),
                    kind => native_body_for(&request, &backend.config.model, kind)?,
                }
            };
            let timeout =
                Duration::from_millis(self.inner.config.attempt_timeout_ms).min(remaining);
            let mut req_builder = self.inner.client.post(
                backend
                    .config
                    .endpoint
                    .as_deref()
                    .unwrap_or(backend.config.kind.endpoint()),
            );
            if backend.config.kind == BackendKind::Gemini {
                req_builder = req_builder.header("x-goog-api-key", &backend.key);
            } else {
                req_builder = req_builder.bearer_auth(&backend.key);
            }
            let result = req_builder.json(&body).timeout(timeout).send().await;
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
                                        "Upstream response body transfer failed; the request was not replayed.",
                                    ));
                                }
                            }
                        }
                        let value: Value =
                            serde_json::from_slice(&bytes).map_err(|_| ApiError::protocol())?;
                        if passthrough {
                            return dialect.passthrough_response(&raw_request, &request, value);
                        }
                        let canonical = match backend.config.kind {
                            // OpenAI officially guarantees complete usage; do not disguise missing fields as compatible mode.
                            BackendKind::Openai => DecisionResponse::parse(value, &request)?,
                            kind => native_response(
                                value,
                                &request,
                                kind,
                                self.inner.config.usage_policy,
                            )?,
                        };
                        return dialect.format_response(&request, canonical);
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
                            "Compatible upstreams are rate limited; retry later.",
                        );
                        last.retry_after = Some(seconds);
                        wait = Duration::from_secs(seconds);
                    } else if [500, 502, 503, 504].contains(&status.as_u16()) {
                        last = ApiError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "upstream_unavailable",
                            "No compatible upstream is available right now.",
                        );
                        wait = backoff(attempt);
                    } else if status.is_client_error() {
                        // 401/403 indicate a server configuration problem; do not mislead callers into changing their own API key.
                        return Err(ApiError::new(
                            if [401, 403].contains(&status.as_u16()) {
                                StatusCode::BAD_GATEWAY
                            } else {
                                StatusCode::BAD_REQUEST
                            },
                            "upstream_client_error",
                            "Upstream rejected the request; no fallback was attempted. Check the protocol or server configuration.",
                        ));
                    } else {
                        return Err(ApiError::new(
                            StatusCode::BAD_GATEWAY,
                            "upstream_http_error",
                            "Upstream returned an unsupported HTTP status; redirects are disabled.",
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
                            "Could not connect to a compatible upstream.",
                        )
                    };
                    wait = backoff(attempt);
                }
                Err(_) => {
                    return Err(ApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "upstream_transport_error",
                        "Upstream transfer failed; the request was not replayed.",
                    ));
                }
            }
            tracing::warn!(
                attempt = attempt + 1,
                error_code = last.code,
                "upstream temporarily failed"
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
        "Request exceeded the gateway time budget.",
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
/// Parse Retry-After as seconds or an HTTP date; past dates yield 0, and future wait times are not truncated.
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
fn valid_client_key(headers: &HeaderMap, expected: &str) -> bool {
    let auth_count = headers.get_all(header::AUTHORIZATION).iter().count();
    let goog_count = headers.get_all("x-goog-api-key").iter().count();
    let provided = match (auth_count, goog_count) {
        (1, 0) => headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer ")),
        (0, 1) => headers.get("x-goog-api-key").and_then(|h| h.to_str().ok()),
        _ => None,
    };
    provided.is_some_and(|v| bool::from(v.as_bytes().ct_eq(expected.as_bytes())))
}

async fn guard(
    State(state): State<Gateway>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(key) = &state.inner.bearer
        && !valid_client_key(request.headers(), key)
    {
        let mut response = ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "A valid Bearer or x-goog-api-key credential is required.",
        )
        .into_response();
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, "Bearer".parse().unwrap());
        return response;
    }
    let Ok(_permit) = state.inner.slots.clone().try_acquire_owned() else {
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_busy",
            "Gateway is at its concurrency limit; retry later.",
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
    dialect: Dialect,
    State(state): State<Gateway>,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
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
            "Request must use application/json.",
        ));
    }
    let bytes = body.map_err(|e| {
        ApiError::new(
            e.status(),
            "invalid_body",
            "Request body exceeded the limit or could not be read.",
        )
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ApiError::invalid())?;
    let (request, need) = dialect.parse_request(value.clone(), state.inner.config.max_images)?;
    state
        .execute(dialect, value, request, need, deadline)
        .await
        .map(Json)
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
        // Use a wide statistical interval to avoid making the test brittle to sampling noise.
        assert!((8500..9500).contains(&heavy_first));
    }
}
