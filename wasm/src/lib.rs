//! WebAssembly C-ABI entrypoints running Decora's real Rust protocol, dialect, and adapter engine in the browser.
use decora::{
    adapter,
    config::{Backend, BackendKind, Capabilities, UsagePolicy},
    dialect::Dialect,
    error::ApiError,
    protocol::{DecisionRequest, DecisionResponse, Requirements},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Allocate a zeroed byte buffer of `len` bytes inside linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let buf = vec![0u8; len].into_boxed_slice();
    Box::into_raw(buf) as *mut u8
}

/// Free a buffer previously returned by `alloc` or `decora_eval`.
///
/// # Safety
/// `ptr` must be either null or a pointer returned by `alloc`/`decora_eval` with the exact same `len`,
/// and must not be freed more than once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // Reconstruct the exact boxed slice created by `alloc` or `decora_eval`.
    let slice = std::ptr::slice_from_raw_parts_mut(ptr, len);
    drop(unsafe { Box::from_raw(slice) });
}

/// Evaluate a JSON command through Decora's Rust protocol engine and return `(out_ptr << 32) | out_len`.
///
/// # Safety
/// If `len > 0`, `ptr` must point to a valid readable byte slice of length `len` in WASM linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn decora_eval(ptr: *const u8, len: usize) -> u64 {
    let input = if ptr.is_null() || len == 0 {
        ""
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        std::str::from_utf8(bytes).unwrap_or("")
    };
    let output = match dispatch(input) {
        Ok(v) => v,
        Err(v) => v,
    };
    let encoded = serde_json::to_string(&output)
        .unwrap_or_else(|_| r#"{"ok":false,"status":500,"code":"serialize_error"}"#.to_owned());
    let boxed = encoded.into_bytes().into_boxed_slice();
    let out_len = boxed.len() as u64;
    let out_ptr = Box::into_raw(boxed) as *mut u8 as u32 as u64;
    (out_ptr << 32) | out_len
}

#[derive(Deserialize)]
struct EvalCommand {
    op: String,
    dialect: String,
    target: String,
    #[serde(default)]
    target_model: Option<String>,
    #[serde(default)]
    backup: Option<String>,
    #[serde(default)]
    backup_model: Option<String>,
    #[serde(default)]
    usage_policy: UsagePolicy,
    request: Value,
    #[serde(default)]
    upstream_response: Option<Value>,
    #[serde(default)]
    phantom: bool,
}

fn parse_dialect(name: &str) -> Result<Dialect, Value> {
    match name {
        "openai" => Ok(Dialect::Openai),
        "gemini" => Ok(Dialect::Gemini),
        "typesafe" => Ok(Dialect::Typesafe),
        "openrouter" => Ok(Dialect::Openrouter),
        _ => Err(err_json(ApiError::invalid(), "inbound_dialect")),
    }
}

fn parse_backend_kind(name: &str) -> Result<BackendKind, Value> {
    match name {
        "openai" => Ok(BackendKind::Openai),
        "gemini" => Ok(BackendKind::Gemini),
        "typesafe" => Ok(BackendKind::Typesafe),
        "openrouter" => Ok(BackendKind::Openrouter),
        _ => Err(err_json(ApiError::invalid(), "target_backend")),
    }
}

fn dialect_endpoint(d: Dialect) -> &'static str {
    match d {
        Dialect::Openai => "POST /v1/decisions",
        Dialect::Gemini => "POST /v1beta/decisions",
        Dialect::Typesafe => "POST /v0/decisions",
        Dialect::Openrouter => "POST /api/v1/decisions",
    }
}

fn default_model(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Openai => "gpt-6-luna",
        BackendKind::Gemini => "gemini-decision-1",
        BackendKind::Typesafe => "jev-latest",
        BackendKind::Openrouter => "typesafe/jev-1.13",
    }
}

