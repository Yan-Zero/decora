# Decora Development Handoff

## Current Status: Same-Dialect Passthrough & Cross-Dialect Translation Complete

**Decora now supports per-attempt dynamic selection between Same-Dialect Native Passthrough and Cross-Dialect Canonical IR Translation on its symmetric Hub-and-Spoke multi-dialect architecture. All 43 tests pass (1 unit test + 42 offline integration tests), and `fmt` / `check` / `clippy` / `audit` are all green.**

**Explicit native-schema exception**: Under `usage_policy: compatible`, cross-dialect routes still allow omitting unknown target-dialect/OpenAI usage breakdowns and attach `usage.decora` provenance metadata. Same-dialect passthrough returns upstream native `usage` verbatim (without injecting `usage.decora`) and is permitted under `usage_policy: strict`.

## Milestone Summary

- Completed all 5 tasks in `.kiro/specs/dialect-passthrough/`, delivering `OpenAI↔OpenAI`, `Gemini↔Gemini`, `TypeSafe↔TypeSafe`, and `OpenRouter↔OpenRouter` same-dialect passthrough with automatic cross-dialect fallback.
- Added English external error messages and reproducible loopback-mock / optional live-API demo capture scripts under `demo/`.
- Added root MIT `LICENSE` file.

## Core Same-Dialect Passthrough Implementation

### 1. Isomorphic Matching & Per-Attempt Switching (`src/dialect.rs`, `src/gateway.rs`)
- `Dialect::matches_backend(kind)` defines the four isomorphic pairs: `Openai↔Openai`, `Gemini↔Gemini`, `Typesafe↔Typesafe`, and `Openrouter↔Openrouter`.
- `Dialect::passthrough_body` clones the validated inbound JSON request and replaces only the top-level `model` with `backend.model`, preserving the client's original `questions` keys, option keys, and `criteria` without rewriting them into `q0`/`q1` or `decora_choice_0`.
- `Gateway::execute` evaluates `dialect.matches_backend(backend.config.kind)` on every attempt; if a same-dialect primary fails with `429`/`5xx`/timeout and falls back to a heterogeneous backup, the next attempt automatically switches to Canonical IR translation (`native_body_for` / `format_response`).

### 2. Strict Verification & Zero-Pollution Passthrough (`src/dialect.rs`)
- `Dialect::passthrough_response` reuses `DecisionResponse::parse_with_usage` and native usage validation to verify finite `[0, 1]` probabilities, distribution sum and weighted `score` expectation tolerance `<= 0.0001`, max-probability `choice` selection, exact `score.legend == raw_request["questions"][key]["criteria"][i]`, and token sum conservation.
- Once verified, only top-level `model` is rewritten back to the public alias; native `answers` and `usage` are returned intact (**no `usage.decora` injection**, preserving OpenRouter `usage.cost` and Gemini `input_tokens_by_modality`).

### 3. Allowing Same-Dialect Candidates under `usage_policy: strict` (`src/adapter.rs`)
- `compatible_for_dialect` allows same-dialect native candidates under `usage_policy: strict` (`/v1beta/decisions -> gemini`, `/v0/decisions -> typesafe`, `/api/v1/decisions -> openrouter`), while continuing to block cross-dialect non-OpenAI candidates (`400 unsupported_usage`) and real `input_image` payloads on non-OpenAI native backends (`400 unsupported_native_images`) before any network call.

## Verification Results

Executed serially:

| Check | Result |
|---|---|
| `cargo fmt --check` | Passed, no formatting diffs |
| `cargo check --locked` | Passed |
| `cargo test --locked` | 1 unit test + 42 offline integration tests = 43 passed, 0 failed (< 1 s) |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed, zero warnings |
| `cargo audit --file Cargo.lock` | 182 locked dependencies, 0 vulnerabilities (1 allowed yanked advisory) |

Key integration test highlights:
- `multi_dialect_gemini_inbound_routes_to_gemini_and_falls_back_to_openai`: verifies `Gemini -> Gemini` passthrough (preserving native keys `"a_damaged"`, `"b_route"`, `"c_level"` and `"a_support"`, `"b_sales"`, with no `usage.decora`), followed by automatic fallback on Gemini `503` to OpenAI with cross-dialect translation (`usage.decora.provider == "openai"`).
- `same_dialect_passthrough_preserves_native_keys_and_works_under_strict_usage`: verifies `/v0/decisions -> typesafe`, `/api/v1/decisions -> openrouter` (preserving native `usage.cost` and Unicode keys without `usage.decora`), and `/v1beta/decisions -> gemini` under `usage_policy: strict`; also verifies that cross-dialect non-OpenAI routes still return `400 unsupported_usage` under `strict`, and that invalid probabilities, tampered `legend`, and invalid `usage` in passthrough mode are rejected as `502 upstream_protocol_error` without fallback.

## Delivered Files
- `src/adapter.rs`, `src/dialect.rs`, `src/gateway.rs`: same-dialect passthrough and per-attempt cross-dialect fallback.
- `tests/offline.rs`: full 43-test offline verification suite.
- `demo/`: reproducible loopback mock and optional real-API proof (`run.sh`, `mock.py`, `capture.sh`, `capture.py`, `table.jq`, `README.md`).
- `README.md`, `docs/PROTOCOL.md`, `HANDOFF.md`, `config.example.yaml`, `.kiro/`: synchronized English documentation, steering rules, specs, and hooks.

## Production Follow-Ups
1. Verify live account entitlements and billing for TypeSafe and OpenRouter (OpenAI `gpt-6-luna` and Gemini `gemini-decision-1` have been verified live via `DEMO_REAL=1 bash demo/capture.sh`).
2. For public deployments, run behind a TLS reverse proxy with firewalling, rate limiting, and process supervision.
