# Decora

**Decora is a standalone, single-tenant multi-dialect Decisions protocol gateway written in Rust.** Every public model alias is served across all four official inbound endpoint dialects (plus `GET /healthz`) using a symmetric **Hub-and-Spoke architecture (`Dialect ↔ Canonical Decision IR ↔ Adapter`)**, allowing SDKs and clients from different ecosystems to route and fail over across any compatible native upstream without rewriting requests:

- `POST /v1/decisions`: OpenAI Decisions dialect (`model / input / questions[]`, question types `predicate / choice / score`)
- `POST /v1beta/decisions`: Google Gemini Decisions dialect (`model / input / questions{}`, question types `noul / choice / score`, supporting plain text and `text / inline_data` multimodal arrays)
- `POST /v0/decisions`: TypeSafe (Jev) Decisions dialect (`model / state / questions{}`, question types `noul / choice / score`)
- `POST /api/v1/decisions`: OpenRouter Decisions dialect (`model / state / questions{}`, question types `noul / choice / score`)

Each inbound endpoint strictly validates its official dialect schema (for example, sending `state` to `/v1/decisions` or sending an array `questions` to `/v0/decisions` returns `400 invalid_request`), with no client-side "conversion strategy" flags. Protocol translation between OpenAI, Gemini, OpenRouter, and TypeSafe/Jev happens automatically inside the gateway—never via chat completions and never by fabricating probabilities.

Text, role messages, JSON parts, and mixed boolean/string choices are adapted automatically; real capabilities such as images are still filtered up front. **One explicit contract exception remains for `usage`**: under the default server-side `usage_policy: compatible`, Decora preserves verified known counts, omits unknown breakdowns, and attaches `usage.decora` provenance metadata—**this is not a strict drop-in match for every native target dialect or OpenAI usage schema**. TypeSafe `billable_input` and Gemini `total_input_tokens` are never disguised as exact total input tokens, and unknown breakdowns are never zero-filled. Operators can enable `usage_policy: strict` on the server; this is not a client request parameter. See the [Protocol Reference & Capability Matrix](docs/PROTOCOL.md).

## Quick Start

Requires the Rust 2024 toolchain (verified locally with `rustc/cargo 1.95.0`). No database, UI, or external service is required.

```bash
cp config.example.yaml config.yaml
# Set OPENAI_API_KEY and DECORA_BEARER_KEY securely in your process or service manager.
# The example config stores environment variable names only; plaintext api_key values are not supported in YAML.
cargo run --locked --release -- config.yaml
```

By default, Decora binds to `127.0.0.1:8787`. `config.yaml`, `.env*`, logs, and build artifacts are listed in `.gitignore`, and the binary **does not auto-load `.env`**. For example, in Git Bash you can enter credentials interactively to avoid recording secrets in shell history:

```bash
read -r -s -p 'OpenAI API key: ' OPENAI_API_KEY; printf '\n'
export OPENAI_API_KEY
read -r -s -p 'Decora inbound key: ' DECORA_BEARER_KEY; printf '\n'
export DECORA_BEARER_KEY
cargo run --locked --release -- config.yaml
```

Use a dedicated, high-entropy random value for `DECORA_BEARER_KEY` rather than reusing an upstream API key. Inbound authentication accepts either `Authorization: Bearer $DECORA_BEARER_KEY` or `x-goog-api-key: $DECORA_BEARER_KEY` (only one valid credential header per request; supplying both or duplicate headers returns `401 invalid_api_key`, and inbound keys are never forwarded upstream). All configured backend credentials are resolved at startup; missing, blank, or newline-containing values prevent startup, and secret values are never printed.

```bash
curl http://127.0.0.1:8787/healthz
# {"status":"ok"}; liveness only—never queries upstreams or exposes configuration.

curl http://127.0.0.1:8787/v1/decisions \
  -H "Authorization: Bearer $DECORA_BEARER_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "decora-luna",
    "input": "The package arrived with a cracked shell; please replace it as soon as possible.",
    "questions": [
      {"type":"predicate","name":"damaged","instructions":"Is the package damaged?"},
      {"type":"choice","name":"team","instructions":"Which team should handle this?",
       "choices":[{"value":"support"},{"value":"sales"}]},
      {"type":"score","name":"urgency","instructions":"Rate handling urgency by level.",
       "levels":[{"label":"Low"},{"label":"High","description":"Needs handling today"}]}
    ]
  }'
```

