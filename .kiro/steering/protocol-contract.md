---
inclusion: fileMatch
fileMatchPattern: "{src/adapter.rs,src/dialect.rs,src/protocol.rs,src/gateway.rs,tests/offline.rs,docs/PROTOCOL.md}"
---

# Protocol Contract Highlights (loaded automatically when editing protocol-related files)

See #[[file:docs/PROTOCOL.md]] for the complete reference. Official schemas are in `reference/` (local, not committed); recheck official documentation before extending a dialect or adapter—do not rely on old assumptions.

## Inbound Dialect Endpoints and Authentication

- `POST /v1/decisions` (`Dialect::Openai`): accepts only the OpenAI Decisions request shape (`model / input / questions[]`).
- `POST /v1beta/decisions` (`Dialect::Gemini`): accepts only the Gemini Decisions request shape (`model / input / questions{}`; question keys are 1..=64 ASCII `[A-Za-z0-9_-]` characters; `choice.criteria` keys are 1..=128 pure ASCII characters and values are strings; `score.criteria` is an array of 2..=10 non-empty strings; `input` supports plain text or a `text / inline_data` array).
- `POST /v0/decisions` (`Dialect::Typesafe`) and `POST /api/v1/decisions` (`Dialect::Openrouter`): accept only the `model / state / questions{}` shape (`choice.criteria` values are strings or `null`; `score.criteria` entries are strings or `{"label", "description"?}`).
- Inbound authentication: when `bearer_key_env` is enabled, support either `Authorization: Bearer <key>` or `x-goog-api-key: <key>` using constant-time comparison. Supplying both authentication headers or duplicate authentication headers returns `401 invalid_api_key`.

## Same-Dialect Native Passthrough

