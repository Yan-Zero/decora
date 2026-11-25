# Requirements: Native Google Gemini Decisions Backend Support (`gemini-backend`)

## Background and Goals

Google has released the native **Gemini Decisions API** (model `gemini-decision-1`, endpoint `POST https://generativelanguage.googleapis.com/v1beta/decisions`). In a single forward pass, it computes `noul` (binary truth probabilities), `choice` (multiple-choice probability distributions), and `score` (weighted scores over ordered levels), without decoding output text tokens (`total_output_tokens` is always `0`).

This spec adds native `kind: gemini` backend support without changing Decora’s fixed external contract (`POST /v1/decisions`, OpenAI-shaped `model / input / questions` requests, and the core `predicate / choice / score / refusal` answers). It strictly follows the **no fabricated data** and **actual capability filtering** principles in `.kiro/steering/protocol-contract.md` and `.kiro/steering/security.md`.

---

## Verified Official Protocol Details (based on `https://aistudio.google.com/docs/decisions-api`)

1. **Entry point and authentication**:
   - Fixed official entry point: `POST https://generativelanguage.googleapis.com/v1beta/decisions`
   - Authentication: HTTP header `x-goog-api-key: $GEMINI_API_KEY` (not `Authorization: Bearer`).
2. **Request top-level structure**:
   - Required fields: `model` (e.g. `"gemini-decision-1"`), `input` (string or content block — **note the field is `input`, not OpenRouter's `state`**), `questions` (`1..=1000` question key-value map; keys must be `1..=64` ASCII `[A-Za-z0-9_-]+`).
3. **Question types and `criteria` constraint differences**:
   - `noul`: corresponds to OpenAI `predicate`; contains `type: "noul"` and text `instructions`.
   - `choice`: `criteria` is a map from option keys to description strings. **Key constraint**: Gemini requires option keys to be `1..=128` characters of pure ASCII (no leading/trailing whitespace, no non-ASCII such as Chinese, no empty string, no control characters), and every description value must be a **string** (empty string `""` when there is no description; `null` and nested objects are not supported).
   - `score`: `criteria` must be an **ordered array of `2..=255` non-empty strings (`string[]`)**; `{label, description}` objects are not supported.
4. **Images, caching, safety identifier and refusal boundaries**:
   - **Images**: Gemini question `instructions` **only support plain text, not images inside questions**; images in the shared `input` block require a `mime_type` matching a real file format. Because OpenAI `input_image.image_url` with a remote HTTP(S) URL carries no `mime_type`, and Decora's security policy forbids the gateway from downloading remote images or guessing/fabricating unknown MIME types, and does not support downgrading images inside questions, `kind: gemini` filters real `input_image` before the request according to its fixed capability boundary and returns `unsupported_native_images`.
   - **Caching and safety identifier**: the official Gemini Decisions API explicitly does not support context caching (`prompt_cache`), nor OpenAI's `safety_identifier`. As with OpenRouter/TypeSafe, these are filtered early and return `unsupported_capability` or `unsupported_native_mapping`.
   - **Refusal (per-question refusal)**: Gemini's official response has no per-question `refusal` answer type (safety blocks return a whole-batch HTTP `400 Bad Request`). Upstream 400s map to the external `400 upstream_client_error` under existing rules with no fallback; undefined answer types return `502 upstream_protocol_error`.
5. **Response top level and usage semantics**:
   - Gemini's response top level contains only `answers` and `usage`, with **no top-level `model` field** (Decora's external response `model` is always the public alias requested by the client).
   - Gemini `usage` fields:
     - `total_input_tokens` (non-negative integer; billed input tokens)
     - `total_output_tokens` (non-negative integer; officially always `0`)
     - `total_tokens` (non-negative integer; equals `total_input_tokens + total_output_tokens`)
     - `input_tokens_by_modality` (optional array: `[{"modality": string, "tokens": integer}]`)

---

## Requirements

