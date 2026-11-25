# Protocol References & Compatibility Boundaries

Verified date: **2026-11-13**. Implementation follows official provider schemas only—third-party gateways, blog posts, or chat structured-output workarounds are never treated as native Decisions APIs.

**External surface uses symmetric multi-dialect inbound endpoints over a Hub-and-Spoke architecture**: Decora eliminates $M \times N$ pairwise converters via **`Dialect ↔ Canonical Decision IR ↔ Adapter`**:

- `POST /v1/decisions` (OpenAI dialect: `model / input / questions[]`)
- `POST /v1beta/decisions` (Google Gemini dialect: `model / input / questions{}`, supporting `text / inline_data`)
- `POST /v0/decisions` (TypeSafe / Jev dialect: `model / state / questions{}`)
- `POST /api/v1/decisions` (OpenRouter dialect: `model / state / questions{}`)

Each inbound endpoint strictly validates its official dialect schema and rejects fields belonging to other dialects; clients never choose conversion strategies. **One explicit response-level exception remains: cross-dialect `usage_policy: compatible` may omit unknown required breakdowns of the target dialect or OpenAI schema and adds a `usage.decora` provenance object, so it is not a strict drop-in match for every native usage schema.** This server-side policy is retained deliberately rather than fabricating zero values.

## 1. Official Sources

