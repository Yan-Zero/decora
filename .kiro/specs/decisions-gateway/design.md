# Design: Decora Core Decisions Gateway and Security Baseline (`decisions-gateway`)

## 1. Module Breakdown

- **`src/config.rs`**: Defines `Config`, `Backend`, `BackendKind`, `Capabilities`. Parses YAML with `serde_saphyr` and enables `#[serde(deny_unknown_fields)]`. `Config::validate` enforces non-loopback bind conditions, endpoint URL security constraints (HTTPS, or a loopback IP with `allow_http_loopback` explicitly enabled; usernames/passwords/query/fragment forbidden), and parameter ranges.
- **`src/protocol.rs`**: Defines `DecisionRequest`, `Question`, `ChoiceOption`, `ChoiceValue`, `ScoreLevel`, `Requirements`, and `DecisionResponse`. `DecisionRequest::parse` collects the `Requirements` the request needs (image count, caching, safety, JSON, messages); `DecisionResponse::parse` validates probability distributions, weighted expectations, and the OpenAI `usage` arithmetic relationships.
- **`src/gateway.rs`**: Holds a shared `reqwest::Client` (redirects and system proxies disabled) and a concurrency-limit `Semaphore` (`max_in_flight`). Provides the `/v1/decisions` and `/healthz` routes, constant-time Bearer authentication middleware, weighted-without-replacement ordering `weighted_order`, jittered exponential `backoff`, and `Retry-After` budget control.
- **`src/error.rs`**: Defines `ApiError`, mapping all internal and upstream errors to fixed public English messages and fixed error codes, and never echoing upstream response bodies.

## 2. Key Invariants

1. **Authenticate before reading large request bodies**: inbound Bearer validation and concurrency-slot acquisition complete before business execution; exceeding `max_body_bytes` or `max_response_bytes` terminates immediately.
2. **No retry for a transport failure after successful response headers**: once upstream returns 2xx response headers, a failure while streaming the body returns `502 upstream_body_error`, avoiding silent duplicate billing without an idempotency credential.
3. **Hide internal topology**: the outward response `model` is always rewritten to the public alias the client requested, never exposing backend model IDs.