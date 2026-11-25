# Decora Product Overview

Decora is an independent **multi-dialect Decisions protocol gateway** implemented in Rust. Every public model alias supports four official-shaped inbound endpoints (plus `GET /healthz`):

- `POST /v1/decisions`: OpenAI Decisions dialect (`model / input / questions[]`, question types `predicate / choice / score`)
- `POST /v1beta/decisions`: Google Gemini Decisions dialect (`model / input / questions{}`, question types `noul / choice / score`)
- `POST /v0/decisions`: TypeSafe (Jev) Decisions dialect (`model / state / questions{}`, question types `noul / choice / score`)
- `POST /api/v1/decisions`: OpenRouter Decisions dialect (`model / state / questions{}`, question types `noul / choice / score`)

The gateway uses a **multi-frontend dialect ↔ canonical intermediate representation (Canonical Decision IR) ↔ multi-backend adapters + same-dialect native passthrough** architecture, allowing any Decisions client to connect without modification. When the selected backend matches the inbound dialect, the request is passed through directly (only `model` is replaced); when falling back or routing to a heterogeneous backend, it is automatically converted through the intermediate representation.

## Core principles (understand before making changes)

1. **External contracts for each endpoint are strictly fixed**: each inbound endpoint accepts only its corresponding official dialect request shape and does not mix in fields specific to another dialect (for example, sending `state/criteria` to `/v1/decisions` or a `questions` array to `/v0/decisions` returns `400 invalid_request`). No client-side “conversion strategy” switch is provided.
2. **Same-dialect native passthrough and automatic heterogeneous conversion**:
   - When the inbound endpoint dialect matches the selected outbound backend `kind` (`/v1/decisions` ↔ `openai`, `/v1beta/decisions` ↔ `gemini`, `/v0/decisions` ↔ `typesafe`, `/api/v1/decisions` ↔ `openrouter`), after schema and mathematical invariant validation the gateway **passes through the original request body directly** (replacing only `model` with the backend model ID; it does not rename `q0/q1` or wrap values in `decora_choice_*`) and returns the validated **native response unchanged** (does not inject `usage.decora`, preserves native fields such as `cost` / `input_tokens_by_modality`, and same-dialect passthrough is not blocked by `usage_policy: strict`).
   - When the inbound dialect and selected outbound backend differ (cross-dialect routing or fallback), `dialect` and `adapter` automatically perform lossless conversion through the canonical intermediate representation.
3. **Do not fabricate data**: omit unknown usage details rather than filling them with 0; do not renormalize, clip, or recompute upstream probabilities; do not synthesize missing confidence; unknown answer types are protocol errors. If an OpenAI upstream returns per-question `refusal` that the current inbound dialect (Gemini / TypeSafe / OpenRouter) cannot represent, return `502 upstream_refusal_unrepresentable`; never disguise the refusal as `0.0` or another probability.
4. **Known contract exception (must be stated honestly)**: the default `usage_policy: compatible` may omit usage details unknown to the target dialect and add a `usage.decora` extension during cross-dialect conversion; this is **not a fully native usage schema**. Do not claim in documentation or code comments that “cross-dialect responses are fully identical.” `strict` is a server option, not a client parameter.
5. The public `model` response contains the requested alias; internal backend IDs, URLs, and credentials are not exposed. Inbound authentication supports both `Authorization: Bearer <token>` and `x-goog-api-key: <token>` (only one valid credential may be supplied per request, and it is never forwarded upstream).
6. **Capability filtering remains real**: images, question types, question count, caching, safety parameters, and usage requirements filter candidates before network calls; images are never encoded as text to pretend they were analyzed.

## Scope boundaries

- No database, UI, multi-tenancy, billing, or persistence; no inbound TLS (an external reverse proxy is required).
- Tool-call history is explicitly `unsupported_tool_history`; `box` questions are not implemented; `file_id` images are unsupported.
- Offline verification uses only loopback mocks and has not consumed real API quota. Do not claim real-account availability or calibrated probabilities.

## Language convention

README, docs, comments, commit messages, and error descriptions use **English**; code identifiers use English. Replies to the user should also be in English.