fn auth_header_name(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Gemini => "x-goog-api-key: $GEMINI_API_KEY",
        BackendKind::Openai => "Authorization: Bearer $OPENAI_API_KEY",
        BackendKind::Typesafe => "Authorization: Bearer $TYPESAFE_API_KEY",
        BackendKind::Openrouter => "Authorization: Bearer $OPENROUTER_API_KEY",
    }
}

fn backend_for(kind: BackendKind, model_override: Option<&str>) -> Backend {
    let is_openai = kind == BackendKind::Openai;
    Backend {
        kind,
        model: model_override
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| default_model(kind))
            .to_owned(),
        api_key_env: "WASM_DEMO_MOCK_KEY".to_owned(),
        endpoint: None,
        priority: 0,
        weight: 1,
        capabilities: Capabilities {
            question_types: vec!["predicate".into(), "choice".into(), "score".into()],
            images: is_openai,
            prompt_cache: is_openai,
            safety_identifier: is_openai,
            max_questions: 400,
        },
    }
}

fn err_json(err: ApiError, stage: &str) -> Value {
    json!({
        "ok": false,
        "stage": stage,
        "status": err.status.as_u16(),
        "code": err.code,
        "error_body": err.error_body(),
    })
}

fn canonical_summary(req: &DecisionRequest, need: &Requirements) -> Value {
    let q_types: Vec<&'static str> = req.questions.iter().map(|q| q.kind()).collect();
    json!({
        "model_alias": req.model,
        "question_count": req.questions.len(),
        "question_types": q_types,
        "image_count": need.images,
        "has_prompt_cache": need.cache,
        "has_safety_identifier": need.safety,
    })
}

fn build_upstream_wire(
    dialect: Dialect,
    backend: &Backend,
    raw_request: &Value,
    req: &DecisionRequest,
) -> Result<(&'static str, Value), ApiError> {
    if dialect.matches_backend(backend.kind) {
        Ok((
            "same_dialect_passthrough",
            dialect.passthrough_body(raw_request, req, &backend.model),
        ))
    } else if backend.kind == BackendKind::Openai {
        Ok(("cross_dialect_ir", req.openai_body(&backend.model)))
    } else {
        Ok((
            "cross_dialect_ir",
            adapter::native_body_for(req, &backend.model, backend.kind)?,
        ))
    }
}

fn verify_and_format(
    dialect: Dialect,
    backend: &Backend,
    raw_request: &Value,
    req: &DecisionRequest,
    upstream_response: Value,
    usage_policy: UsagePolicy,
) -> Result<(&'static str, Value), ApiError> {
    if dialect.matches_backend(backend.kind) {
        let out = dialect.passthrough_response(raw_request, req, upstream_response)?;
        Ok(("same_dialect_passthrough", out))
    } else {
        let canonical = if backend.kind == BackendKind::Openai {
            DecisionResponse::parse(upstream_response, req)?
        } else {
            adapter::native_response(upstream_response, req, backend.kind, usage_policy)?
        };
        let out = dialect.format_response(req, canonical)?;
        Ok(("cross_dialect_ir", out))
    }
}