Score questions are scored by **level position `0..N-1`**, not by parsing numeric values from labels. `confidence` and the maximum class probability are independent fields. Probabilities are the upstream model's native estimates; Decora does not claim they are calibrated on your production data.

### Usage Policy & Explicit Contract Exception

The following is a **server-side `config.yaml` operator setting**, not a request parameter. Compatible `usage` is intentionally not coerced into a strict OpenAI schema: omitting unknown required breakdowns and adding `usage.decora` are explicit, documented differences.

- `compatible` (default): All four upstreams (OpenAI, Gemini, OpenRouter, TypeSafe) can participate in routing. Native OpenAI responses are still strictly validated against OpenAI's full usage schema; same-dialect passthrough (`/v1beta/decisions -> gemini`, `/v0/decisions -> typesafe`, `/api/v1/decisions -> openrouter`) returns native `usage` verbatim **without injecting `usage.decora`**; cross-dialect routing preserves known counts/breakdowns, omits unknown fields, and attaches `usage.decora`.
- `strict`: Requires complete, exact target-dialect usage. **Same-dialect passthrough candidates** (`/v1/decisions -> openai`, `/v1beta/decisions -> gemini`, `/v0/decisions -> typesafe`, `/api/v1/decisions -> openrouter`) are allowed and returned verbatim; cross-dialect non-OpenAI candidates are rejected before any network call (`400 unsupported_usage`) or skipped in favor of compatible candidates.

For example, a cross-dialect TypeSafe response can include the following `usage` (contract example, not a live call log). Here `132` is **billable input plus output**, not verified OpenAI total tokens:

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

Google Gemini native responses do not report cached or reasoning token breakdowns (`total_output_tokens` is always `0`), so cross-dialect `usage` maps to:

```json
{
  "input_tokens": 120,
  "output_tokens": 0,
  "total_tokens": 120,
  "decora": {
    "provider": "gemini",
    "input_tokens_basis": "total_input_tokens",
    "total_tokens_basis": "total_input_plus_output",
    "details_complete": false,
    "input_tokens_by_modality": [
      {"modality": "text", "tokens": 120}
    ]
  }
}
```

OpenRouter uses `provider_reported / provider_input_plus_output`; when `cached_tokens` is provided upstream, it is preserved without fabricating `cache_write_tokens`. When upstream `cost` is present, it is preserved as `usage.decora.provider_cost`. Deployments requiring exact OpenAI usage schemas can set `usage_policy: strict`; clients must never interpret omitted breakdown fields as `0`.

### Same-Dialect Passthrough & Automatic Cross-Dialect Translation

**Clients always send the native Decisions request for their chosen endpoint; the routing behavior below is automatic and requires no client switches.**

- **Same-Dialect Passthrough**: When the inbound dialect matches the target backend `kind` for the current attempt (`OpenAI↔OpenAI`, `Gemini↔Gemini`, `TypeSafe↔TypeSafe`, `OpenRouter↔OpenRouter`), Decora clones the validated request and replaces only the top-level `model` with the backend model ID, preserving original `questions` keys, option keys, and `criteria`. The upstream response still undergoes full probability sum, max-probability choice, `score.legend`, weighted expectation, and token conservation checks, then is returned natively (restoring only the public `model` alias, **without injecting `usage.decora`**, and preserving native fields such as `cost` and `input_tokens_by_modality`).
- **Cross-Dialect Translation & Fallback**: When routing across different provider protocols—or when a same-dialect primary fails with `429`/`5xx`/timeout and falls back to a heterogeneous backup—Decora automatically switches per attempt to translation via the Canonical IR.
- The OpenAI adapter preserves the native `input` and `questions` structure.
- Gemini carries shared input in top-level `input` (plain strings verbatim; role message arrays deterministically text-encoded) and authenticates with `x-goog-api-key`. Choice options use collision-proof internal ASCII keys (`decora_choice_0..N-1`) with compact JSON criteria strings to support Unicode and mixed boolean/string choices losslessly; score criteria map to an ordered array of non-empty strings.
- OpenRouter and TypeSafe (Jev) pass string `input` directly into `state`; role message arrays are encoded as bounded JSON text inside `state`, preserving all four valid roles, order, `phase`, empty content, Unicode, and nested JSON.
- String `instructions` pass through verbatim; text/JSON content arrays are encoded automatically so clients never flatten data or choose wire formats.
- There are no input-conversion flags or role/JSON/typed-choice capability toggles; real capabilities such as images, question types, question count, prompt caching, and safety identifiers are still filtered independently.

