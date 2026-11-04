//! 全部测试只连接临时 loopback mock；不读取真实凭据，不调用公共 API。
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use decora::{
    Config, Gateway,
    adapter::{native_answers, native_body},
    gateway::retry_after,
    protocol::{DecisionRequest, DecisionResponse},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use tower::ServiceExt;

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Value,
    retry: Option<String>,
    location: Option<String>,
    delay_ms: u64,
}
impl Reply {
    fn ok(body: Value) -> Self {
        Self {
            status: StatusCode::OK,
            body,
            retry: None,
            location: None,
            delay_ms: 0,
        }
    }
    fn status(code: u16) -> Self {
        Self {
            status: StatusCode::from_u16(code).unwrap(),
            ..Self::ok(json!({"error": "sk-UPSTREAM-MUST-NOT-LEAK"}))
        }
    }
}
#[derive(Default)]
struct MockState {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<(HeaderMap, Value)>>,
}
struct Mock {
    url: String,
    state: Arc<MockState>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mock {
    async fn new(replies: Vec<Reply>) -> Self {
        async fn handler(
            State(state): State<Arc<MockState>>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> Response {
            state.seen.lock().unwrap().push((headers, body));
            let reply = state
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Reply::status(500));
            tokio::time::sleep(Duration::from_millis(reply.delay_ms)).await;
            let mut response = (reply.status, Json(reply.body)).into_response();
            if let Some(value) = reply.retry {
                response
                    .headers_mut()
                    .insert("retry-after", value.parse().unwrap());
            }
            if let Some(value) = reply.location {
                response
                    .headers_mut()
                    .insert("location", value.parse().unwrap());
            }
            response
        }
        let state = Arc::new(MockState {
            replies: Mutex::new(replies.into()),
            ..Default::default()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/decisions", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/decisions", post(handler))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, state, task }
    }
    fn count(&self) -> usize {
        self.state.seen.lock().unwrap().len()
    }
}
fn usage() -> Value {
    json!({"input_tokens":100,"input_tokens_details":{"cached_tokens":10,"cache_write_tokens":20},"output_tokens":4,"output_tokens_details":{"reasoning_tokens":2},"total_tokens":104})
}
fn predicate() -> Value {
    json!({"model":"decora-test","input":"package damaged","questions":[{"type":"predicate","instructions":"Damaged?"}]})
}
fn success() -> Value {
    json!({"model":"gpt-6-luna","answers":[{"type":"predicate","name":null,"probability":0.8}],"usage":usage()})
}
fn backend(url: &str, priority: u32) -> Value {
    json!({"kind":"openai","model":"gpt-6-luna","api_key_env":"TEST_UPSTREAM_KEY","endpoint":url,"priority":priority,"weight":1,"capabilities":{"question_types":["predicate","choice","score"],"images":true,"messages":true,"json_input":true,"prompt_cache":true,"safety_identifier":true,"max_questions":400}})
}
fn config_value(backends: Value) -> Value {
    let ids: Vec<String> = backends.as_object().unwrap().keys().cloned().collect();
    json!({"allow_http_loopback":true,"bearer_key_env":"TEST_GATEWAY_KEY","max_attempts":3,"attempt_timeout_ms":1000,"request_timeout_ms":3000,"backends":backends,"models":{"decora-test":ids}})
}
fn gateway(value: Value) -> Gateway {
    let cfg = Config::from_yaml(&value.to_string()).unwrap();
    Gateway::with_env(cfg, |name| match name {
        "TEST_UPSTREAM_KEY" => Some("sk-offline-upstream".into()),
        "TEST_GATEWAY_KEY" => Some("offline-client-key".into()),
        _ => None,
    })
    .unwrap()
}
async fn call_raw(
    gateway: &Gateway,
    path: &str,
    body: String,
    auth: Option<&str>,
    content_type: &str,
) -> (StatusCode, HeaderMap, Value) {
    let mut req = axum::http::Request::builder()
        .method(if path == "/healthz" { "GET" } else { "POST" })
        .uri(path)
        .header("content-type", content_type);
    if let Some(auth) = auth {
        req = req.header("authorization", auth);
    }
    let response = gateway
        .router()
        .oneshot(req.body(axum::body::Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)}));
    (status, headers, value)
}
async fn call(gateway: &Gateway, value: Value) -> (StatusCode, HeaderMap, Value) {
    call_raw(
        gateway,
        "/v1/decisions",
        value.to_string(),
        Some("Bearer offline-client-key"),
        "application/json",
    )
    .await
}
fn parse(value: Value) -> DecisionRequest {
    DecisionRequest::parse(value, 32).unwrap().0
}

