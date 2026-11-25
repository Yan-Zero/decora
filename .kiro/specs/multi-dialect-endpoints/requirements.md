# Requirements: Multi-Dialect Inbound Endpoints and Symmetric Hub-and-Spoke Protocol Architecture (`multi-dialect-endpoints`)

## Background and Goals

In previous versions, Decora could route OpenAI Decisions requests to four native upstreams—OpenAI, Google Gemini, OpenRouter, and TypeSafe (Jev)—but the only inbound endpoint was the OpenAI-shaped `POST /v1/decisions`. `src/protocol.rs` and `src/adapter.rs` also coupled the frontend request format with the internal intermediate representation.

This spec aims to:
1. **Improve code layering**: Add `src/dialect.rs` and streamline `src/adapter.rs` to build a symmetric **multi-frontend dialect (Dialect) ↔ canonical internal representation (Canonical Decision IR) ↔ multi-backend adapter (Adapter)** hub-and-spoke architecture, eliminating $M \times N$ pairwise conversions and duplicate validation.
2. **Provide native inbound endpoints for four official request shapes**:
   - `POST /v1/decisions`: OpenAI Decisions dialect
   - `POST /v1beta/decisions`: Google Gemini Decisions dialect
   - `POST /v0/decisions`: TypeSafe (Jev) Decisions dialect
   - `POST /api/v1/decisions`: OpenRouter Decisions dialect
3. **Support dual-mode inbound authentication**: Accept both `Authorization: Bearer <token>` and `x-goog-api-key: <token>`, allowing SDKs and clients from different ecosystems to connect without modification and route across any compatible upstream with fallback.

---

## Requirements

### Requirement 1: Refactor to a Symmetric Hub-and-Spoke Code Structure
- **Acceptance criterion 1.1**: Add `src/dialect.rs` and define the `Dialect` enum (`Openai`, `Gemini`, `Typesafe`, `Openrouter`), encapsulating inbound request parsing in `Dialect::parse_request` and outbound response formatting in `Dialect::format_response`.
- **Acceptance criterion 1.2**: Refactor `src/adapter.rs` to eliminate duplicated code (e.g., extract shared `score_criterion`, `choice_key_for`, and non-negative `i32` token parsing helpers), so `native_answers_for` no longer needs to call the full `native_body_for` again.
- **Acceptance criterion 1.3**: Preserve the existing public symbol signatures of `decora::protocol`, `decora::adapter`, `decora::gateway`, and `decora::config` for backward compatibility, ensuring all 38 existing offline tests pass without compromise.

### Requirement 2: Multi-Dialect Inbound Endpoints and Strict Schema Boundaries
- **Acceptance criterion 2.1**: Bind each inbound endpoint strictly to its official dialect schema; never infer or mix fields from other dialects on the same path:
  - Sending `state`, `criteria`, or object-valued `questions` to `/v1/decisions` returns `400 invalid_request`.
  - Sending `state`, array-valued `questions`, or `predicate` question types to `/v1beta/decisions` returns `400 invalid_request`.
  - Sending `input`, array-valued `questions`, or `predicate` question types to `/v0/decisions` or `/api/v1/decisions` returns `400 invalid_request`.
- **Acceptance criterion 2.2**: The Gemini inbound endpoint (`POST /v1beta/decisions`) supports:
  - Top-level `model`, `input` (plain text or a multimodal content array of `{"text": ...}` and `{"inline_data": {"mime_type": ..., "data": ...}}` entries), and `questions` (an object with `1..=400` entries and keys of `1..=64` ASCII `[A-Za-z0-9_-]` characters).
  - `noul`, `choice` (an object with `2..=255` entries, keys that are `1..=128` plain ASCII characters without leading or trailing whitespace, and string descriptions as values), and `score` (a `criteria` array of `2..=10` non-empty strings).
  - Responses return `model`, an `answers` object keyed by the original question keys (containing `noul`, `choice + confidence + probabilities`, or `score + confidence + probabilities + legend`), and Gemini-shaped `usage` (`total_input_tokens / total_output_tokens / total_tokens`, with optional `input_tokens_by_modality` and `decora`).
- **Acceptance criterion 2.3**: The TypeSafe (`POST /v0/decisions`) and OpenRouter (`POST /api/v1/decisions`) inbound endpoints support:
  - Top-level `model`, `state` (plain text), and `questions` (an object with `1..=400` entries and non-empty string keys).
  - `noul`, `choice` (an object with `2..=255` entries, arbitrary non-empty string keys including Chinese characters, and string or `null` descriptions), and `score` (an array with `2..=10` elements, each either a non-empty string `label` or `{"label": ..., "description"?: ...}`).
  - Responses return `model`, an `answers` object keyed by the original question keys (`score.legend` exactly reproduces the `criteria` entries requested by the caller), and `usage` (`input_tokens / output_tokens / total_tokens`, with optional `input_tokens_details / output_tokens_details` and `decora`).

### Requirement 3: Cross-Dialect Routing, Honest Refusal Boundaries, and Transparent Usage
- **Acceptance criterion 3.1**: Any inbound dialect endpoint can route to any compatible backend (`openai`, `gemini`, `openrouter`, `typesafe`). If the primary candidate returns 429/5xx or a connection timeout, automatically fall back to the next-priority candidate (for example, a Gemini inbound request falls back from a Gemini upstream returning 503 to an OpenAI or TypeSafe backend, then returns a Gemini-formatted response).
- **Acceptance criterion 3.2**: If a non-OpenAI inbound endpoint (`/v1beta/decisions`, `/v0/decisions`, `/api/v1/decisions`) routes to an OpenAI backend and that backend returns a per-question `type: "refusal"`, the gateway must return `502 upstream_refusal_unrepresentable` because the target dialect has no per-question refusal representation. It must never fabricate `0.0` or another probability, and must not trigger cross-model fallback.
- **Acceptance criterion 3.3**: When a non-OpenAI inbound endpoint routes to an OpenAI backend under `usage_policy: compatible`, `usage.decora` transparently marks `"provider": "openai"`, `"input_tokens_basis": "openai_input_tokens"`, `"total_tokens_basis": "openai_input_plus_output"`, and `"details_complete": true`.

### Requirement 4: Dual-Mode Inbound Authentication (`Authorization: Bearer` and `x-goog-api-key`)
- **Acceptance criterion 4.1**: When `bearer_key_env` is configured, all four Decisions inbound endpoints accept either `Authorization: Bearer <key>` or `x-goog-api-key: <key>` using a single fixed-length comparison (`ConstantTimeEq`).
- **Acceptance criterion 4.2**: If a request includes both `Authorization` and `x-goog-api-key`, repeats an authentication header with the same name, or provides a mismatched credential, immediately return `401 invalid_api_key` (with `WWW-Authenticate: Bearer`) before reading the request body. Never forward client inbound credentials to any upstream.