Internal message encoding serializes the following JSON object **into a single string** for text-only upstreams (clients never construct it). Question content uses the same envelope with `"kind": "question_content"`:

```json
{"decora_encoding":"openai_decisions_text_v1","kind":"messages","data":[{"role":"system","content":"system tag"},{"role":"user","content":[{"type":"input_json","data":{"ok":false,"nested":[1,"Unicode"]}}]}]}
```

This preserves role labels and structured data, **which is not equivalent to native OpenAI role-instruction hierarchy**. JSON data boundaries are also not prompt-injection isolation: a model can still follow adversarial instructions inside data. This is an upstream semantic difference, not a client-selectable format; deployments needing native role hierarchy should route to an upstream that enforces it natively.

Boolean and string choices can be mixed freely, including `true` / `"true"` and `false` / `"false"`. Whenever a choice group contains any boolean value, **all options in that group** use internal `decora_choice_0..N-1` keys with original typed values and descriptions preserved in `criteria`. Returned `choice` and probability values are restored to their exact original JSON types, and boolean `false` is never treated as missing/default. Even if a client supplies a literal string equal to an internal key (such as `"decora_choice_0"`), no collision occurs; string-only choice groups on TypeSafe/OpenRouter keep their original string keys.

### Images & Multimodal Inputs

The OpenAI adapter preserves native structure and supports images. Non-OpenAI backends still filter real `input_image` parts up front and **never encode image URLs into text and pretend the image was inspected**; compatible OpenAI candidates can be selected instead. Ordinary keys named `image_url`, `type`, or `role` inside `input_json.data` are treated purely as JSON data. Prompt-cache and `safety_identifier` parameters are not silently downgraded and are filtered when unsupported.

```bash
curl http://127.0.0.1:8787/v1/decisions \
  -H "Authorization: Bearer $DECORA_BEARER_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model":"decora-luna",
    "input":[{"role":"user","content":[
      {"type":"input_text","text":"This is the shared reference image."},
      {"type":"input_image","image_url":"https://example.com/reference.png"},
      {"type":"input_json","data":{"batch":"A"}}
    ]}],
    "questions":[{"type":"predicate","name":"same","instructions":[
      {"type":"input_text","text":"Is the candidate image a variant of the reference image?"},
      {"type":"input_image","image_url":"https://example.com/candidate.png"}
    ]}]
  }'
```

The image URLs above are placeholders; replace them with publicly accessible HTTP(S) URLs or `data:image/png;base64,...` data URLs. Decora never downloads remote images and does not accept `file_id`. Images across shared `input` and **all question `instructions`** are counted together: defaults are **32 images and 8 MiB total JSON body** (configurable up to 1,500 images and 64 MiB body, conservatively below upstream general limits). Model content limits (format, resolution, animation, patches, context) are validated by the upstream, and `4xx` responses are not retried on another model.

## Routing Configuration

See [`config.example.yaml`](config.example.yaml) for all fields. `models` maps public aliases to lists of backend IDs; each backend binds `kind`, `model`, `endpoint`, `api_key_env`, `capabilities`, `priority`, and `weight`.

Below is a **minimal standalone TypeSafe/Jev configuration** requiring no OpenAI key. Set `TYPESAFE_API_KEY` and `DECORA_BEARER_KEY`, and change `model` in the `curl` request above to `decora-jev` (the model name must be entitled on your account):

```yaml
listen: 127.0.0.1:8787
bearer_key_env: DECORA_BEARER_KEY
usage_policy: compatible
backends:
  jev-official:
    kind: typesafe
    model: jev-latest
    api_key_env: TYPESAFE_API_KEY
    priority: 0
    weight: 1
    capabilities:
      question_types: [predicate, choice, score]
      images: false
      max_questions: 400
models:
  decora-jev: [jev-official]
```

The following is the same OpenAI Decisions request shape carrying role messages, in-question JSON, and mixed typed choices. `decora-jev` is just an example public alias—operators can swap upstreams behind an alias without changing client code:

```bash
curl http://127.0.0.1:8787/v1/decisions \
  -H "Authorization: Bearer $DECORA_BEARER_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model":"decora-jev",
    "input":[{"role":"user","content":[{"type":"input_json","data":{"flag":false,"text":"true"}}]}],
    "questions":[{"type":"choice","instructions":[
      {"type":"input_text","text":"Choose the exact JSON type and value of flag."},
      {"type":"input_json","data":{"field":"flag"}}
    ],"choices":[{"value":true},{"value":"true"},{"value":false},{"value":"false"}]}]
  }'
```