#[tokio::test]
async fn openai_roundtrip_all_types_preserves_native_probabilities_and_usage() {
    let request = json!({"model":"decora-test","input":"x","questions":[
        {"type":"predicate","name":"duplicate","instructions":"yes?"},
        {"type":"choice","name":"duplicate","instructions":"pick","choices":[{"value":true},{"value":"true","description":"text"}]},
        {"type":"score","instructions":"rate","levels":[{"label":"low"},{"label":"high","description":"urgent"}]}
    ],"prompt_cache_key":"cache-key","prompt_cache_options":{"mode":"explicit","ttl":"30m"},"safety_identifier":"person"});
    let response = json!({"model":"resolved-private-model","answers":[
        {"type":"predicate","name":"duplicate","probability":0.83},
        {"type":"choice","name":"duplicate","choice":true,"confidence":0.61,"probabilities":[{"value":true,"probability":0.7},{"value":"true","probability":0.3}]},
        {"type":"score","name":null,"score":0.6,"confidence":0.4,"probabilities":[{"value":0,"label":"low","probability":0.4},{"value":1,"label":"high","probability":0.6}]}
    ],"usage":usage()});
    let mock = Mock::new(vec![Reply::ok(response.clone())]).await;
    let gw = gateway(config_value(json!({"primary":backend(&mock.url,0)})));
    let (status, _, body) = call(&gw, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model"], "decora-test");
    assert_eq!(body["answers"], response["answers"]);
    assert_eq!(body["usage"], usage());
    let seen = mock.state.seen.lock().unwrap();
    assert_eq!(seen[0].0["authorization"], "Bearer sk-offline-upstream");
    assert_ne!(seen[0].0["authorization"], "Bearer offline-client-key");
    assert_eq!(seen[0].1["model"], "gpt-6-luna");
    assert_eq!(seen[0].1["questions"], request["questions"]);
    assert_eq!(seen[0].1["safety_identifier"], "person");
}

#[tokio::test]
async fn authentication_runs_before_body_and_health_reveals_nothing() {
    let mock = Mock::new(vec![]).await;
    let gw = gateway(config_value(json!({"primary":backend(&mock.url,0)})));
    for auth in [None, Some("Bearer bad"), Some("Basic offline-client-key")] {
        let (s, h, v) = call_raw(
            &gw,
            "/v1/decisions",
            "not json".into(),
            auth,
            "application/json",
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert_eq!(h["www-authenticate"], "Bearer");
        assert_eq!(v["error"]["code"], "invalid_api_key");
        assert!(!v.to_string().contains("sk-offline"));
    }
    let (s, _, v) = call_raw(&gw, "/healthz", String::new(), None, "application/json").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({"status":"ok"}));
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn route_filters_capabilities_before_priority_and_fallback() {
    let first = Mock::new(vec![Reply::status(503)]).await;
    let fallback = Mock::new(vec![Reply::ok(success())]).await;
    let incompatible = Mock::new(vec![]).await;
    let mut no_images = backend(&incompatible.url, 0);
    no_images["capabilities"]["images"] = json!(false);
    let gw = gateway(config_value(
        json!({"incompatible":no_images,"first":backend(&first.url,1),"fallback":backend(&fallback.url,2)}),
    ));
    let request = json!({"model":"decora-test","input":[{"role":"user","content":[{"type":"input_image","image_url":"https://example.com/a.png"}]}],"questions":[{"type":"predicate","instructions":[{"type":"input_text","text":"compare"},{"type":"input_image","image_url":"data:image/png;base64,aGVsbG8="}]}]});
    let (s, _, v) = call(&gw, request.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(first.count(), 1);
    assert_eq!(fallback.count(), 1);
    assert_eq!(incompatible.count(), 0);
    let seen = fallback.state.seen.lock().unwrap();
    assert_eq!(seen[0].1["input"], request["input"]);
    assert_eq!(seen[0].1["questions"], request["questions"]);
}

#[tokio::test]
async fn client_errors_refusals_and_redirects_never_switch_model() {
    for code in [400, 401, 403, 404, 422, 302] {
        let target = Mock::new(vec![Reply::ok(success())]).await;
        let mut reply = Reply::status(code);
        reply.location = Some(target.url.clone());
        let source = Mock::new(vec![reply]).await;
        let gw = gateway(config_value(
            json!({"a":backend(&source.url,0),"b":backend(&target.url,1)}),
        ));
        let (s, _, v) = call(&gw, predicate()).await;
        assert!(s.is_client_error() || s.is_server_error());
        assert_eq!(source.count(), 1);
        assert_eq!(target.count(), 0);
        assert!(!v.to_string().contains("sk-UPSTREAM"));
        assert!(!v.to_string().contains(&target.url));
    }
    let mut refusal = success();
    refusal["answers"][0] = json!({"type":"refusal","name":null});
    let source = Mock::new(vec![Reply::ok(refusal)]).await;
    let target = Mock::new(vec![]).await;
    let gw = gateway(config_value(
        json!({"a":backend(&source.url,0),"b":backend(&target.url,1)}),
    ));
    let (s, _, v) = call(&gw, predicate()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["answers"][0]["type"], "refusal");
    assert_eq!(target.count(), 0);
}

#[tokio::test]
async fn retry_after_is_respected_and_attempts_are_bounded() {
    let mut limited = Reply::status(429);
    limited.retry = Some("0".into());
    let a = Mock::new(vec![limited]).await;
    let b = Mock::new(vec![Reply::ok(success())]).await;
    let gw = gateway(config_value(
        json!({"a":backend(&a.url,0),"b":backend(&b.url,1)}),
    ));
    assert_eq!(call(&gw, predicate()).await.0, StatusCode::OK);
    assert_eq!(a.count(), 1);
    assert_eq!(b.count(), 1);
    let mut limited = Reply::status(429);
    limited.retry = Some("120".into());
    let a = Mock::new(vec![limited]).await;
    let b = Mock::new(vec![]).await;
    let gw = gateway(config_value(
        json!({"a":backend(&a.url,0),"b":backend(&b.url,1)}),
    ));
    let (s, h, v) = call(&gw, predicate()).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(h["retry-after"], "120");
    assert_eq!(v["error"]["code"], "rate_limit_exceeded");
    assert_eq!(b.count(), 0);
    let a = Mock::new(vec![
        Reply::status(503),
        Reply::status(503),
        Reply::status(503),
    ])
    .await;
    let mut cfg = config_value(json!({"a":backend(&a.url,0)}));
    cfg["max_attempts"] = json!(2);
    let gw = gateway(cfg);
    assert_eq!(
        call(&gw, predicate()).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(a.count(), 2);
}

#[tokio::test]
async fn per_attempt_timeout_can_fallback_but_total_deadline_cancels() {
    let mut delayed = Reply::ok(success());
    delayed.delay_ms = 500;
    let a = Mock::new(vec![delayed.clone()]).await;
    let b = Mock::new(vec![Reply::ok(success())]).await;
    let mut cfg = config_value(json!({"a":backend(&a.url,0),"b":backend(&b.url,1)}));
    cfg["attempt_timeout_ms"] = json!(30);
    let gw = gateway(cfg);
    assert_eq!(call(&gw, predicate()).await.0, StatusCode::OK);
    assert_eq!(a.count(), 1);
    assert_eq!(b.count(), 1);
    let a = Mock::new(vec![delayed]).await;
    let mut cfg = config_value(json!({"a":backend(&a.url,0)}));
    cfg["request_timeout_ms"] = json!(50);
    let gw = gateway(cfg);
    assert_eq!(call(&gw, predicate()).await.0, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(a.count(), 1);
}

#[tokio::test]
async fn bad_upstream_probabilities_names_and_usage_are_not_repaired_or_retried() {
    let mut variants = Vec::new();
    for p in [-0.1, 1.1] {
        let mut v = success();
        v["answers"][0]["probability"] = json!(p);
        variants.push(v);
    }
    let mut v = success();
    v["answers"][0].as_object_mut().unwrap().remove("name");
    variants.push(v);
    let mut v = success();
    v["answers"][0]["name"] = json!("wrong");
    variants.push(v);
    let mut v = success();
    v["usage"]["total_tokens"] = json!(1);
    variants.push(v);
    let mut v = success();
    v["usage"]["input_tokens"] = json!(-1);
    variants.push(v);
    let mut v = success();
    v["usage"]
        .as_object_mut()
        .unwrap()
        .remove("input_tokens_details");
    variants.push(v);
    let mut v = success();
    v["usage"]["output_tokens_details"]["reasoning_tokens"] = json!(5);
    variants.push(v);
    let mut v = success();
    v["usage"]["input_tokens_details"]["cached_tokens"] = json!(99);
    variants.push(v);
    let mut v = success();
    v["answers"] = json!([]);
    variants.push(v);
    let mut v = success();
    v["secret"] = json!("sk-UPSTREAM-MUST-NOT-LEAK");
    variants.push(v);
    let count = variants.len();
    let source = Mock::new(variants.into_iter().map(Reply::ok).collect()).await;
    let target = Mock::new(vec![]).await;
    let gw = gateway(config_value(
        json!({"a":backend(&source.url,0),"b":backend(&target.url,1)}),
    ));
    for _ in 0..count {
        let (s, _, v) = call(&gw, predicate()).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY);
        assert_eq!(v["error"]["code"], "upstream_protocol_error");
        assert!(!v.to_string().contains("sk-UPSTREAM"));
    }
    assert_eq!(source.count(), count);
    assert_eq!(target.count(), 0);
}

#[tokio::test]
async fn image_limits_body_limits_unknown_models_and_unsupported_do_not_call_upstream() {
    let source = Mock::new(vec![]).await;
    let mut cfg = config_value(json!({"a":backend(&source.url,0)}));
    cfg["max_body_bytes"] = json!(1024);
    cfg["max_images"] = json!(1);
    let gw = gateway(cfg);
    let mut v = predicate();
    v["model"] = json!("unknown");
    assert_eq!(call(&gw, v).await.0, StatusCode::NOT_FOUND);
    let mut v = predicate();
    v["input"] = json!("x".repeat(2000));
    assert_eq!(call(&gw, v).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    let mut v = predicate();
    v["input"] = json!([{"role":"user","content":[{"type":"input_image","image_url":"https://example.com/a.png"}]}]);
    v["questions"][0]["instructions"] =
        json!([{"type":"input_image","image_url":"https://example.com/b.png"}]);
    let (s, _, v) = call(&gw, v).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "image_limit_exceeded");
    assert_eq!(
        call_raw(
            &gw,
            "/v1/decisions",
            "{}".into(),
            Some("Bearer offline-client-key"),
            "text/plain"
        )
        .await
        .0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        call_raw(
            &gw,
            "/v1/decisions",
            "{".into(),
            Some("Bearer offline-client-key"),
            "application/json"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for kind in ["typesafe", "openrouter"] {
        let mut b = backend(&source.url, 0);
        b["kind"] = json!(kind);
        let gw = gateway(config_value(json!({"native":b})));
        let (s, _, v) = call(&gw, predicate()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"]["code"], "unsupported_usage");
    }
    assert_eq!(source.count(), 0);
}

#[test]
fn request_schema_boundaries_and_multimodal_roles() {
    for count in [1, 400] {
        let mut v = predicate();
        v["questions"] = json!(vec![v["questions"][0].clone(); count]);
        assert!(DecisionRequest::parse(v, 32).is_ok());
    }
    for count in [0, 401] {
        let mut v = predicate();
        v["questions"] = json!(vec![v["questions"][0].clone(); count]);
        assert!(DecisionRequest::parse(v, 32).is_err());
    }
    let image =
        json!({"type":"input_image","image_url":"https://example.com/a.png","detail":"original"});
    let v = json!({"model":"x","input":[{"role":"system","content":[{"type":"input_text","text":"system","prompt_cache_breakpoint":{"mode":"explicit"}}]},{"role":"user","content":[{"type":"input_json","data":{"role":"not-a-role","image_url":"not-an-image"}},image]}],"questions":[{"type":"predicate","instructions":[{"type":"input_json","data":[{"key":1}]},{"type":"input_text","text":"yes?"}]}]});
    let (_, need) = DecisionRequest::parse(v, 32).unwrap();
    assert_eq!(need.images, 1);
    assert!(need.json && need.messages && need.cache);
    let mut invalid = Vec::new();
    let mut v = predicate();
    v["upstream_url"] = json!("https://evil.example");
    invalid.push(v);
    let mut v = predicate();
    v["questions"][0]["name"] = Value::Null;
    invalid.push(v);
    let mut v = predicate();
    v["questions"][0]["instructions"] = json!([]);
    invalid.push(v);
    let mut v = predicate();
    v["questions"][0]["instructions"] =
        json!([{"type":"input_text","text":"x","prompt_cache_breakpoint":{"mode":"explicit"}}]);
    invalid.push(v);
    let mut v = predicate();
    v["input"] = json!([{"role":"developer","content":[{"type":"input_image","image_url":"https://example.com/a"}]}]);
    invalid.push(v);
    let mut v = predicate();
    v["input"] = json!([{"role":"user","content":[{"type":"input_image","file_id":"file-x"}]}]);
    invalid.push(v);
    let mut v = predicate();
    v["input"] =
        json!([{"role":"user","content":[{"type":"input_image","image_url":"file:///secret"}]}]);
    invalid.push(v);
    let mut v = predicate();
    v["input"] = json!([{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,***"}]}]);
    invalid.push(v);
    let mut v = predicate();
    v["input"] = json!([{"type":"function_call","call_id":"a","name":"tool","arguments":"{}"}]);
    invalid.push(v);
    let mut v = predicate();
    v["prompt_cache_options"] = json!({"ttl":"forever"});
    invalid.push(v);
    for v in invalid {
        assert!(DecisionRequest::parse(v.clone(), 32).is_err(), "{v}");
    }
}

#[test]
fn choice_and_score_distribution_validation() {
    let choice = parse(
        json!({"model":"x","input":"x","questions":[{"type":"choice","instructions":"pick","choices":[{"value":"a"},{"value":"b"}]}]}),
    );
    let good = json!({"model":"upstream","answers":[{"type":"choice","name":null,"choice":"a","confidence":0.3,"probabilities":[{"value":"a","probability":0.7},{"value":"b","probability":0.3}]}],"usage":usage()});
    assert!(DecisionResponse::parse(good.clone(), &choice).is_ok());
    for (key, value) in [
        ("choice", json!("b")),
        ("confidence", json!(1.5)),
        (
            "probabilities",
            json!([{"value":"a","probability":0.7},{"value":"a","probability":0.3}]),
        ),
        (
            "probabilities",
            json!([{"value":"a","probability":0.7},{"value":"b","probability":0.2}]),
        ),
    ] {
        let mut v = good.clone();
        v["answers"][0][key] = value;
        assert!(DecisionResponse::parse(v, &choice).is_err());
    }
    let score = parse(
        json!({"model":"x","input":"x","questions":[{"type":"score","instructions":"rate","levels":[{"label":"low"},{"label":"high"}]}]}),
    );
    let good = json!({"model":"upstream","answers":[{"type":"score","name":null,"score":0.8,"confidence":0.5,"probabilities":[{"value":0,"label":"low","probability":0.2},{"value":1,"label":"high","probability":0.8}]}],"usage":usage()});
    assert!(DecisionResponse::parse(good.clone(), &score).is_ok());
    let mut v = good.clone();
    v["answers"][0]["score"] = json!(0.7);
    assert!(DecisionResponse::parse(v, &score).is_err());
    let mut v = good;
    v["answers"][0]["probabilities"][1]["label"] = json!("wrong");
    assert!(DecisionResponse::parse(v, &score).is_err());
    let duplicate = json!({"model":"x","input":"x","questions":[{"type":"choice","instructions":"pick","choices":[{"value":true},{"value":true}]}]});
    assert!(DecisionRequest::parse(duplicate, 32).is_err());
}

#[test]
fn official_native_schema_mapping_is_not_chat_completion_or_fabricated_usage() {
    let req = parse(
        json!({"model":"x","input":"ticket","questions":[{"type":"predicate","name":"same","instructions":"billing?"},{"type":"choice","name":"same","instructions":"team?","choices":[{"value":"a","description":"Team A"},{"value":"b"}]},{"type":"score","instructions":"urgency","levels":[{"label":"low"},{"label":"high","description":"urgent"}]}]}),
    );
    let body = native_body(&req, "jev-latest").unwrap();
    assert_eq!(body["state"], "ticket");
    assert!(body.get("messages").is_none());
    assert_eq!(body["questions"]["q0"]["type"], "noul");
    assert_eq!(
        body["questions"]["q1"]["criteria"],
        json!({"a":"Team A","b":null})
    );
    assert_eq!(
        body["questions"]["q2"]["criteria"],
        json!(["low",{"label":"high","description":"urgent"}])
    );
    let upstream = json!({"answers":{"q2":{"type":"score","score":0.8,"confidence":0.5,"legend":{"0":"low","1":{"label":"high","description":"urgent"}},"probabilities":{"0":0.2,"1":0.8}},"q0":{"type":"noul","noul":0.9},"q1":{"type":"choice","choice":"a","confidence":0.3,"probabilities":{"a":0.7,"b":0.3}}},"usage":{"input_tokens":100,"output_tokens":4}});
    let answers = native_answers(&upstream, &req).unwrap();
    let serialized = serde_json::to_value(&answers).unwrap();
    assert_eq!(serialized[0]["name"], "same");
    assert_eq!(serialized[0]["probability"], 0.9);
    assert_eq!(serialized[2]["probabilities"][1]["label"], "high");
    // 测试借助独立、已知的完整 usage 检验答案；native usage 本身确实不能通过合同。
    assert!(
        DecisionResponse::parse(
            json!({"model":"jev","answers":serialized,"usage":usage()}),
            &req
        )
        .is_ok()
    );
    assert!(
        DecisionResponse::parse(
            json!({"model":"jev","answers":answers,"usage":upstream["usage"]}),
            &req
        )
        .is_err()
    );
    let mut missing = upstream.clone();
    missing["answers"]["q1"]
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    assert!(native_answers(&missing, &req).is_err());
    let mut refusal = upstream;
    refusal["answers"]["q0"] = json!({"type":"refusal"});
    assert!(native_answers(&refusal, &req).is_err());
    let req = parse(
        json!({"model":"x","input":"x","questions":[{"type":"choice","instructions":"pick","choices":[{"value":true},{"value":"true"}]}]}),
    );
    assert!(native_body(&req, "jev").is_err());
    let req = parse(
        json!({"model":"x","input":[{"role":"user","content":"x"}],"questions":[{"type":"predicate","instructions":"x"}]}),
    );
    assert!(native_body(&req, "jev").is_err());
}

#[test]
fn config_rejects_open_proxy_unbounded_or_plaintext_secret_settings() {
    let base = config_value(json!({"a":backend("http://127.0.0.1:1234/decisions",0)}));
    assert!(Config::from_yaml(&base.to_string()).is_ok());
    let mut invalid = Vec::new();
    let mut v = base.clone();
    v["listen"] = json!("0.0.0.0:8787");
    invalid.push(v);
    let mut v = base.clone();
    v["listen"] = json!("0.0.0.0:8787");
    v["allow_public_bind"] = json!(true);
    v.as_object_mut().unwrap().remove("bearer_key_env");
    invalid.push(v);
    let mut v = base.clone();
    v["backends"]["a"]["api_key"] = json!("sk-DO-NOT-INLINE");
    invalid.push(v);
    let mut v = base.clone();
    v["max_attempts"] = json!(0);
    invalid.push(v);
    let mut v = base.clone();
    v["max_images"] = json!(1501);
    invalid.push(v);
    let mut v = base.clone();
    v["backends"]["a"]["weight"] = json!(0);
    invalid.push(v);
    for endpoint in [
        "http://example.com/decisions",
        "https://user:password@example.com/decisions",
        "https://example.com/decisions?key=x",
        "https://example.com/decisions#secret",
    ] {
        let mut v = base.clone();
        v["backends"]["a"]["endpoint"] = json!(endpoint);
        invalid.push(v);
    }
    for v in invalid {
        assert!(Config::from_yaml(&v.to_string()).is_err(), "{v}");
    }
    let cfg = Config::from_yaml(&base.to_string()).unwrap();
    assert!(Gateway::with_env(cfg, |_| None).is_err());
    let cfg = Config::from_yaml(&base.to_string()).unwrap();
    assert!(Gateway::with_env(cfg, |_| Some("line\nbreak".into())).is_err());
    let cfg = Config::from_yaml(include_str!("../config.example.yaml")).unwrap();
    assert_eq!(cfg.listen.ip().to_string(), "127.0.0.1");
}

#[tokio::test]
async fn retry_budget_includes_time_spent_receiving_slow_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut limited = Reply::status(429);
    limited.retry = Some("1".into());
    let upstream = Mock::new(vec![limited]).await;
    let mut cfg = config_value(json!({"a":backend(&upstream.url,0)}));
    cfg["request_timeout_ms"] = json!(1400);
    let gw = gateway(cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, gw.router()).await.unwrap();
    });
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = predicate().to_string();
    let headers = format!(
        "POST /v1/decisions HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer offline-client-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await.unwrap();
    stream
        .write_all(&body.as_bytes()[..body.len() - 1])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    stream
        .write_all(&body.as_bytes()[body.len() - 1..])
        .await
        .unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 429"), "{response}");
    assert!(response.contains("rate_limit_exceeded"));
    assert_eq!(upstream.count(), 1);
    task.abort();
}

#[tokio::test]
async fn real_http_ingress_and_response_size_limit() {
    let upstream = Mock::new(vec![Reply::ok(success())]).await;
    let gw = gateway(config_value(json!({"a":backend(&upstream.url,0)})));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/decisions", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, gw.router()).await.unwrap();
    });
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth("offline-client-key")
        .json(&predicate())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["model"],
        "decora-test"
    );
    task.abort();
    let mut large = success();
    large["model"] = json!("x".repeat(2000));
    let upstream = Mock::new(vec![Reply::ok(large)]).await;
    let mut cfg = config_value(json!({"a":backend(&upstream.url,0)}));
    cfg["max_response_bytes"] = json!(1024);
    let (status, _, body) = call(&gateway(cfg), predicate()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"]["code"], "upstream_protocol_error");
    assert_eq!(upstream.count(), 1);
}

#[tokio::test]
async fn concurrent_requests_have_an_explicit_capacity_limit() {
    let mut delayed = Reply::ok(success());
    delayed.delay_ms = 150;
    let upstream = Mock::new(vec![delayed]).await;
    let mut cfg = config_value(json!({"a":backend(&upstream.url,0)}));
    cfg["max_in_flight"] = json!(1);
    let gw = gateway(cfg);
    let first_gw = gw.clone();
    let first = tokio::spawn(async move { call(&first_gw, predicate()).await });
    // 等待 mock 明确收到请求，而不是假定任务调度顺序。
    tokio::time::timeout(Duration::from_secs(1), async {
        while upstream.count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (status, _, body) = call(&gw, predicate()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "gateway_busy");
    assert_eq!(first.await.unwrap().0, StatusCode::OK);
    assert_eq!(upstream.count(), 1);
}

#[test]
fn json_part_limits_and_assistant_mixed_content_follow_official_openapi() {
    let mut req = predicate();
    req["input"] = json!([{"role":"assistant","phase":null,"content":[{"type":"output_text","text":"prior"},{"type":"refusal","refusal":"declined"}]}]);
    assert!(DecisionRequest::parse(req, 32).is_ok());
    let with_data = |data: Value| json!({"model":"x","input":[{"role":"user","content":[{"type":"input_json","data":data}]}],"questions":[{"type":"predicate","instructions":"yes?"}]});
    let nested = |count: usize| {
        let mut value = json!("leaf");
        for _ in 0..count {
            value = json!({"x":value});
        }
        value
    };
    assert!(DecisionRequest::parse(with_data(nested(32)), 32).is_ok());
    assert!(DecisionRequest::parse(with_data(nested(33)), 32).is_err());
    assert!(DecisionRequest::parse(with_data(json!({"values":vec![0;10_000]})), 32).is_err());
    assert!(DecisionRequest::parse(with_data(json!({"x":"x".repeat(256*1024)})), 32).is_err());
    assert!(DecisionRequest::parse(with_data(json!([1, 2])), 32).is_err());
}

#[tokio::test]
async fn no_compatible_candidate_is_contacted_and_mixed_refusal_is_preserved() {
    let upstream = Mock::new(vec![]).await;
    let mut b = backend(&upstream.url, 0);
    b["capabilities"]["question_types"] = json!(["score"]);
    let (s, _, v) = call(&gateway(config_value(json!({"a":b}))), predicate()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "unsupported_capability");
    assert_eq!(upstream.count(), 0);
    let mut request = predicate();
    request["questions"] = json!([{"type":"predicate","name":"one","instructions":"q1"},{"type":"predicate","name":"two","instructions":"q2"}]);
    let response = json!({"model":"upstream","answers":[{"type":"refusal","name":"one"},{"type":"predicate","name":"two","probability":0.7}],"usage":usage()});
    let upstream = Mock::new(vec![Reply::ok(response.clone())]).await;
    let (s, _, v) = call(
        &gateway(config_value(json!({"a":backend(&upstream.url,0)}))),
        request,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["answers"], response["answers"]);
    assert_eq!(upstream.count(), 1);
}

#[test]
fn retry_after_supports_integer_http_date_and_past_date() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    assert_eq!(retry_after(Some("7"), now), Some(7));
    assert_eq!(
        retry_after(
            Some(&httpdate::fmt_http_date(now + Duration::from_secs(10))),
            now
        ),
        Some(10)
    );
    assert_eq!(
        retry_after(
            Some(&httpdate::fmt_http_date(now - Duration::from_secs(10))),
            now
        ),
        Some(0)
    );
    assert_eq!(retry_after(Some("nonsense"), now), None);
    assert_eq!(retry_after(None, now), None);
}