/// Generate a schema-accurate synthetic native upstream response matching `demo/mock.py`.
fn mock_upstream_fixture(wire_body: &Value, kind: BackendKind, phantom: bool) -> Value {
    let (pred_p, choice_probs, high_p): (f64, [f64; 3], f64) = match kind {
        BackendKind::Openai => (0.91, [0.05, 0.10, 0.85], 0.90),
        BackendKind::Gemini => (0.84, [0.10, 0.20, 0.70], 0.75),
        BackendKind::Typesafe => (0.88, [0.15, 0.10, 0.75], 0.80),
        BackendKind::Openrouter => (0.79, [0.20, 0.15, 0.65], 0.65),
    };

    let build_answer = |q: &Value, is_openai: bool| -> Value {
        let qtype = q.get("type").and_then(Value::as_str).unwrap_or("predicate");
        let mut ans = Map::new();
        ans.insert("type".into(), json!(qtype));
        if is_openai {
            ans.insert("name".into(), q.get("name").cloned().unwrap_or(Value::Null));
        }
        match qtype {
            "predicate" | "noul" => {
                let field = if qtype == "predicate" {
                    "probability"
                } else {
                    "noul"
                };
                ans.insert(field.into(), json!(pred_p));
            }
            "choice" => {
                let values: Vec<Value> = if is_openai {
                    q.get("choices")
                        .and_then(Value::as_array)
                        .map(|arr| arr.iter().filter_map(|c| c.get("value").cloned()).collect())
                        .unwrap_or_default()
                } else {
                    q.get("criteria")
                        .and_then(Value::as_object)
                        .map(|obj| obj.keys().map(|k| json!(k)).collect())
                        .unwrap_or_default()
                };
                let n = values.len();
                let mut probs: Vec<f64> = if n == 3 {
                    choice_probs.to_vec()
                } else if n == 2 {
                    let lo = ((1.0 - choice_probs[2]) * 100.0).round() / 100.0;
                    vec![lo, choice_probs[2]]
                } else if n > 0 {
                    let base = (1.0 / (n as f64) * 10000.0).floor() / 10000.0;
                    let mut v = vec![base; n];
                    let sum_init: f64 = v[..n - 1].iter().sum();
                    v[n - 1] = ((1.0 - sum_init) * 10000.0).round() / 10000.0;
                    v
                } else {
                    vec![]
                };
                let max_idx = probs
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let conf = probs.get(max_idx).copied().unwrap_or(0.0);
                if phantom && let Some(last) = probs.last_mut() {
                    *last = ((*last - 0.07) * 100.0).round() / 100.0;
                }
                if let Some(chosen) = values.get(max_idx) {
                    ans.insert("choice".into(), chosen.clone());
                }
                ans.insert("confidence".into(), json!(conf));
                if is_openai {
                    let arr: Vec<Value> = values
                        .into_iter()
                        .zip(probs)
                        .map(|(v, p)| json!({"value": v, "probability": p}))
                        .collect();
                    ans.insert("probabilities".into(), Value::Array(arr));
                } else {
                    let mut map = Map::new();
                    for (v, p) in values.into_iter().zip(probs) {
                        if let Some(s) = v.as_str() {
                            map.insert(s.to_owned(), json!(p));
                        }
                    }
                    ans.insert("probabilities".into(), Value::Object(map));
                }
            }
            "score" => {
                let criteria: Vec<Value> = if is_openai {
                    q.get("levels")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    q.get("criteria")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                };
                let n = criteria.len();
                let probs: Vec<f64> = if n == 2 {
                    let lo = ((1.0 - high_p) * 100.0).round() / 100.0;
                    vec![lo, high_p]
                } else if n > 0 {
                    let base = (1.0 / (n as f64) * 10000.0).floor() / 10000.0;
                    let mut v = vec![base; n];
                    let sum_init: f64 = v[..n - 1].iter().sum();
                    v[n - 1] = ((1.0 - sum_init) * 10000.0).round() / 10000.0;
                    v
                } else {
                    vec![]
                };
                let score: f64 = probs.iter().enumerate().map(|(i, p)| (i as f64) * p).sum();
                let score = (score * 10000.0).round() / 10000.0;
                let conf = probs.iter().copied().fold(0.0_f64, f64::max);
                ans.insert("score".into(), json!(score));
                ans.insert("confidence".into(), json!(conf));
                if is_openai {
                    let arr: Vec<Value> = criteria
                        .iter()
                        .enumerate()
                        .zip(&probs)
                        .map(|((i, lv), p)| {
                            json!({
                                "value": i,
                                "label": lv.get("label").and_then(Value::as_str).unwrap_or(""),
                                "probability": p,
                            })
                        })
                        .collect();
                    ans.insert("probabilities".into(), Value::Array(arr));
                } else {
                    let mut pmap = Map::new();
                    let mut lmap = Map::new();
                    for (i, (c, p)) in criteria.iter().zip(&probs).enumerate() {
                        pmap.insert(i.to_string(), json!(p));
                        lmap.insert(i.to_string(), c.clone());
                    }
                    ans.insert("legend".into(), Value::Object(lmap));
                    ans.insert("probabilities".into(), Value::Object(pmap));
                }
            }
            _ => {}
        }
        Value::Object(ans)
    };

    let answers = if kind == BackendKind::Openai {
        let list = wire_body
            .get("questions")
            .and_then(Value::as_array)
            .map(|qs| qs.iter().map(|q| build_answer(q, true)).collect())
            .unwrap_or_default();
        Value::Array(list)
    } else {
        let mut map = Map::new();
        if let Some(qs) = wire_body.get("questions").and_then(Value::as_object) {
            for (k, q) in qs {
                map.insert(k.clone(), build_answer(q, false));
            }
        }
        Value::Object(map)
    };

    let usage = match kind {
        BackendKind::Openai => json!({
            "input_tokens": 100,
            "input_tokens_details": {"cached_tokens": 10, "cache_write_tokens": 20},
            "output_tokens": 4,
            "output_tokens_details": {"reasoning_tokens": 2},
            "total_tokens": 104,
        }),
        BackendKind::Gemini => json!({
            "total_input_tokens": 120,
            "total_output_tokens": 0,
            "total_tokens": 120,
            "input_tokens_by_modality": [{"modality": "text", "tokens": 120}],
        }),
        BackendKind::Typesafe => json!({
            "input_tokens": 100,
            "output_tokens": 4,
        }),
        BackendKind::Openrouter => json!({
            "input_tokens": 100,
            "output_tokens": 4,
            "cost": 0.0001,
        }),
    };

    let mut result = Map::new();
    if kind != BackendKind::Gemini
        && let Some(model) = wire_body.get("model")
    {
        result.insert("model".into(), model.clone());
    }
    result.insert("answers".into(), answers);
    result.insert("usage".into(), usage);
    Value::Object(result)
}