- When the inbound endpoint `Dialect` exactly matches the target candidate `BackendKind` (`Openai↔Openai`, `Gemini↔Gemini`, `Typesafe↔Typesafe`, `Openrouter↔Openrouter`):
  1. **Pass through outbound request**: replace only the top-level `model` in the original request JSON with the configured `backend.model` (for the OpenAI dialect, also remove optional `description: null`); preserve original question keys, option keys, and `criteria` structure without rewriting to `q0/q1` or `decora_choice_*`.
  2. **Inbound multimodal boundary**: even for same-dialect `Gemini↔Gemini`, if a request contains an `inline_data` image and the backend does not enable `capabilities.images: true` (or the adapter's inherent boundary does not support it), intercept before making a network call; do not blindly pass through an unsupported capability.
  3. **Pass through and validate outbound response**: the upstream response must still pass full probability distribution, expectation conservation, `legend` consistency, and usage conservation checks. On success, return the native response structure directly (write the client's public `model` alias to the top-level `model`, **do not inject `usage.decora`**, and preserve native fields such as OpenRouter `usage.cost` and Gemini `usage.input_tokens_by_modality`).
  4. **Strict Usage exemption**: `usage_policy: strict` blocks only candidates requiring cross-protocol usage conversion. When the inbound dialect matches the backend `kind`, usage is complete and native to that dialect, so same-dialect passthrough is allowed in `strict` mode.

## Canonical IR Request Limits

- `questions`: 1–400 items; OpenAI dialect `name` may be omitted, duplicated, or empty, but cannot be explicitly null. For Gemini/TypeSafe/OpenRouter, question names come from `questions` object keys (inherently unique and non-empty).
- `choice`: 2–255 unique options; values are strings or booleans (`true` differs from `"true"`).
- `score`: 2–10 ordered levels, scored by **position 0..N-1**; do not parse numeric labels, which may repeat.
- Roles are system / developer / user / assistant only; `phase` is commentary / final_answer / null.
- `input_json.data` must be an object or array of objects: ≤32 levels, ≤10,000 values, ≤256 KiB.
- Images: `image_url` only (HTTP(S) or PNG/JPEG/WEBP/GIF data URL; Gemini inbound `inline_data` is validated and normalized to a data URL); `file_id` is not accepted. By default, input and all question instructions combined allow ≤32 images, configurable up to 1500; body defaults to 8 MiB, maximum 64 MiB.
- `safety_identifier` ≤128 characters; `prompt_cache_key` ≤64 characters.

## Answer Validation (Must Not Be Relaxed)

- Probabilities and confidence must be finite and in `[0,1]`; confidence need not equal the maximum probability.
- Choice probabilities must cover every original typed option exactly once; the selected option must have the maximum probability (ties are allowed).
- Score probabilities must cover all indices and labels; score = probability-weighted expectation. Sum and expectation tolerance is **0.0001**; exceeding it returns `upstream_protocol_error`. Do not renormalize.
- Preserve OpenAI refusals at `/v1/decisions` as HTTP 200 `type: refusal`; never fallback because of a refusal. If a non-OpenAI inbound endpoint (`/v1beta/decisions`, `/v0/decisions`, `/api/v1/decisions`) receives an upstream per-question `refusal`, return `502 upstream_refusal_unrepresentable` because that dialect cannot express per-question refusals; never fabricate probabilities.

## Heterogeneous Cross-Dialect Internal Mapping Invariants

- Internal re-encoding occurs only when the inbound dialect differs from the outbound backend `kind`:
  - Non-OpenAI native backends (Gemini, OpenRouter, Jev): predicate → `noul`; duplicate/omitted names use internal `q0/q1…` keys and are restored.
  - OpenRouter/Jev: choice → criteria object, score → criteria array. Choice groups containing booleans use internal `decora_choice_0…N-1` keys; preserve the original JSON types of `value`/`description` in criteria and avoid collisions when restoring. Groups containing only strings retain their original string keys.
  - Gemini: use top-level `input` (not `state`) and outbound `x-goog-api-key` authentication. Choice uniformly uses internal `decora_choice_0…N-1` keys and compact JSON criteria strings; score criteria is an ordered array of non-empty strings. The response may omit top-level `model`.
  - Serialize message/JSON/text arrays as one string: `{"decora_encoding":"openai_decisions_text_v1","kind":"messages"|"question_content","data":...}`. Preserve JSON semantics; whitespace and key ordering are not guaranteed.
- Role labels ≠ native instruction hierarchy; JSON boundaries ≠ prompt-injection isolation. Documentation must not imply otherwise.
- `role/type/image_url` keys in JSON data are ordinary data.
- Exclude real `input_image` independently before encoding for Gemini/OpenRouter/Jev; `capabilities.images: true` cannot extend an adapter's inherent capabilities.

## Usage

- Same-dialect native passthrough: validate native usage conservation and return unchanged; do not add `usage.decora`.
- Cross-dialect compatible: preserve real known values and omit unknowns rather than filling with 0; `usage.decora` marks `provider / input_tokens_basis / total_tokens_basis / details_complete`, and uses `provider_cost` when cost is present. TypeSafe uses `billable_input` and does not apply OpenAI cache-inclusion semantics; Gemini uses `total_input_tokens` and may preserve valid `input_tokens_by_modality`; when an OpenAI backend is called by a non-OpenAI inbound endpoint, mark `provider: "openai"` and `input_tokens_basis: "openai_input_tokens"`.
- strict: before the request, exclude all backends that differ from the inbound dialect and cannot provide complete target usage; if no compatible candidate remains, return `400 unsupported_usage`.
- Probability, confidence, and retry rules are identical in all modes.

## Retries and Fallback

- Fallback is allowed only for connection failures, send-phase timeouts, and HTTP 429/500/502/503/504; `max_attempts` is the **total number of calls for the entire request**.
- If the primary candidate (such as a same-dialect passthrough backend) fails with 503/429 and falls back to a heterogeneous backend, the gateway automatically switches seamlessly to cross-dialect adapter conversion, and vice versa (passthrough or conversion is determined dynamically from each attempted candidate's `backend.kind`).
- Wait for `Retry-After` on 429 (integer seconds or HTTP date) before trying the next candidate; if that exceeds the remaining total budget, return 429 immediately with the suggested wait in seconds.
- Do not fallback on client errors, upstream 401/403, refusals, invalid probabilities/usage, redirects, or body-transfer failures after successful headers.
- Upstream 400/404/422 → external 400 `upstream_client_error`; upstream 401/403 → external 502; do not echo the upstream body.
