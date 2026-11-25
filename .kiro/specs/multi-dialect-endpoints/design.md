# Design: Multi-Dialect Inbound Endpoints and Symmetric Hub-and-Spoke Protocol Architecture (`multi-dialect-endpoints`)

## 1. Three-Stage Hub-and-Spoke Pipeline

```text
  [OpenAI Client]       POST /v1/decisions      ──┐
  [Gemini Client]       POST /v1beta/decisions  ──┼─► Dialect::parse_request
  [TypeSafe Client]     POST /v0/decisions      ──┤         │
  [OpenRouter Client]   POST /api/v1/decisions  ──┘         ▼
                                              Canonical IR (DecisionRequest, Requirements)
                                                            │
                                                            ▼
                                              Gateway::execute (routing + Adapter + mathematical invariant checks)
                                                            │
                                                            ▼
                                              Canonical IR (DecisionResponse)
                                                            │
                                                            ▼
                                              Dialect::format_response ──► return response in the corresponding dialect
```

## 2. Module Design and Responsibility Refactoring

### 2.1 `src/dialect.rs` (new)
- Define `#[derive(Clone, Copy, PartialEq, Eq)] pub enum Dialect { Openai, Gemini, Typesafe, Openrouter }`.
- **`Dialect::parse_request(self, value: Value, max_images: usize) -> Result<(DecisionRequest, Requirements), ApiError>`**:
  - `Dialect::Openai`: delegate directly to `DecisionRequest::parse(value, max_images)`.
  - `Dialect::Gemini`:
    - Validate that the top-level object contains only `['model', 'input', 'questions']` and that all are present.
    - If `input` is a string, preserve it unchanged. If it is a non-empty array (`<= 16384`), convert each `{"text": s}` to `{"type": "input_text", "text": s}` and each `{"inline_data": {"mime_type": m, "data": d}}` (validate that `m` contains no `;`) to `{"type": "input_image", "image_url": format!("data:{m};base64,{d}")}`. Wrap the result in a single `role: "user"` message and pass it to `DecisionRequest::parse` for shared base64, MIME, and image-count validation.
    - Validate that `questions` is a non-empty object (`1..=400`) and each key `k` contains `1..=64` ASCII `[A-Za-z0-9_-]` characters:
      - `type == "noul"` -> `Question::Predicate { name: Some(k), instructions }` (`instructions` must be a string).
      - `type == "choice"` -> validate `criteria` as an object with `2..=255` entries, keys that are `1..=128` plain ASCII characters without leading or trailing whitespace, and string descriptions `s` as values (preserve `description: Some(s)` so responses can reproduce them).
      - `type == "score"` -> validate `criteria` as an array with `2..=10` non-empty strings, mapped to `ScoreLevel { label, description: None }`.
  - `Dialect::Typesafe | Dialect::Openrouter`:
    - Validate that the top-level object contains only `['model', 'state', 'questions']` and that all are present; `state` must be a string.
    - Validate that `questions` is a non-empty object (`1..=400`) and each key `k` is a non-empty string:
      - `type == "noul"` -> `Question::Predicate { name: Some(k), instructions }` (`instructions` must be a string).
      - `type == "choice"` -> validate `criteria` as an object with `2..=255` entries, string keys, and values that are either `null` (`description: None`) or strings `s` (`description: Some(s)`).
      - `type == "score"` -> validate `criteria` as an array with `2..=10` entries, each either a string `label` (`description: None`) or `{"label": s, "description"?: d}` (`description` cannot be `null`).
- **`Dialect::format_response(self, req: &DecisionRequest, resp: DecisionResponse) -> Result<Value, ApiError>`**:
  - `Dialect::Openai`: serialize `resp` directly.
  - `Dialect::Gemini | Dialect::Typesafe | Dialect::Openrouter`:
    - Iterate over `resp.answers.iter().zip(&req.questions)`:
      - On `Answer::Refusal`, immediately return `ApiError::new(StatusCode::BAD_GATEWAY, "upstream_refusal_unrepresentable", "This dialect cannot express a per-question refusal; no probabilities were fabricated.")`.
      - `Answer::Predicate { probability, .. }` -> `{"type": "noul", "noul": probability}`.
      - `Answer::Choice { choice, confidence, probabilities, .. }` -> restore `choice` and `probabilities` as an object keyed by option strings.
      - `Answer::Score { score, confidence, probabilities, .. }` -> convert `probabilities` to an object indexed by `"0"..` and build `legend` (under Gemini, use the string `l.label`; under TypeSafe/OpenRouter, use `{"label": l.label, "description": d}` if `l.description` is `Some(d)`, otherwise `l.label`).
    - Build dialect-specific `usage`:
      - When `resp.usage.decora` is `None` (i.e., the upstream is the native OpenAI backend) and the inbound dialect is not OpenAI, automatically add `decora` metadata:
        `{"provider": "openai", "input_tokens_basis": "openai_input_tokens", "total_tokens_basis": "openai_input_plus_output", "details_complete": true}`.
      - `Dialect::Gemini` emits `total_input_tokens`, `total_output_tokens`, `total_tokens`, optional `input_tokens_by_modality`, and `decora`.
      - `Dialect::Typesafe | Dialect::Openrouter` emits `input_tokens`, `output_tokens`, `total_tokens`, optional `input_tokens_details / output_tokens_details`, and `decora`.

### 2.2 Streamlining and Deduplicating `src/adapter.rs`
- Extract two pure functions: `choice_key_for(choices: &[ChoiceOption], index: usize, kind: BackendKind) -> String` and `score_criterion(level: &ScoreLevel, kind: BackendKind) -> Value`.
- Have `native_answers_for` use `choice_key_for` and `score_criterion` directly to validate `legend`, eliminating the redundant work of rebuilding the entire `native_body_for` during response parsing.
- Extract a shared `non_negative_i32(source: &Map<String, Value>, key: &str) -> Result<i32, ApiError>` helper, eliminating duplicated closures across Gemini and OpenRouter/TypeSafe.

### 2.3 Multi-Endpoint Mounting and Dual-Mode Authentication in `src/gateway.rs`
- Mount four inbound routes in `Gateway::router`:
  - `POST /v1/decisions` -> `Dialect::Openai`
  - `POST /v1beta/decisions` -> `Dialect::Gemini`
  - `POST /v0/decisions` -> `Dialect::Typesafe`
  - `POST /api/v1/decisions` -> `Dialect::Openrouter`
- Extract a helper in `guard` to validate either `Authorization: Bearer <key>` or `x-goog-api-key: <key>` (mutually exclusive, single-valued, fixed-length `ct_eq` comparison).