fn dispatch(input: &str) -> Result<Value, Value> {
    let cmd: EvalCommand = serde_json::from_str(input).map_err(|_| {
        json!({
            "ok": false,
            "stage": "command",
            "status": 400,
            "code": "invalid_command",
            "error_body": ApiError::invalid().error_body(),
        })
    })?;
    let dialect = parse_dialect(&cmd.dialect)?;
    let target_kind = parse_backend_kind(&cmd.target)?;
    let backend = backend_for(target_kind, cmd.target_model.as_deref());

    let (req, need) = dialect
        .parse_request(cmd.request.clone(), 32)
        .map_err(|e| err_json(e, "inbound_parse"))?;

    match cmd.op.as_str() {
        "translate" => {
            adapter::compatible_for_dialect(dialect, &backend, &req, &need, cmd.usage_policy)
                .map_err(|e| err_json(e, "capability_filter"))?;
            let (mode, upstream_body) = build_upstream_wire(dialect, &backend, &cmd.request, &req)
                .map_err(|e| err_json(e, "outbound_translate"))?;
            let mock_response = mock_upstream_fixture(&upstream_body, backend.kind, cmd.phantom);
            let verified = verify_and_format(
                dialect,
                &backend,
                &cmd.request,
                &req,
                mock_response.clone(),
                cmd.usage_policy,
            );
            let (verify_ok, client_status, client_body, error_code) = match verified {
                Ok((_, body)) => (true, 200_u16, body, None),
                Err(e) => (false, e.status.as_u16(), e.error_body(), Some(e.code)),
            };
            Ok(json!({
                "ok": true,
                "mode": mode,
                "inbound_endpoint": dialect_endpoint(dialect),
                "target_endpoint": backend.kind.endpoint(),
                "auth_header": auth_header_name(backend.kind),
                "canonical_summary": canonical_summary(&req, &need),
                "upstream_body": upstream_body,
                "mock_upstream_response": mock_response,
                "verify_ok": verify_ok,
                "client_status": client_status,
                "client_response": client_body,
                "error_code": error_code,
            }))
        }
        "verify" => {
            adapter::compatible_for_dialect(dialect, &backend, &req, &need, cmd.usage_policy)
                .map_err(|e| err_json(e, "capability_filter"))?;
            let upstream_resp = cmd
                .upstream_response
                .ok_or_else(|| err_json(ApiError::invalid(), "missing_upstream_response"))?;
            let (mode, client_response) = verify_and_format(
                dialect,
                &backend,
                &cmd.request,
                &req,
                upstream_resp,
                cmd.usage_policy,
            )
            .map_err(|e| err_json(e, "upstream_verify"))?;
            Ok(json!({
                "ok": true,
                "mode": mode,
                "client_status": 200,
                "client_response": client_response,
            }))
        }
        "failover" => {
            let backup_kind = parse_backend_kind(cmd.backup.as_deref().unwrap_or("typesafe"))?;
            let backup_backend = backend_for(backup_kind, cmd.backup_model.as_deref());

            // Attempt 1: Primary (simulated 503 Service Unavailable)
            adapter::compatible_for_dialect(dialect, &backend, &req, &need, cmd.usage_policy)
                .map_err(|e| err_json(e, "primary_capability_filter"))?;
            let (primary_mode, primary_wire) =
                build_upstream_wire(dialect, &backend, &cmd.request, &req)
                    .map_err(|e| err_json(e, "primary_translate"))?;

            // Attempt 2: Backup (per-attempt switch between passthrough & Canonical IR)
            adapter::compatible_for_dialect(
                dialect,
                &backup_backend,
                &req,
                &need,
                cmd.usage_policy,
            )
            .map_err(|e| err_json(e, "backup_capability_filter"))?;
            let (backup_mode, backup_wire) =
                build_upstream_wire(dialect, &backup_backend, &cmd.request, &req)
                    .map_err(|e| err_json(e, "backup_translate"))?;

            let mock_backup_resp = cmd.upstream_response.unwrap_or_else(|| {
                mock_upstream_fixture(&backup_wire, backup_backend.kind, cmd.phantom)
            });
            let (_, client_response) = verify_and_format(
                dialect,
                &backup_backend,
                &cmd.request,
                &req,
                mock_backup_resp.clone(),
                cmd.usage_policy,
            )
            .map_err(|e| err_json(e, "backup_verify"))?;

            Ok(json!({
                "ok": true,
                "inbound_endpoint": dialect_endpoint(dialect),
                "canonical_summary": canonical_summary(&req, &need),
                "attempts": [
                    {
                        "attempt": 1,
                        "role": "primary",
                        "target": cmd.target,
                        "mode": primary_mode,
                        "target_endpoint": backend.kind.endpoint(),
                        "auth_header": auth_header_name(backend.kind),
                        "upstream_body": primary_wire,
                        "simulated_status": 503,
                        "outcome": "retryable_503_fallback",
                    },
                    {
                        "attempt": 2,
                        "role": "backup",
                        "target": cmd.backup.unwrap_or_else(|| "typesafe".into()),
                        "mode": backup_mode,
                        "target_endpoint": backup_backend.kind.endpoint(),
                        "auth_header": auth_header_name(backup_backend.kind),
                        "upstream_body": backup_wire,
                        "simulated_status": 200,
                        "outcome": "verified_ok",
                    }
                ],
                "mode": backup_mode,
                "target_endpoint": backup_backend.kind.endpoint(),
                "auth_header": auth_header_name(backup_backend.kind),
                "upstream_body": backup_wire,
                "mock_upstream_response": mock_backup_resp,
                "verify_ok": true,
                "client_status": 200,
                "client_response": client_response,
            }))
        }
        _ => Err(err_json(ApiError::invalid(), "unknown_op")),
    }
}
