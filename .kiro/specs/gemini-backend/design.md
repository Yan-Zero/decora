# Design: Google Gemini Native Decisions Backend Support (`gemini-backend`)

## 1. Architecture overview

This extension keeps Decora's existing four-layer responsibility split unchanged:
1. **`src/config.rs`**: add a `Gemini` variant to `BackendKind`, with default endpoint `https://generativelanguage.googleapis.com/v1beta/decisions`.
2. **`src/adapter.rs`**: dispatch upstream native request construction (`native_body_for`) and native response parsing (`native_response`) by `BackendKind`, reusing the same core answer validation and capability boundary checks.
3. **`src/gateway.rs`**: select the authentication header by `BackendKind` for outbound requests (`Gemini` uses `x-goog-api-key`, other backends use `Authorization: Bearer`).
4. **`tests/offline.rs` and documentation**: add loopback mock end-to-end regression tests, and update `README.md`, `docs/PROTOCOL.md`, `config.example.yaml` and `HANDOFF.md`.

---

## 2. Detailed design

### 2.1 `src/config.rs`: backend kinds and default endpoints

```rust
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Openai,
    Openrouter,
    Typesafe,
    Gemini,
}

impl BackendKind {
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1/decisions",
            Self::Openrouter => "https://openrouter.ai/api/alpha/decisions",
            Self::Typesafe => "https://api.typesafe.ai/v1/systemone",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/decisions",
        }
    }
}
```

### 2.2 `src/gateway.rs`: outbound auth header dispatch

When building the `reqwest::RequestBuilder` in `Gateway::execute`:
- When `backend.config.kind == BackendKind::Gemini`, call `.header("x-goog-api-key", &backend.key)`.
- When `backend.config.kind` is `Openai | Openrouter | Typesafe`, keep `.bearer_auth(&backend.key)`.

### 2.3 `src/adapter.rs`: request mapping (`native_body_for`)

To preserve backward compatibility, either keep `native_body(req, model)` or add `native_body_for(req, model, kind: BackendKind)`:

1. **Top-level input field**:
   - `Openrouter` / `Typesafe`: keep the existing `"state": textual(&req.input, "messages")` (verified by existing tests and the contract).
   - `Gemini`: use `"input": textual(&req.input, "messages")`. When `req.input` is a string, pass it through directly; when `req.input` is a role-message array, serialize it to the string `{"decora_encoding":"openai_decisions_text_v1","kind":"messages","data":...}`.

2. **Question conversion (`questions` map, `q0..qN-1`)**:
   - **`predicate` -> `noul`**:
     `{"type": "noul", "instructions": textual(q.instructions(), "question_content")}`.
   - **`choice`**:
     - `Openrouter` / `Typesafe`: keep existing behavior (pure string groups use the original string keys and `choice.description`; groups containing booleans use `decora_choice_0..N-1` and `{"value": ..., "description": ...}`).
     - `Gemini`: because Gemini requires option keys to be `1..=128` characters of pure ASCII (no Chinese, empty strings, leading/trailing whitespace, etc.) and `criteria` values to be strings, Gemini uniformly uses `decora_choice_0..N-1` as option keys and serializes each option's `criteria` value as a compact JSON string:
       - no description: `json!({"value": choice.value}).to_string()`
       - with description: `json!({"value": choice.value, "description": d}).to_string()`
       This satisfies Gemini's pure-ASCII key and string-description requirements while fully preserving the original option value semantics at evaluation time, and on return uses the `decora_choice_0..N-1` index to restore any `ChoiceValue` 100% losslessly (including Chinese, empty strings, boolean `true`/`false` and strings identical to internal keys).
   - **`score`**:
     - `Openrouter` / `Typesafe`: keep existing behavior (the `l.label` string when there is no description, `{"label": l.label, "description": d}` when there is one).
     - `Gemini`: Gemini requires `score.criteria` to be `2..=255` **non-empty strings** (`string[]`). To prevent an empty `label` from violating Gemini's non-empty constraint while preserving both `label` and `description`:
       - when `description` is `None` and `!l.label.is_empty()`, use the `l.label` string directly;
       - when `l.label.is_empty()` or `description.is_some()`, use the compact JSON string `json!({"label": l.label, "description": l.description}).to_string()` (always non-empty).
       The response `legend` is still strictly compared item by item against `body["questions"][&key]["criteria"][i]`, then restored to the OpenAI contract form `{"value": i, "label": l.label, "probability": ...}`.

### 2.4 `src/adapter.rs`: response and usage parsing (`native_response`)

1. **Top-level `model` field validation**:
   - `Openrouter` / `Typesafe`: continue to require the upstream response to contain a non-empty string `value["model"]`.
   - `Gemini`: the official response top level has only `answers` and `usage` (no `model`); if `value.get("model")` exists, it must be a non-empty string; if absent, pass `&req.model` directly to `DecisionResponse::parse_with_usage` (which always sets the external `model` to `req.model`).

2. **Gemini `usage` parsing (`usage_policy: compatible`)**:
   - Read the required fields `total_input_tokens`, `total_output_tokens` and `total_tokens`; all must be non-negative `i32` integers.
   - Validate `total_input_tokens.checked_add(total_output_tokens) == Some(total_tokens)`; otherwise return `502 upstream_protocol_error`.
   - If `input_tokens_by_modality` is present:
     - it must be an array; each element must be an object containing a non-empty string `modality` and a non-negative `i32` `tokens`;
     - after validation, write it to `usage.decora.input_tokens_by_modality`.
   - Produce the top-level compatible `usage`:
     ```json
     {
       "input_tokens": total_input_tokens,
       "output_tokens": total_output_tokens,
       "total_tokens": total_tokens,
       "decora": {
         "provider": "gemini",
         "input_tokens_basis": "total_input_tokens",
         "total_tokens_basis": "total_input_plus_output",
         "details_complete": false
       }
     }
     ```
     (If `input_tokens_by_modality` is present, include it in `usage.decora` as well.)
   - Do not fabricate `input_tokens_details` or `output_tokens_details`.

---

## 3. Failure and boundary behavior

- **Retry and fallback**: identical to OpenRouter/TypeSafe. A cross-protocol fallback chain such as `[gemini-primary, jev-official, openai-primary]` can be configured under a single public alias.
- **Security and privacy**: `GEMINI_API_KEY` is referenced only via the `api_key_env` environment variable name, validated at startup to be non-empty printable ASCII; logs print no request content, response body, URL or key.