| Provider | Documentation | Machine-Readable Schema | Native POST Endpoint |
|---|---|---|---|
| OpenAI | [Guide](https://developers.openai.com/api/docs/guides/decisions), [Create Reference](https://developers.openai.com/api/reference/resources/decisions/methods/create) | [Official openai-openapi](https://raw.githubusercontent.com/openai/openai-openapi/master/openapi.yaml) | `https://api.openai.com/v1/decisions` |
| Google Gemini | [Decisions API](https://aistudio.google.com/docs/decisions-api) | [AI Studio Official Spec](https://aistudio.google.com/docs/decisions-api) | `https://generativelanguage.googleapis.com/v1beta/decisions` |
| OpenRouter | [Alpha Decisions API](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-questions-and-answers-request) | [Official OpenAPI](https://openrouter.ai/openapi.json) | `https://openrouter.ai/api/alpha/decisions` |
| TypeSafe / Jev | [Official API](https://docs.typesafe.ai/api) | [Official OpenAPI](https://api.typesafe.ai/openapi.json) | `https://api.typesafe.ai/v1/systemone` |

Additional references: [OpenAI Images & Vision Guide](https://developers.openai.com/api/docs/guides/images-vision), [Prompt Caching Guide](https://developers.openai.com/api/docs/guides/prompt-caching). Gemini endpoints authenticate with server-configured `x-goog-api-key`; all other endpoints use server-configured Bearer keys; caller credentials are never forwarded.

Downloaded reference snapshots are stored locally under `reference/` (excluded from git to avoid bloating the repository with full upstream schemas). Recorded SHA-256 checksums:

```text
OpenAI openapi.yaml
2213c5372faf662dd32dc91b2b2acdcf455df609c6fe9ab3a3413674f8b46449
OpenRouter openapi.json
07b8b1ac91ba9e522540417420cd74004db5246d5341e5c86d68443e4438af82
TypeSafe openapi.json
3ca93b6e6b9aa8d7820531b78c66ed7a9798b11e6e163ad96b140fb3d75fbcf4
```

Upstream endpoints and schemas can evolve; re-verify them whenever extending adapter boundaries. OpenRouter's operation-level server is `https://openrouter.ai`, overriding the global `/api/v1` server, so the correct Alpha Decisions path is `/api/alpha/decisions` without duplicating `/api/v1`.

## 2. OpenAI Public Contract

### Request

Required top-level fields are `model`, `input`, and `questions`. `model` must be a configured public alias; OpenAI's documented native model is `gpt-6-luna`.

- `input`: string or ordered message array supporting roles `user`, `developer`, `system`, and `assistant`.
- `user` content: string or array of `input_text / input_image / input_json`.
- `developer` / `system` content: string or array of `input_text`.
- `assistant` content: string or mixed array of `output_text / refusal`; optional `phase` in `commentary | final_answer | null`.
- `input_json.data`: must be a JSON object or array of objects; nested values may be arbitrary JSON. Limits per block: **32 container levels, 10,000 values, 256 KiB compact UTF-8 JSON** (root container counted in depth and value totals).
- `questions`: **1–400** items, answered in input order. Each question requires `instructions`; `name` is optional and may repeat or be an empty string, but cannot be explicitly `null`.
- Question `instructions`: string or non-empty array of `input_text / input_image / input_json`. Cache breakpoints are not supported inside question instructions.
- `predicate`: requires only `instructions`; returns probability that the statement is true.
- `choice`: requires **2–255** unique options `{value: string|boolean, description?: string}`. Boolean `true` and string `"true"` are distinct values.
- `score`: requires **2–10** ordered levels `{label: string, description?: string}`. Level scores are positional `0..N-1`; labels are never parsed as numbers and need not be unique.
- `safety_identifier` and `prompt_cache_key`: optional or `null`, up to 128 and 64 characters respectively.
- `prompt_cache_options`: optional or `null`; supports `mode: implicit|explicit|null`, `ttl: 30m`. Shared input text/image items may carry `prompt_cache_breakpoint: {mode: explicit}` or `null`.
- No `stream`, top-level chat `messages`, `provider`, or `upstream_url` fields; unknown fields are rejected rather than silently dropped.

The official schema also defines function and custom tool-call history items. This version explicitly rejects them with `unsupported_tool_history` instead of flattening tool calls into user text. Other unimplemented content types return `unsupported_content`.

### Images

- Only `input_image.image_url` is accepted; `file_id` is unsupported.
- Accepts HTTP(S) URLs or base64 data URLs with PNG/JPEG/WEBP/GIF MIME declarations. Decora validates data URL MIME syntax and base64 encoding, but does not decode image binaries to inspect dimensions, animation, or magic bytes.
- Images across shared `input` and all question `instructions` are **counted together**. Official general limits are 1,500 images / 512 MB; Decora defaults to 32 images (configurable up to 1,500).
- Default JSON body limit is 8 MiB (configurable up to 64 MiB), conservatively below upstream limits. Decora never fetches remote URLs; remote payload size, patch counts, and context limits are checked upstream. Keys named `image_url` inside `input_json.data` are plain data and are never counted or loaded as images.

### Response

Top-level fields are strictly `model`, `answers`, and `usage`. Decora sets `model` to the client's public alias rather than exposing internal backend model IDs.

`answers` preserves question order; `name` is always present (`null` for unnamed questions).

| Type | Required Answer Fields (besides `type`/`name`) |
|---|---|
| `predicate` | `probability` |
| `choice` | `choice`, `confidence`, `probabilities: [{value, probability}]` |
| `score` | `score`, `confidence`, `probabilities: [{value: integer, label, probability}]` |
| `refusal` | None; official contract has no refusal reason text |

A `refusal` may replace any individual question's answer while other questions still succeed. Decora preserves HTTP 200 and never falls back to another model to bypass refusals.

Validation rules:

- All probabilities and `confidence` must be finite numbers in `[0, 1]`; `confidence` is not required to equal the maximum option probability.
- `choice` probabilities must cover all original typed options with no omissions or duplicates, and the selected `choice` must achieve the maximum probability (ties allowed).
- `score` probabilities must cover all original level indices `0..N-1` and labels with no omissions or duplicates, and `score` must equal the probability-weighted expectation $\sum_i i \cdot p_i$.
- Absolute tolerance for probability sums and score expectations is **0.0001** (`1e-4`). Decora never re-normalizes, clamps, or recomputes upstream numbers; larger deviations return `upstream_protocol_error`.

All **OpenAI native usage** fields are required; the OpenAI adapter always validates them strictly. Cross-provider compatible usage is specified below.

```json
{
  "input_tokens": 100,
  "input_tokens_details": {"cached_tokens": 10, "cache_write_tokens": 20},
  "output_tokens": 4,
  "output_tokens_details": {"reasoning_tokens": 2},
  "total_tokens": 104
}
```

(Contract example, not a live API response.) All counts must be non-negative integers within the schema's `int32` upper bound; `total_tokens == input_tokens + output_tokens`; `cached_tokens + cache_write_tokens <= input_tokens` (prompt caching docs confirm read/write counts are disjoint and included in `input_tokens`); `reasoning_tokens <= output_tokens`. Missing fields or invalid counts are rejected rather than zero-filled.

## 3. Default Compatibility Policy & Capability Matrix

All four adapters implement real HTTP transports; end-to-end loopback mocks verify request paths, auth headers (`Bearer` or `x-goog-api-key`), `input`/`state` and `questions` mapping, core answers, and usage. **In cross-dialect `compatible` mode, `usage` is an explicit partial-compatibility contract** rather than a claim of full metadata isomorphism.

| Capability | OpenAI Native | Google Gemini Native | OpenRouter Native | TypeSafe / Jev Native |
|---|---|---|---|---|
| Default HTTP execution | **Native structure** | **Text / roles / JSON / typed choices across all 3 question types** (top-level `input`, `x-goog-api-key`) | **Text / roles / JSON / typed choices across all 3 question types** (top-level `state`, `Bearer`) | Same as OpenRouter |
| `usage_policy: strict` (cross-dialect) | Full OpenAI usage supported | Filtered before network (`unsupported_usage`) | Filtered before network (`unsupported_usage`) | Filtered before network (`unsupported_usage`) |
| Plain-text `predicate` | Verbatim, `probability` | `predicate ↔ noul`, `noul → probability` | Same | Same |
| Plain-text `choice` (string values) | Verbatim, including descriptions | Mapped to `decora_choice_0..N-1` + compact JSON criteria string | `choices → criteria` object; probability map → array | Same as OpenRouter |
| Mixed boolean/string `choice` | Native typed distinction | Unified `decora_choice_0..N-1` keys + compact JSON criteria; lossless type restoration | Group-wide internal indexed keys; exact typed restoration | Same as OpenRouter |
| Plain-text `score` | Verbatim ordered levels/descriptions | `levels →` ordered non-empty `string[]` criteria; original labels & weighted score restored | `levels → criteria` array; `legend`/indices → original labels | Same as OpenRouter |
| Duplicate / omitted `name` | Supported | Internal `q0`/`q1` keys; restores original names & order | Same | Same |
| Role messages / JSON parts | Native structure & role hierarchy | Deterministic internal encoding into top-level `input` string | Deterministic internal encoding into top-level `state` string | Same as OpenRouter |
| Native role instruction hierarchy | Enforced by OpenAI | Text labels do not provide native hierarchy guarantees | Text labels do not provide native hierarchy guarantees | Same |
| `input` / question images | Supported with shared count limit | Filtered up front; other compatible candidates may serve | Same | Same (`danger-zone` not enabled) |
| `refusal` | Supported, preserved, never retried | No per-question refusal; batch safety 400 → `upstream_client_error` | No `refusal` in union; never fabricated, unknown type errors without retry | Same as OpenRouter |
| Prompt cache & `safety_identifier` | Supported when backend declares capability | Unsupported; filtered up front | Filtered up front, never silently dropped | Same |
| `usage` breakdowns | Full strict validation | Requires `total_input`/`total_output(0)`/`total_tokens`; unknown breakdowns omitted | Preserves upstream breakdowns; omits unknown | Only `input`/`output`; unknown omitted |
| `usage` provenance tag | Native OpenAI contract, no extension | `usage.decora` marks `total_input_tokens` + optional modality | `usage.decora` marks `provider_reported` | `usage.decora` marks `billable_input` |
| Probability calibration | Passes through native estimates | Same; never chat-completion JSON | Same | Same |

### Server Setting `usage_policy: compatible` (Default)

OpenRouter's `DecisionsResponse.usage` requires `input_tokens`/`output_tokens` with optional `cost` and `input_tokens_details.cached_tokens`. TypeSafe's `Usage` contains only `input_tokens`/`output_tokens`, where `input_tokens` is explicitly **billable input tokens**. These omissions do not block core decision calls:

1. `input_tokens` and `output_tokens` use real non-negative upstream integers within `int32`; `total_tokens` is their sum. If an upstream also returns `total_tokens`, it must match that sum rather than silently overriding contradictory counts.
2. Actually reported `cached_tokens`, `cache_write_tokens`, and `reasoning_tokens` are preserved; unknown breakdowns are omitted (never zero-filled), and `*_details` objects are emitted only when at least one breakdown is known. OpenAI/OpenRouter cached counts are checked against total input; TypeSafe `billable_input` is not guaranteed to include all cached tokens, so OpenAI's upper bound is not misapplied to it.
3. Cross-dialect responses include `usage.decora`, exposing fixed provider names (never internal backend IDs, URLs, or credentials): `provider`, `input_tokens_basis`, `total_tokens_basis`, `details_complete`, plus `provider_cost` when present. Arbitrary upstream metadata and error bodies are never passed through.
4. The OpenAI adapter always enforces its full official usage contract; `compatible` never relaxes validation of OpenAI responses.

Compatible TypeSafe example:

```json
{
  "input_tokens": 120,
  "output_tokens": 12,
  "total_tokens": 132,
  "decora": {
    "provider": "typesafe",
    "input_tokens_basis": "billable_input",
    "total_tokens_basis": "billable_input_plus_output",
    "details_complete": false
  }
}
```

Here `132` is **billable input plus output** and must not be equated with exact OpenAI total tokens, full context length, or cross-provider billing parity. OpenRouter tags `provider: openrouter`, `input_tokens_basis: provider_reported`, `total_tokens_basis: provider_input_plus_output`. Google Gemini tags `provider: gemini`, `input_tokens_basis: total_input_tokens`, `total_tokens_basis: total_input_plus_output`, preserving valid `input_tokens_by_modality` inside `usage.decora`. `details_complete` only indicates whether all three breakdown fields were reported.

### Server Setting `usage_policy: strict` (Optional)

Requires complete target-dialect usage and accounting semantics. **Same-dialect passthrough candidates** (`Openai↔Openai`, `Gemini↔Gemini`, `Typesafe↔Typesafe`, `Openrouter↔Openrouter`) are permitted because they return the caller's exact native usage untouched. Cross-dialect non-OpenAI candidates are filtered out before network calls (returning `400 unsupported_usage` if no compatible candidate remains). Probability, confidence, score expectation, refusal, and retry rules are identical under `compatible` and `strict`.

### Native HTTP Translation Scope

`adapter::native_body_for`, `native_answers_for`, and `native_response` power the live gateway HTTP path:

- Native requests use `input` (Gemini) or `state` (OpenRouter/TypeSafe) with keyed `questions`, never OpenAI's `questions[]` or chat completions. String `input`/`instructions` pass verbatim; role messages and text/JSON content arrays are deterministically text-encoded.
- Score `criteria`: OpenRouter/TypeSafe use label strings when no description exists, or `{label, description}` objects when descriptions exist; Gemini requires non-empty `string[]`, serializing empty labels or described levels as compact JSON strings. Returned `legend` must match `criteria` exactly.
- Choice `criteria`: Gemini constrains option keys to 1..=128 ASCII characters with string descriptions, so Decora uses `decora_choice_0..N-1` plus compact JSON strings to support Unicode, empty strings, booleans, and colliding string literals losslessly.
- Top-level `model`: Gemini native responses omit top-level `model` (if present, it must be a non-empty string); Decora always outputs the requested public `model` alias.
- OpenRouter declares choice/score `confidence`, `probabilities`, and score `legend` as optional in its OpenAPI spec, but complete core answers require them. Missing fields return `upstream_protocol_error` rather than fabricating values or guessing missing `legend` mappings.
- Images: Gemini instructions are text-only and shared input images require `mime_type`, while Decora's security policy forbids fetching remote images or guessing MIME types; TypeSafe images depend on assets and `danger-zone`; OpenRouter varies by provider. Non-OpenAI backends therefore exclude real `input_image` up front.
- `box` questions are not implemented because OpenAI's contract has no equivalent type. Undefined native refusals are never mapped to `0`/`false` and never retried on another model.

## 4. Hub-and-Spoke Multi-Dialect Architecture & Inbound Mapping

### Symmetric Three-Stage Pipeline (with Same-Dialect Passthrough Branch)

```text
  POST /v1/decisions       (OpenAI)     ──┐
  POST /v1beta/decisions   (Gemini)     ──┼─► Dialect::parse_request ──► Canonical IR
  POST /v0/decisions       (TypeSafe)   ──┤                                    │
  POST /api/v1/decisions   (OpenRouter) ──┘                                    ▼
                                                            Gateway::execute (per-attempt routing)
                                                                ├─ Same-dialect passthrough: Dialect::passthrough_body/response (keys preserved, no usage.decora)
                                                                └─ Cross-dialect translation: Adapter + Dialect::format_response
```

- **Same-Dialect Passthrough**: Evaluated per attempt against the selected backend. When `dialect.matches_backend(backend.kind)` is true, Decora clones the validated raw JSON and replaces only top-level `model`, preserving original `questions` keys, option keys, and `criteria`. The response still undergoes full probability sum, max-probability choice, weighted score expectation, `legend` consistency, and token conservation checks, then returns natively (without `usage.decora`, preserving native `cost` and `input_tokens_by_modality`). If a same-dialect primary fails with `503`/`429`/timeout and falls back to a heterogeneous backup, the next attempt automatically switches to Canonical IR translation.

| Inbound Endpoint | Dialect | Shared Input | Question Container & Types | Refusal & Usage Handling |
|---|---|---|---|---|
| `POST /v1/decisions` | `Dialect::Openai` | String or role message array (`input_text`/`input_image`/`input_json`) | Array `questions[]` (`predicate / choice / score`) | Preserves per-question `refusal`; OpenAI backend omits `usage.decora` |
| `POST /v1beta/decisions` | `Dialect::Gemini` | String or `text / inline_data` array (normalized to `user` message) | Map `questions{}` with keys `1..=64` ASCII `[A-Za-z0-9_-]` (`noul / choice / score`); `choice` keys `1..=128` trimmed ASCII | Upstream per-question `refusal` → `502 upstream_refusal_unrepresentable`; outputs `total_input_tokens / total_output_tokens / total_tokens` + optional `input_tokens_by_modality` + `usage.decora` when cross-dialect (`provider: openai`) |
| `POST /v0/decisions` | `Dialect::Typesafe` | String `state` | Map `questions{}` with non-empty string keys (`noul / choice / score`); `choice` supports Unicode keys & `null` descriptions; `score` supports `{label, description}` | Upstream per-question `refusal` → `502 upstream_refusal_unrepresentable`; `score.legend` restores exact `criteria`; outputs `input_tokens / output_tokens / total_tokens` + `usage.decora` when cross-dialect |
| `POST /api/v1/decisions` | `Dialect::Openrouter` | String `state` | Same as TypeSafe dialect | Same as TypeSafe dialect |

### External Invariants & Internal Adaptation

All public aliases are shared across all four endpoints; translation is strictly internal with no client conversion flags.

- Strict dialect boundaries: sending `state` or object `questions` to `/v1/decisions`, sending `state` or array `questions` to `/v1beta/decisions`, or sending `input` or array `questions` to `/v0/decisions` / `/api/v1/decisions` returns `400 invalid_request`.
- Message arrays accept only `system`, `developer`, `user`, and `assistant`, preserving order, role tags, `phase`, and string/array content shapes without inventing custom roles or unsupported fields.
- String content (including empty strings) is preserved verbatim; `input_text`/`output_text`/`refusal` and `input_json.data` are encoded as data. Empty message/content arrays, Unicode, and nested JSON are supported within request size and depth limits.
- String `instructions` pass verbatim; text/JSON content arrays are encoded automatically across `predicate`, `choice`, and `score`.
- Mixed boolean/string choices map reversibly and restore exact original JSON types.
- Images, question types, question counts, prompt caching, and `safety_identifier` are filtered against real backend capabilities; `usage_policy` remains an operator setting.

### Explicit Encoding & Semantic Boundaries

For text-only upstreams, Decora serializes message arrays as a JSON string of the following object (question content uses `"kind": "question_content"`). Clients never submit this envelope:

```json
{
  "decora_encoding": "openai_decisions_text_v1",
  "kind": "messages",
  "data": [{"role":"user","content":[{"type":"input_json","data":{"flag":false,"nested":[null,1,"Unicode"]}}]}]
}
```

JSON escaping establishes explicit data boundaries and preserves JSON types rather than concatenating unescaped delimiters. Object key order and original whitespace/formatting are not guaranteed. Role labels and ordering remain visible to the model, **but non-OpenAI upstreams do not enforce OpenAI's native role instruction hierarchy**, and JSON encoding is not prompt-injection isolation. Deployments requiring native role hierarchy should route to backends that provide it.

Real `input_image` parts are inspected before encoding; even if an operator sets `images: true` on a non-OpenAI native backend, adapter image boundaries cannot be bypassed. Both shared input and question images are rejected up front rather than stringified. Keys named `role`, `type`, or `image_url` inside `input_json.data` remain plain data and never trigger image filtering or role creation. Cache breakpoints and `safety_identifier` are never downgraded to text.

### Reversible Mapping for Mixed Boolean/String Choices

Whenever a `choice` group contains any boolean option, all options in that group are keyed as `decora_choice_0..N-1` with criteria `{value: <original typed JSON>, description?: <string>}`:

```json
{
  "decora_choice_0": {"value":true,"description":"boolean true"},
  "decora_choice_1": {"value":"true","description":"string true"},
  "decora_choice_2": {"value":false,"description":"boolean false"},
  "decora_choice_3": {"value":"decora_choice_0","description":"literal string colliding with internal key"}
}
```

Because every option in the group uses the indexed map, `true` vs. `"true"`, `false` vs. `"false"`, literal `"decora_choice_0"`, and empty strings never collide. `false` is an explicit boolean value, never missing/default. Upstream `choice` and `probabilities` keys must match the internal keys, are restored to their original JSON types and question order, and are then validated for coverage, sum, and max-probability consistency. Unknown keys, missing options, or unexpected native boolean choices are rejected. String-only groups on TypeSafe/OpenRouter keep their original string keys.

## 5. Minimum Error & Routing Contract

- Missing/invalid inbound key: `401` with fixed error envelope and `WWW-Authenticate: Bearer`.
- Unknown alias: `404`; invalid schema or unsupported capability: `400`; body too large: `413`; non-JSON content type: `415`.
- Upstream `400`/`404`/`422`: mapped to `400 upstream_client_error` without echoing sensitive upstream bodies; upstream `401`/`403` is treated as a server misconfiguration (`502`).
- Upstream `429`: honors `Retry-After` before the next attempt, or returns `429` with the remaining suggested delay if the budget would be exceeded.
- Compatible upstreams exhausted: `503`; overall timeout: `504`; invalid upstream JSON, probabilities, or usage: `502`.
- HTTP redirects are never followed; response body read failures after headers arrive are never retried to avoid duplicate billing and semantic drift.
- `priority` and `weight` are server-side only; user input never selects arbitrary URLs. `/healthz` returns only fixed liveness status.