Equivalent OpenRouter configuration uses `kind: openrouter`, an entitled model ID (official docs example: `typesafe/jev-1.13`), and `api_key_env: OPENROUTER_API_KEY`. Gemini uses `kind: gemini`, `gemini-decision-1`, and `api_key_env: GEMINI_API_KEY` (automatically sending `x-goog-api-key`). Default endpoints are `https://api.typesafe.ai/v1/systemone`, `https://openrouter.ai/api/alpha/decisions`, and `https://generativelanguage.googleapis.com/v1beta/decisions` (no need to set `endpoint` manually). Multiple backends can be attached to a single alias for cross-protocol fallback.

1. Candidates are filtered by question types, images, prompt caching, `safety_identifier`, question count, and server `usage_policy`; valid role messages, JSON parts, and typed choices are adapted automatically rather than filtered out.
2. Lower `priority` is tried first; candidates with equal priority are ordered by **weighted sampling without replacement** using `weight`.
3. Each request allows at most `max_attempts` **total upstream calls** (including the first attempt); once all candidates are tried, subsequent attempts wrap around from the start of that request's permutation. There is no runtime model discovery or background probing.
4. Only connection failures, send-stage timeouts, and HTTP `429/500/502/503/504` trigger fallback/retry, using bounded exponential backoff with jitter.
5. `429 Retry-After` supports both integer seconds and HTTP-date headers and waits before trying the next candidate. If the required wait would exceed the remaining request timeout budget, Decora returns `429` immediately with the remaining suggested wait seconds rather than shortening the upstream's requested delay.
6. Client errors, upstream auth errors, refusals, invalid probabilities, missing/invalid required usage counts, HTTP redirects, and response-body transfer errors after response headers arrive are **never retried**. On `/v1/decisions`, OpenAI per-question `refusal` is preserved with HTTP 200 alongside other answered questions. On non-OpenAI inbound endpoints (`/v1beta/decisions`, `/v0/decisions`, `/api/v1/decisions`), an upstream OpenAI per-question `refusal` cannot be represented in the target dialect, so Decora returns `502 upstream_refusal_unrepresentable` without fabricating probabilities or switching models. Gemini/OpenRouter/TypeSafe do not define per-question refusals; unknown answer types return a protocol error without fallback.

`attempt_timeout_ms` bounds one upstream send + response body read; `request_timeout_ms` bounds inbound body reading, all attempts, and retry waits (defaults: 30 s and 60 s). Response bodies have a separate size cap, and exceeding `max_in_flight` returns `503` immediately. Retrying after timeouts can cause duplicate upstream billing because Decisions has no verified idempotency-key contract here; set `max_attempts: 1` when duplicate calls must be strictly prevented.

## Security Defaults

- Loopback-only by default. Binding to a non-loopback address requires both `allow_public_bind: true` and `bearer_key_env`, and public deployments should terminate TLS, enforce firewalling, and rate-limit traffic in an external reverse proxy (Decora has no built-in TLS server).
- Removing `bearer_key_env` enables unauthenticated loopback-only mode; do not use this on untrusted multi-user machines.
- Clients can only reference configured public aliases and cannot supply arbitrary upstream URLs, backend model names, provider parameters, or forwarded headers.
- Default endpoints are official HTTPS URLs; operators may configure other trusted HTTPS endpoints. Userinfo, query strings, and fragments in endpoint URLs are rejected. Plain HTTP is allowed only for loopback **IP** addresses when `allow_http_loopback: true` is explicitly set for testing.
- Uses a shared `reqwest` connection pool with TLS certificate validation, **HTTP redirects disabled, and environment proxy auto-discovery disabled**, preventing credential leakage across domains. Outbound requests send only the backend credential, never the caller's inbound key.
- Logs emit only fixed messages, attempt counts, and fixed error codes—never inputs, questions, response bodies, secrets, upstream URLs, aliases, or YAML contents. The binary pins `decora=info` and ignores `RUST_LOG` to prevent accidental sensitive HTTP debug logs.
- All external errors use the fixed envelope `{"error":{"message":...,"type":...,"param":null,"code":...}}` and never echo upstream error bodies.

