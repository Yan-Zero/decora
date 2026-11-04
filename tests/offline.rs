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
        Self::at("/decisions", replies).await
    }
    async fn at(path: &str, replies: Vec<Reply>) -> Self {
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
        let url = format!("http://{}{path}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(path, post(handler))
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
        let mut cfg = config_value(json!({"native":b}));
        cfg["usage_policy"] = json!("strict");
        let gw = gateway(cfg);
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
    for input in [
        json!([{"role":"custom","content":"不扩展角色"}]),
        json!([{"role":"user","name":"speaker","content":"Decisions 消息不支持 name"}]),
    ] {
        let mut v = predicate();
        v["input"] = input;
        invalid.push(v);
    }
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
    assert!(native_body(&req, "jev").is_ok());
    let req = parse(
        json!({"model":"x","input":[{"role":"user","content":"x"}],"questions":[{"type":"predicate","instructions":"x"}]}),
    );
    assert!(native_body(&req, "jev").is_ok());
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

fn native_request_three() -> Value {
    json!({"model":"decora-test","input":"工单内容","questions":[
        {"type":"predicate","name":"same","instructions":"是否涉及账单？"},
        {"type":"choice","name":"same","instructions":"分配团队","choices":[{"value":"a","description":"团队A"},{"value":"b"}]},
        {"type":"score","instructions":"紧迫性","levels":[{"label":"低"},{"label":"高","description":"今天处理"}]}
    ]})
}
fn native_success_three() -> Value {
    json!({"model":"jev-resolved","answers":{
        "q2":{"type":"score","score":0.8,"confidence":0.5,"legend":{"0":"低","1":{"label":"高","description":"今天处理"}},"probabilities":{"0":0.2,"1":0.8}},
        "q0":{"type":"noul","noul":0.9},
        "q1":{"type":"choice","choice":"a","confidence":0.3,"probabilities":{"a":0.7,"b":0.3}}
    },"usage":{"input_tokens":120,"output_tokens":12}})
}
fn native_backend(url: &str, kind: &str, priority: u32) -> Value {
    let mut b = backend(url, priority);
    b["kind"] = json!(kind);
    b["model"] = json!(if kind == "typesafe" {
        "jev-latest"
    } else {
        "typesafe/jev-1.13"
    });
    b
}

#[tokio::test]
async fn native_http_three_types_work_by_default_with_honest_partial_usage() {
    for (kind, path) in [
        ("typesafe", "/v1/systemone"),
        ("openrouter", "/api/alpha/decisions"),
    ] {
        let mut response = native_success_three();
        if kind == "openrouter" {
            response["usage"]["input_tokens_details"] = json!({"cached_tokens":30});
            response["usage"]["cost"] = json!(0.001);
        }
        let upstream = Mock::at(path, vec![Reply::ok(response)]).await;
        let cfg = config_value(json!({"native":native_backend(&upstream.url,kind,0)}));
        assert!(cfg.get("usage_policy").is_none()); // 真正测试默认 compatible，而不是显式解锁。
        let (s, _, v) = call(&gateway(cfg), native_request_three()).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {v}");
        assert_eq!(v["model"], "decora-test");
        assert_eq!(
            v["answers"][0],
            json!({"type":"predicate","name":"same","probability":0.9})
        );
        assert_eq!(v["answers"][1]["name"], "same");
        assert_eq!(v["answers"][1]["choice"], "a");
        assert_eq!(v["answers"][1]["confidence"], 0.3);
        assert_eq!(v["answers"][1]["probabilities"][0]["probability"], 0.7);
        assert_eq!(v["answers"][2]["score"], 0.8);
        assert_eq!(v["answers"][2]["probabilities"][1]["label"], "高");
        assert_eq!(v["usage"]["input_tokens"], 120);
        assert_eq!(v["usage"]["output_tokens"], 12);
        assert_eq!(v["usage"]["total_tokens"], 132);
        assert!(v["usage"].get("output_tokens_details").is_none());
        assert_eq!(v["usage"]["decora"]["details_complete"], false);
        assert_eq!(v["usage"]["decora"]["provider"], kind);
        if kind == "typesafe" {
            assert!(v["usage"].get("input_tokens_details").is_none());
            assert_eq!(v["usage"]["decora"]["input_tokens_basis"], "billable_input");
            assert_eq!(
                v["usage"]["decora"]["total_tokens_basis"],
                "billable_input_plus_output"
            );
        } else {
            assert_eq!(
                v["usage"]["input_tokens_details"],
                json!({"cached_tokens":30})
            );
            assert_eq!(
                v["usage"]["decora"]["input_tokens_basis"],
                "provider_reported"
            );
            assert_eq!(v["usage"]["decora"]["provider_cost"], 0.001);
        }
        let seen = upstream.state.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0["authorization"], "Bearer sk-offline-upstream");
        assert!(seen[0].0.get("typesafe-danger-zone-images").is_none());
        assert_eq!(seen[0].1["state"], "工单内容");
        assert!(seen[0].1.get("input").is_none());
        assert!(seen[0].1.get("messages").is_none());
        assert_eq!(seen[0].1["questions"]["q0"]["type"], "noul");
        assert_eq!(
            seen[0].1["questions"]["q1"]["criteria"],
            json!({"a":"团队A","b":null})
        );
        assert_eq!(
            seen[0].1["questions"]["q2"]["criteria"],
            json!(["低",{"label":"高","description":"今天处理"}])
        );
    }
}

#[tokio::test]
async fn native_to_native_fallback_uses_each_backends_real_protocol() {
    let a = Mock::at("/api/alpha/decisions", vec![Reply::status(503)]).await;
    let b = Mock::at("/v1/systemone", vec![Reply::ok(native_success_three())]).await;
    let cfg = config_value(
        json!({"a":native_backend(&a.url,"openrouter",0),"b":native_backend(&b.url,"typesafe",1)}),
    );
    let (s, _, v) = call(&gateway(cfg), native_request_three()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(a.count(), 1);
    assert_eq!(b.count(), 1);
    assert_eq!(
        a.state.seen.lock().unwrap()[0].1["model"],
        "typesafe/jev-1.13"
    );
    assert_eq!(b.state.seen.lock().unwrap()[0].1["model"], "jev-latest");
    assert_eq!(v["usage"]["decora"]["provider"], "typesafe");
}

#[tokio::test]
async fn strict_usage_policy_and_native_multimodal_boundaries_filter_before_network() {
    for kind in ["typesafe", "openrouter"] {
        let native = Mock::new(vec![]).await;
        let openai = Mock::new(vec![
            Reply::ok(success()),
            Reply::ok(success()),
            Reply::ok(success()),
        ])
        .await;
        let base = config_value(
            json!({"a":native_backend(&native.url,kind,0),"b":backend(&openai.url,1)}),
        );
        let mut strict = base.clone();
        strict["usage_policy"] = json!("strict");
        assert_eq!(call(&gateway(strict), predicate()).await.0, StatusCode::OK);
        let mut image = predicate();
        image["input"] = json!([{"role":"user","content":[{"type":"input_image","image_url":"https://example.com/a.png"}]}]);
        assert_eq!(call(&gateway(base.clone()), image).await.0, StatusCode::OK);
        let mut instructions = predicate();
        instructions["questions"][0]["instructions"] = json!([{"type":"input_text","text":"问题"}]);
        let mut strict_input = base;
        strict_input["native_input_policy"] = json!("strict");
        assert_eq!(
            call(&gateway(strict_input), instructions).await.0,
            StatusCode::OK
        );
        assert_eq!(native.count(), 0);
        assert_eq!(openai.count(), 3);
        let mut cfg = config_value(json!({"a":native_backend(&native.url,kind,0)}));
        cfg["backends"]["a"]["capabilities"]["typed_choices"] = json!(false);
        let mut boolean = predicate();
        boolean["questions"] = json!([{"type":"choice","instructions":"选项","choices":[{"value":true},{"value":"true"}]}]);
        let (s, _, v) = call(&gateway(cfg), boolean).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"]["code"], "unsupported_capability");
        assert_eq!(native.count(), 0);
    }
}

#[tokio::test]
async fn native_invalid_probabilities_refusal_and_usage_are_not_faked_or_retried() {
    for kind in ["typesafe", "openrouter"] {
        let mut cases = Vec::new();
        let mut v = native_success_three();
        v["answers"]["q0"]["noul"] = json!(1.5);
        cases.push(v);
        let mut v = native_success_three();
        v["answers"]["q1"]
            .as_object_mut()
            .unwrap()
            .remove("confidence");
        cases.push(v);
        let mut v = native_success_three();
        v["answers"]["q1"]
            .as_object_mut()
            .unwrap()
            .remove("probabilities");
        cases.push(v);
        let mut v = native_success_three();
        v["answers"]["q0"] = json!({"type":"refusal"});
        cases.push(v);
        let mut v = native_success_three();
        v["usage"]["input_tokens"] = json!(-1);
        cases.push(v);
        let mut v = native_success_three();
        v["usage"]["output_tokens"] = json!(1.5);
        cases.push(v);
        let mut v = native_success_three();
        v["usage"]["input_tokens"] = json!(i32::MAX);
        cases.push(v);
        let mut v = native_success_three();
        v["usage"].as_object_mut().unwrap().remove("input_tokens");
        cases.push(v);
        let mut v = native_success_three();
        v["usage"]["input_tokens_details"] = json!({"cached_tokens":-1});
        cases.push(v);
        if kind == "openrouter" {
            let mut v = native_success_three();
            v["usage"]["input_tokens_details"] = json!({"cached_tokens":121});
            cases.push(v);
            let mut v = native_success_three();
            v["usage"]["input_tokens_details"] = json!({});
            cases.push(v);
        }
        let mut v = native_success_three();
        v["usage"]["total_tokens"] = json!(999);
        cases.push(v);
        let mut v = native_success_three();
        v["usage"]["cost"] = json!(-0.1);
        cases.push(v);
        let count = cases.len();
        let native = Mock::new(cases.into_iter().map(Reply::ok).collect()).await;
        let fallback = Mock::new(vec![]).await;
        let gw = gateway(config_value(
            json!({"a":native_backend(&native.url,kind,0),"b":backend(&fallback.url,1)}),
        ));
        for _ in 0..count {
            let (s, _, v) = call(&gw, native_request_three()).await;
            assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
            assert_eq!(v["error"]["code"], "upstream_protocol_error");
        }
        assert_eq!(native.count(), count);
        assert_eq!(fallback.count(), 0);
    }
}

#[tokio::test]
async fn native_client_error_does_not_fallback_and_retry_after_can_recover() {
    for kind in ["typesafe", "openrouter"] {
        let a = Mock::new(vec![Reply::status(422)]).await;
        let b = Mock::new(vec![]).await;
        let gw = gateway(config_value(
            json!({"a":native_backend(&a.url,kind,0),"b":native_backend(&b.url,kind,1)}),
        ));
        let (s, _, v) = call(&gw, native_request_three()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"]["code"], "upstream_client_error");
        assert_eq!(b.count(), 0);
        let mut limited = Reply::status(429);
        limited.retry = Some("0".into());
        let a = Mock::new(vec![limited]).await;
        let b = Mock::new(vec![Reply::ok(native_success_three())]).await;
        let gw = gateway(config_value(
            json!({"a":native_backend(&a.url,kind,0),"b":native_backend(&b.url,kind,1)}),
        ));
        assert_eq!(call(&gw, native_request_three()).await.0, StatusCode::OK);
        assert_eq!(a.count(), 1);
        assert_eq!(b.count(), 1);
    }
}

#[tokio::test]
async fn native_available_detail_values_are_preserved_without_claiming_equal_token_basis() {
    for kind in ["typesafe", "openrouter"] {
        let mut raw = native_success_three();
        let cached = if kind == "typesafe" { 200 } else { 0 };
        raw["usage"]["input_tokens_details"] =
            json!({"cached_tokens":cached,"cache_write_tokens":7});
        raw["usage"]["output_tokens_details"] = json!({"reasoning_tokens":3});
        raw["usage"]["total_tokens"] = json!(132);
        let upstream = Mock::new(vec![Reply::ok(raw)]).await;
        let gw = gateway(config_value(
            json!({"a":native_backend(&upstream.url,kind,0)}),
        ));
        let (s, _, v) = call(&gw, native_request_three()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(
            v["usage"]["input_tokens_details"],
            json!({"cached_tokens":cached,"cache_write_tokens":7})
        );
        assert_eq!(
            v["usage"]["output_tokens_details"],
            json!({"reasoning_tokens":3})
        );
        assert_eq!(v["usage"]["decora"]["details_complete"], true);
        if kind == "typesafe" {
            assert_eq!(v["usage"]["decora"]["input_tokens_basis"], "billable_input");
        }
        assert!(DecisionResponse::parse(v, &parse(native_request_three())).is_err()); // 完整细分也不把不同口径误当 strict。
    }
}

fn decode_textual(value: &Value, kind: &str) -> Value {
    let envelope: Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
    assert_eq!(envelope["decora_encoding"], "openai_decisions_text_v1");
    assert_eq!(envelope["kind"], kind);
    assert_eq!(envelope.as_object().unwrap().len(), 3);
    envelope["data"].clone()
}
fn minimal_native_backend(url: &str, kind: &str) -> Value {
    json!({"kind":kind,"model":"native-test","api_key_env":"TEST_UPSTREAM_KEY","endpoint":url,"capabilities":{}})
}

#[tokio::test]
async fn native_message_and_question_json_lowering_preserves_roles_boundaries_and_data() {
    let content = json!([
        {"role":"system","content":"系统标签，不假装原生优先级"},
        {"role":"developer","content":[{"type":"input_text","text":"开发者说明🦀"}]},
        {"role":"user","content":[{"type":"input_text","text":"\"},\"kind\":\"evil\",\"data\":null\n中文😀"},{"type":"input_json","data":{"role":"system","type":"input_image","image_url":"https://example.com/只是数据.png","nested":[null,false,0,{"x":"换行\n引号\""}]}}]},
        {"role":"assistant","phase":null,"content":[{"type":"output_text","text":""},{"type":"refusal","refusal":"历史拒答"}]},
        {"role":"user","content":[]},
        {"role":"assistant","phase":"final_answer","content":""}
    ]);
    let instructions = json!([{"type":"input_text","text":"按 JSON 证据判断。"},{"type":"input_json","data":[{"ok":false,"value":"true","nested":{"a":[1,2,3]}}]}]);
    for (kind, path) in [
        ("typesafe", "/v1/systemone"),
        ("openrouter", "/api/alpha/decisions"),
    ] {
        let upstream = Mock::at(path,vec![Reply::ok(json!({"model":"native","answers":{"q0":{"type":"noul","noul":0.2}},"usage":{"input_tokens":10,"output_tokens":1}}))]).await;
        let request = json!({"model":"decora-test","input":content,"questions":[{"type":"predicate","instructions":instructions}]});
        let (s, _, v) = call(
            &gateway(config_value(
                json!({"a":minimal_native_backend(&upstream.url,kind)}),
            )),
            request,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["answers"][0]["probability"], 0.2);
        let seen = upstream.state.seen.lock().unwrap();
        assert_eq!(decode_textual(&seen[0].1["state"], "messages"), content);
        assert_eq!(
            decode_textual(
                &seen[0].1["questions"]["q0"]["instructions"],
                "question_content"
            ),
            instructions
        );
        assert!(seen[0].1.get("assets").is_none());
        assert!(seen[0].1.get("messages").is_none());
    }
}

#[tokio::test]
async fn native_three_types_combine_lowered_messages_question_json_and_boolean_choices() {
    for kind in ["typesafe", "openrouter"] {
        let mut request = native_request_three();
        request["input"] = json!([{"role":"system","content":"标签仅作文本上下文"},{"role":"user","content":[{"type":"input_json","data":{"ticket":"中文工单","flag":false}}]}]);
        for q in request["questions"].as_array_mut().unwrap() {
            q["instructions"] = json!([{"type":"input_text","text":q["instructions"]},{"type":"input_json","data":{"rubric":["甲","乙"],"ok":true}}]);
        }
        request["questions"][1]["choices"] =
            json!([{"value":true,"description":"真"},{"value":false,"description":"假"}]);
        let mut response = native_success_three();
        response["answers"]["q1"] = json!({"type":"choice","choice":"decora_choice_1","confidence":0.3,"probabilities":{"decora_choice_0":0.3,"decora_choice_1":0.7}});
        let upstream = Mock::new(vec![Reply::ok(response)]).await;
        let (s, _, v) = call(
            &gateway(config_value(
                json!({"a":minimal_native_backend(&upstream.url,kind)}),
            )),
            request.clone(),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["answers"][0]["probability"], 0.9);
        assert_eq!(v["answers"][1]["choice"], false);
        assert_eq!(v["answers"][2]["score"], 0.8);
        let seen = upstream.state.seen.lock().unwrap();
        assert_eq!(
            decode_textual(&seen[0].1["state"], "messages"),
            request["input"]
        );
        for i in 0..3 {
            assert_eq!(
                decode_textual(
                    &seen[0].1["questions"][format!("q{i}")]["instructions"],
                    "question_content"
                ),
                request["questions"][i]["instructions"]
            );
        }
    }
}

#[tokio::test]
async fn native_empty_and_string_content_survive_textual_lowering() {
    for kind in ["typesafe", "openrouter"] {
        for input in [
            json!(""),
            json!([]),
            json!([{"role":"user","content":""}]),
            json!([{"role":"developer","content":[]},{"role":"user","content":[]}]),
        ] {
            let upstream = Mock::new(vec![Reply::ok(json!({"model":"native","answers":{"q0":{"type":"noul","noul":0.5}},"usage":{"input_tokens":0,"output_tokens":0}}))]).await;
            let request = json!({"model":"decora-test","input":input,"questions":[{"type":"predicate","instructions":""}]});
            let (s, _, v) = call(
                &gateway(config_value(
                    json!({"a":minimal_native_backend(&upstream.url,kind)}),
                )),
                request,
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{v}");
            let seen = upstream.state.seen.lock().unwrap();
            if input.is_string() {
                assert_eq!(seen[0].1["state"], input);
            } else {
                assert_eq!(decode_textual(&seen[0].1["state"], "messages"), input);
            }
            assert_eq!(seen[0].1["questions"]["q0"]["instructions"], "");
        }
    }
}

fn typed_choice_request() -> Value {
    json!({"model":"decora-test","input":"选择分类","questions":[{"type":"choice","name":"typed","instructions":"按值的类型和描述选择","choices":[
        {"value":true,"description":"布尔真"},{"value":"true","description":"字符串真"},
        {"value":false,"description":"布尔假，不是缺省"},{"value":"false","description":"字符串假"},
        {"value":"decora_choice_0","description":"与内部键相同的原始字符串😀"},{"value":""}
    ]}]})
}
fn typed_native_response(selected: usize) -> Value {
    let probabilities: serde_json::Map<String, Value> = (0..6)
        .map(|i| {
            (
                format!("decora_choice_{i}"),
                json!(if i == selected { 0.75 } else { 0.05 }),
            )
        })
        .collect();
    json!({"model":"native","answers":{"q0":{"type":"choice","choice":format!("decora_choice_{selected}"),"confidence":0.6,"probabilities":probabilities}},"usage":{"input_tokens":10,"output_tokens":1}})
}

#[tokio::test]
async fn native_mixed_typed_choices_roundtrip_without_bool_string_or_internal_key_collisions() {
    for kind in ["typesafe", "openrouter"] {
        for selected in 0..6 {
            let request = typed_choice_request();
            let upstream = Mock::new(vec![Reply::ok(typed_native_response(selected))]).await;
            let (s, _, v) = call(
                &gateway(config_value(
                    json!({"a":minimal_native_backend(&upstream.url,kind)}),
                )),
                request.clone(),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{v}");
            assert_eq!(
                v["answers"][0]["choice"],
                request["questions"][0]["choices"][selected]["value"]
            );
            for i in 0..6 {
                assert_eq!(
                    v["answers"][0]["probabilities"][i]["value"],
                    request["questions"][0]["choices"][i]["value"]
                );
                assert_eq!(
                    v["answers"][0]["probabilities"][i]["probability"],
                    json!(if i == selected { 0.75 } else { 0.05 })
                );
            }
            let seen = upstream.state.seen.lock().unwrap();
            let criteria = &seen[0].1["questions"]["q0"]["criteria"];
            assert_eq!(criteria.as_object().unwrap().len(), 6);
            for i in 0..6 {
                assert_eq!(
                    criteria[format!("decora_choice_{i}")],
                    request["questions"][0]["choices"][i]
                );
            }
        }
    }
}

#[tokio::test]
async fn invalid_typed_choice_keys_are_protocol_errors_without_fallback() {
    for kind in ["typesafe", "openrouter"] {
        let mut raw_bool = typed_native_response(2);
        raw_bool["answers"]["q0"]["choice"] = json!(false);
        let mut unknown = typed_native_response(2);
        unknown["answers"]["q0"]["choice"] = json!("false");
        let mut probs = typed_native_response(2);
        let old = probs["answers"]["q0"]["probabilities"]
            .as_object_mut()
            .unwrap()
            .remove("decora_choice_0")
            .unwrap();
        probs["answers"]["q0"]["probabilities"]["foreign"] = old;
        let upstream = Mock::new(vec![
            Reply::ok(raw_bool),
            Reply::ok(unknown),
            Reply::ok(probs),
        ])
        .await;
        let fallback = Mock::new(vec![]).await;
        let gw = gateway(config_value(
            json!({"a":native_backend(&upstream.url,kind,0),"b":backend(&fallback.url,1)}),
        ));
        for _ in 0..3 {
            let (s, _, v) = call(&gw, typed_choice_request()).await;
            assert_eq!(s, StatusCode::BAD_GATEWAY);
            assert_eq!(v["error"]["code"], "upstream_protocol_error");
        }
        assert_eq!(upstream.count(), 3);
        assert_eq!(fallback.count(), 0);
    }
}

#[tokio::test]
async fn native_input_strict_is_independent_of_typed_choice_and_usage_policies() {
    for kind in ["typesafe", "openrouter"] {
        let upstream = Mock::new(vec![Reply::ok(typed_native_response(2))]).await;
        let mut cfg = config_value(json!({"a":minimal_native_backend(&upstream.url,kind)}));
        cfg["native_input_policy"] = json!("strict");
        let gw = gateway(cfg);
        for request in [
            json!({"model":"decora-test","input":[{"role":"user","content":"x"}],"questions":[{"type":"predicate","instructions":"q"}]}),
            json!({"model":"decora-test","input":"x","questions":[{"type":"predicate","instructions":[{"type":"input_json","data":{"x":false}}]}]}),
        ] {
            let (s, _, v) = call(&gw, request).await;
            assert_eq!(s, StatusCode::BAD_REQUEST);
            assert_eq!(v["error"]["code"], "unsupported_textual_lowering");
        }
        assert_eq!(upstream.count(), 0);
        let (s, _, v) = call(&gw, typed_choice_request()).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["answers"][0]["choice"], false);
        assert_eq!(upstream.count(), 1);
    }
}

#[tokio::test]
async fn native_actual_images_are_never_hidden_inside_textual_lowering() {
    for kind in ["typesafe", "openrouter"] {
        let native = Mock::new(vec![]).await;
        let openai = Mock::new(vec![Reply::ok(success()), Reply::ok(success())]).await;
        let gw = gateway(config_value(
            json!({"a":native_backend(&native.url,kind,0),"b":backend(&openai.url,1)}),
        ));
        for question_image in [false, true] {
            let mut request = predicate();
            if question_image {
                request["questions"][0]["instructions"] = json!([{"type":"input_json","data":{"x":1}},{"type":"input_image","image_url":"https://example.com/image.png"}]);
            } else {
                request["input"] = json!([{"role":"user","content":[{"type":"input_json","data":{"x":1}},{"type":"input_image","image_url":"https://example.com/image.png"}]}]);
            }
            let (s, _, v) = call(&gw, request.clone()).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            let seen = openai.state.seen.lock().unwrap();
            let body = &seen.last().unwrap().1;
            assert_eq!(body["input"], request["input"]);
            assert_eq!(body["questions"], request["questions"]);
        }
        assert_eq!(native.count(), 0);
        assert_eq!(openai.count(), 2);
    }
}

#[tokio::test]
async fn lowering_capabilities_can_be_disabled_and_openai_keeps_original_structures() {
    for (capability, request) in [
        (
            "messages",
            json!({"model":"decora-test","input":[{"role":"user","content":"x"}],"questions":[{"type":"predicate","instructions":"q"}]}),
        ),
        (
            "json_input",
            json!({"model":"decora-test","input":"x","questions":[{"type":"predicate","instructions":[{"type":"input_json","data":{"x":1}}]}]}),
        ),
        ("typed_choices", typed_choice_request()),
    ] {
        let upstream = Mock::new(vec![]).await;
        let mut cfg = config_value(json!({"a":minimal_native_backend(&upstream.url,"typesafe")}));
        cfg["backends"]["a"]["capabilities"][capability] = json!(false);
        let (s, _, v) = call(&gateway(cfg), request).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"]["code"], "unsupported_capability");
        assert_eq!(upstream.count(), 0);
    }
    let mut request = typed_choice_request();
    request["input"] = json!([{"role":"system","content":"原生系统指令"},{"role":"user","content":[{"type":"input_json","data":{"x":[false,"true"]}}]}]);
    request["questions"][0]["instructions"] =
        json!([{"type":"input_text","text":"任务"},{"type":"input_json","data":{"key":null}}]);
    let probabilities: Vec<Value> = (0..6).map(|i| json!({"value":request["questions"][0]["choices"][i]["value"],"probability":if i == 2 { 0.75 } else { 0.05 }})).collect();
    let upstream = Mock::new(vec![Reply::ok(json!({"model":"openai","answers":[{"type":"choice","name":"typed","choice":false,"confidence":0.6,"probabilities":probabilities}],"usage":usage()}))]).await;
    let mut cfg = config_value(json!({"a":backend(&upstream.url,0)}));
    cfg["native_input_policy"] = json!("strict");
    let (s, _, v) = call(&gateway(cfg), request.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["answers"][0]["choice"], false);
    let seen = upstream.state.seen.lock().unwrap();
    assert_eq!(seen[0].1["input"], request["input"]);
    assert_eq!(seen[0].1["questions"], request["questions"]);
}

#[test]
fn official_native_endpoints_and_usage_policy_are_explicit() {
    use decora::config::{BackendKind, NativeInputPolicy, UsagePolicy};
    assert_eq!(
        BackendKind::Typesafe.endpoint(),
        "https://api.typesafe.ai/v1/systemone"
    );
    assert_eq!(
        BackendKind::Openrouter.endpoint(),
        "https://openrouter.ai/api/alpha/decisions"
    );
    let cfg = config_value(json!({"a":backend("http://127.0.0.1:1234/decisions",0)}));
    let parsed = Config::from_yaml(&cfg.to_string()).unwrap();
    assert!(parsed.usage_policy == UsagePolicy::Compatible);
    assert!(parsed.native_input_policy == NativeInputPolicy::Textual);
    let mut bad = cfg.clone();
    bad["usage_policy"] = json!("guess_zero");
    assert!(Config::from_yaml(&bad.to_string()).is_err());
    let mut bad = cfg;
    bad["native_input_policy"] = json!("invent_roles");
    assert!(Config::from_yaml(&bad.to_string()).is_err());
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
