# Requirements: Decora Core Decisions Gateway and Security Baseline (`decisions-gateway`)

## Background and Goals

Build a standalone Rust single-tenant **OpenAI Decisions protocol gateway** (`POST /v1/decisions` and `GET /healthz`) that strictly validates native probabilities, confidences, weighted-score expectations, and token statistics in requests and responses, provides capability-filtered weighted-priority routing with bounded failover, and meets high-security and zero-sensitive-log-leakage requirements by default.

---

## Requirements List

### Requirement 1: OpenAI Decisions Request and Response Contract Validation
- **Acceptance Criterion 1.1**: Expose only `POST /v1/decisions` and `GET /healthz`; unknown routes return `404 not_found`, and unknown JSON fields are always rejected (`400 invalid_request`).
- **Acceptance Criterion 1.2**: Validate `model`, `input` (a string, or an array of messages with role `user / system / developer / assistant`), and `questions` (`1..=400` questions, supporting `predicate`, `choice`, `score`).
- **Acceptance Criterion 1.3**: `choice` supports `2..=255` unique options (distinguishing boolean `true` from the string `"true"`); `score` supports `2..=10` ordered levels and computes the weighted expectation from index positions `0..N-1`.
- **Acceptance Criterion 1.4**: Validate `input_json.data` (object or array of objects, depth `<= 32`, value count `<= 10,000`, compact bytes `<= 256 KiB`) and `input_image.image_url` (HTTP(S) or base64 data URL; `file_id` forbidden; the total image count across the shared input and all questions must be `<= max_images`).
- **Acceptance Criterion 1.5**: Strictly validate that upstream probabilities and `confidence` are finite and within `[0, 1]`; the absolute error tolerance for the `choice` probability sum and for the `score` probability sum/weighted expectation is `0.0001`; upstream probabilities are never renormalized or clamped.
- **Acceptance Criterion 1.6**: Preserve per-question `type: refusal` (HTTP 200) and never switch models to replay because of a single-question refusal.

### Requirement 2: Capability Filtering, Weighted-Priority Routing, and Bounded Retries
- **Acceptance Criterion 2.1**: Before any network request, filter compatible backend candidates by question type, question count, images, caching (`prompt_cache`), and `safety_identifier`.
- **Acceptance Criterion 2.2**: Order candidates by ascending `priority`, and within the same priority perform weighted sampling without replacement by `weight`.
- **Acceptance Criterion 2.3**: A request makes at most `max_attempts` total upstream calls; only connection failures, send-phase timeouts, and HTTP `429/500/502/503/504` permit failover or retry.
- **Acceptance Criterion 2.4**: Parse `429 Retry-After` (integer seconds or HTTP date); if the wait exceeds the remaining total request budget (`request_timeout_ms`), immediately return `429 rate_limit_exceeded` to the client with the suggested wait in seconds, without shortening the wait the upstream requires.

### Requirement 3: Secure Defaults and Offline Verification
- **Acceptance Criterion 3.1**: Listen on `127.0.0.1:8787` by default; a non-loopback bind requires both `allow_public_bind: true` and `bearer_key_env`.
- **Acceptance Criterion 3.2**: All secrets are resolved at startup only via environment variable names (plaintext secrets in YAML are forbidden); inbound Bearer credentials use `subtle` constant-time comparison; outbound requests disable redirect following and automatic environment proxy discovery.
- **Acceptance Criterion 3.3**: Logging is fixed at `decora=info` and records only fixed messages and error codes — never inputs, questions, response bodies, URLs, aliases, or secrets; all outward errors use the fixed structure `{"error":{"message","type","param":null,"code"}}`.
- **Acceptance Criterion 3.4**: Provide fully offline loopback-mock integration tests that read no real API keys and make no paid network calls.