Error codes: `invalid_request`, `unsupported_content`, `unsupported_tool_history`, `image_limit_exceeded`, `model_not_found`, `unsupported_capability`, `unsupported_native_mapping`, `unsupported_native_images`, `unsupported_usage`, `invalid_api_key`, `rate_limit_exceeded`, `upstream_protocol_error`, `upstream_refusal_unrepresentable`, `upstream_client_error`, `upstream_timeout`, `gateway_busy`.

## Offline Verification & Reproducible Demo

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Verified result: **43 passing tests** (1 unit test + 42 offline integration tests), all four gates green, and `cargo audit` across 182 locked crates reports **0 vulnerabilities** (see [`HANDOFF.md`](HANDOFF.md)).

To run the reproducible end-to-end demo script against loopback mocks (zero API keys or cost required) or optionally against real OpenAI/Gemini endpoints, see [`demo/README.md`](demo/README.md):

```bash
bash demo/run.sh
```

The integration test suite binds loopback TCP mocks on ephemeral ports without reading real keys or calling paid APIs. It covers real inbound TCP requests, all four inbound dialects (`/v1/decisions`, `/v1beta/decisions`, `/v0/decisions`, `/api/v1/decisions`) across all four upstream adapters (OpenAI, Gemini, OpenRouter, TypeSafe), same-dialect passthrough and cross-dialect fallback, Gemini `inline_data` image routing to OpenAI vision backends, strict dialect schema enforcement, `502 upstream_refusal_unrepresentable`, `compatible` vs. `strict` usage policies, Gemini `total_input_tokens` and TypeSafe `billable_input` provenance tags, probability/expectation/usage math validation, four-role and in-question JSON round-tripping, Unicode/empty/escaping edge cases, mixed boolean/string/internal-key collision avoidance, image filtering, duplicate names, dual-header auth (`Bearer` and `x-goog-api-key` plus conflict rejection), weighted priority routing, redirect blocking, `429 Retry-After`, shared retry budgets after slow uploads, timeouts, concurrency limits, and inbound/outbound body size caps.

## Pre-Production Checklist

1. Configure native Decisions models entitled on your actual accounts and a high-entropy inbound key; restrict config/env file permissions to the service account.
2. Authorize a minimal live request to verify account entitlements, real responses, and billing.
3. For public deployments, place Decora behind a TLS reverse proxy with access control, process supervision, and rate limiting. This version does not include multi-tenancy, billing, persistence, or a database. Dependencies are pinned in `Cargo.lock`; review advisories before deploying.

### Key Boundaries

- **Usage is explicitly partially compatible in cross-dialect mode**: the default mode preserves verified counts and tags their provenance; TypeSafe totals are `billable_input + output` and Gemini totals are `total_input_tokens + output`, not exact OpenAI totals. Use `usage_policy: strict` when complete native usage schemas are required.
- **Capability compliance is not model calibration**: offline tests prove protocol translation and validation, not account access or statistical calibration on your business data.
- **Strict, conservative protocol subset**: tool-call history explicitly returns `unsupported_tool_history`; unknown fields, over-limit payloads, and invalid upstream responses are rejected rather than silently dropped. Responses always report the requested public `model` alias to avoid leaking internal routing names.

## Repository Layout

- `src/config.rs`: Configuration and credential resolution; `src/protocol.rs`: Canonical Decision IR and strict validation.
- `src/dialect.rs`: Strict parsing, passthrough, and response formatting for all four inbound dialects (OpenAI, Gemini, TypeSafe, OpenRouter).
- `src/adapter.rs`: Capability filtering and native upstream protocol adapters; `src/gateway.rs`: HTTP routing, dual-header auth, fallback, and retry budgets.
- `tests/offline.rs`: Offline integration test suite; `docs/PROTOCOL.md`: Official references, Hub-and-Spoke architecture, and capability matrix.
- `demo/`: Reproducible loopback mock and optional real-API demo scripts (`demo/run.sh`, `demo/mock.py`, `demo/README.md`).
- `.kiro/`: Kiro steering documents, feature specs (`requirements.md` / `design.md` / `tasks.md`), and agent hooks.
- `HANDOFF.md`: Implementation history, verification results, and handoff notes.

## Official References

- [OpenAI Decisions Guide](https://developers.openai.com/api/docs/guides/decisions)
- [OpenAI Decisions Create Reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- [Google Gemini Decisions API](https://aistudio.google.com/docs/decisions-api)
- [OpenRouter Alpha Decisions](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-questions-and-answers-request)
- [TypeSafe Official API](https://docs.typesafe.ai/api)