### Requirement 1: Configuration and backend kind extension (`BackendKind::Gemini`)
- **Acceptance criterion 1.1**: `BackendKind` gains `gemini` (YAML `kind: gemini`); the default official endpoint is `https://generativelanguage.googleapis.com/v1beta/decisions`.
- **Acceptance criterion 1.2**: Configuration validation matches existing backends: validate the `api_key_env` environment variable name, HTTPS (or explicitly allowed loopback IP HTTP) endpoint rules, question types and the `max_questions` (`1..=400`) range.
- **Acceptance criterion 1.3**: When sending requests to `kind: gemini`, the gateway passes the backend's resolved key via the `x-goog-api-key` header, does not send `Authorization: Bearer`, and never forwards client inbound credentials.

### Requirement 2: Fixed external contract and lossless internal conversion
- **Acceptance criterion 2.1**: For a public alias bound to `kind: gemini`, clients still send exactly the same OpenAI `POST /v1/decisions` request (supporting string or role-message-array `input`, in-question `input_text / input_json`, Chinese/empty-string/boolean mixed `choice`, and `score` with optional descriptions).
- **Acceptance criterion 2.2**: The Gemini request body carries the shared input in the top-level `"input"` field (a string is passed through as-is; a role-message array is encoded as a string using the existing `openai_decisions_text_v1` JSON text encoding).
- **Acceptance criterion 2.3**: Question keys always use ASCII-safe `q0..qN-1`.
- **Acceptance criterion 2.4**: Because Gemini `choice.criteria` only allows non-empty pure-ASCII keys and string description values, while OpenAI `choice` allows Chinese strings (e.g. `"售后"`), empty strings, leading/trailing whitespace and booleans (`true` / `"true"`), the Gemini backend uniformly uses internal ASCII keys `decora_choice_0..N-1`, encodes the original typed `value` and optional `description` as string descriptions, and on response parsing maps by index to restore the exact original `ChoiceValue` (including `false`, `"false"`, Chinese, empty strings and strings identical to internal keys).
- **Acceptance criterion 2.5**: Because Gemini `score.criteria` requires a non-empty `string[]` array, while OpenAI `score.levels` allows empty labels or levels with a `description`, the Gemini backend serializes each level as a non-empty string criterion, and after validating response `legend` consistency, restores the original `label` and probability distribution exactly by position `0..N-1`.

### Requirement 3: Capability filtering and security boundaries
- **Acceptance criterion 3.1**: Requests containing real `input_image` (in the shared `input` or in question `instructions`) exclude `kind: gemini` candidates before encoding; if no other compatible candidate exists they return `400 unsupported_native_images` (even when `capabilities.images: true` is configured, the adapter's fixed capabilities are not expanded).
- **Acceptance criterion 3.2**: `"image_url"`, `"type"` and `"role"` keys inside `input_json.data` remain ordinary JSON data and do not trigger image blocking.
- **Acceptance criterion 3.3**: Requests carrying `prompt_cache_*` or `safety_identifier` are excluded before being sent to Gemini.

### Requirement 4: Usage compatibility, strict mode and the "no fabricated data" principle
- **Acceptance criterion 4.1**: Under `usage_policy: strict`, `kind: gemini` is excluded before the request is sent; if no compatible candidate exists, the gateway returns `400 unsupported_usage`.
- **Acceptance criterion 4.2**: Under `usage_policy: compatible`, strictly validate that Gemini `usage` `total_input_tokens`, `total_output_tokens` and `total_tokens` are non-negative `i32` integers and that `total_tokens == total_input_tokens + total_output_tokens`; map them to top-level `input_tokens`, `output_tokens` and `total_tokens`, omitting unknown `input_tokens_details` and `output_tokens_details` (no fabricated 0).
- **Acceptance criterion 4.3**: Mark in `usage.decora`:
  - `"provider": "gemini"`
  - `"input_tokens_basis": "total_input_tokens"`
  - `"total_tokens_basis": "total_input_plus_output"`
  - `"details_complete": false`
  - If the upstream returns valid `input_tokens_by_modality` (each entry with a non-empty string `modality` and non-negative `i32` `tokens`), keep it as-is in `usage.decora.input_tokens_by_modality`; invalid formats return `502 upstream_protocol_error`.
- **Acceptance criterion 4.4**: Gemini responses need not contain a top-level `model` field; if present it must be a non-empty string. The final external response `model` is always the public alias requested by the client.
