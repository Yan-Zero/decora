# Design: Same-Dialect Native Passthrough (`dialect-passthrough`)

## 1. Dialect and Backend Matching (`src/dialect.rs`)

Add a method on `Dialect`:
```rust
impl Dialect {
    pub fn matches_backend(self, kind: BackendKind) -> bool {
        matches!(
            (self, kind),
            (Self::Openai, BackendKind::Openai)
                | (Self::Gemini, BackendKind::Gemini)
                | (Self::Typesafe, BackendKind::Typesafe)
                | (Self::Openrouter, BackendKind::Openrouter)
        )
    }
}
```

## 2. Candidate Compatibility Filtering (`src/adapter.rs`)

- Add `compatible_for_dialect(dialect: Dialect, backend: &Backend, req: &DecisionRequest, need: &Requirements, usage_policy: UsagePolicy) -> Result<(), ApiError>` (while keeping the original `compatible` signature and having it call this with `Dialect::Openai` by default, ensuring backward compatibility):
  - Base capability checks (`max_questions`, `question_types`, `images`, `prompt_cache`, `safety_identifier`) apply uniformly to all backends;
  - If `backend.kind != BackendKind::Openai`:
    - Still call `validate_native_request(req)` (ensuring that even `Gemini -> Gemini` passthrough carrying `inline_data` images returns `unsupported_native_images` due to the native Decisions adapter boundary);
    - When `usage_policy == UsagePolicy::Strict`, return `Err(ApiError::unsupported("unsupported_usage"))` only when `!dialect.matches_backend(backend.kind)`; if `dialect.matches_backend(backend.kind)` is true, allow that same-dialect candidate through.

## 3. Passthrough Request Construction and Passthrough Response Validation (`src/dialect.rs`)

- **`Dialect::passthrough_body(self, raw_request: &Value, req: &DecisionRequest, backend_model: &str) -> Value`**:
  - `Self::Openai`: call `req.openai_body(backend_model)`;
  - `Self::Gemini | Self::Typesafe | Self::Openrouter`: clone the `raw_request` that already passed strict `parse_request` validation, set only `body["model"] = json!(backend_model)`, and fully preserve the original `questions` key names, option key names, and `criteria`.
- **`Dialect::passthrough_response(self, raw_request: &Value, req: &DecisionRequest, upstream_body: Value, usage_policy: UsagePolicy) -> Result<Value, ApiError>`**:
  - `Self::Openai`: call `DecisionResponse::parse_with_usage(upstream_body, req, usage_policy)` and serialize it to `Value`;
  - `Self::Gemini | Self::Typesafe | Self::Openrouter`:
    1. Validate the top-level `model` (optional non-empty string for Gemini; required non-empty string for TypeSafe/OpenRouter), `answers` (object whose key set corresponds one-to-one with the `name` of `req.questions`), and `usage` (object).
    2. Remap the answers stored under the original question key `q.name()` in `upstream_body["answers"]` into an object indexed by `q0..qN-1`, and reuse `DecisionResponse::parse_with_usage` to strictly validate all questions' probability ranges, distribution sums, `choice` highest-probability option, `score` weighted expectation, and `legend == raw_request["questions"][name]["criteria"][i]`.
    3. Validate native `usage` conservation (Gemini validates `total_input_tokens + total_output_tokens == total_tokens` and the optional `input_tokens_by_modality`; TypeSafe/OpenRouter validate `input_tokens + output_tokens == total_tokens`, the optional `input_tokens_details / output_tokens_details`, and the optional non-negative finite `cost`).
    4. Build the output JSON: `{"model": req.model, "answers": upstream_body["answers"], "usage": validated_native_usage}` (**do not inject `usage.decora`**).

## 4. Gateway Execution Pipeline (`src/gateway.rs`)

- `Gateway::execute` takes `(dialect: Dialect, raw_request: Value, request: DecisionRequest, need: Requirements, deadline: Deadline) -> Result<Value, ApiError>`:
  - At the routing stage, call `compatible_for_dialect(dialect, ...)`;
  - In each `attempt` loop iteration:
    - If `dialect.matches_backend(backend.config.kind)`:
      - Send `dialect.passthrough_body(&raw_request, &request, &backend.config.model)`;
      - After receiving a 2xx response, call `dialect.passthrough_response(&raw_request, &request, value, self.inner.config.usage_policy)` and return the result directly;
    - Otherwise (cross-dialect routing, or failover to a heterogeneous backend):
      - Send via `BackendKind::Openai => request.openai_body(...)` or `native_body_for(&request, ...)`;
      - After receiving a 2xx response, convert it to a Canonical `DecisionResponse` via `DecisionResponse::parse` / `native_response`, then call `dialect.format_response(&request, resp)` to return.