# Requirements: OpenRouter and TypeSafe/Jev Native Paths and Transparent Usage Compatibility Policy (`native-upstreams-and-usage`)

## Background and Goals

Besides OpenAI `/v1/decisions`, OpenRouter (`/api/alpha/decisions`) and TypeSafe/Jev (`/v1/systemone`) also expose native single-decision endpoints, but they use a `state` + named `questions` dictionary + `noul / choice / score` + `criteria` protocol shape, and the returned `usage` does not include the full OpenAI cached read/write and reasoning breakdown (TypeSafe's `input_tokens` is even on a `billable_input` basis).

The goal of this spec is to implement real HTTP native adapters for OpenRouter and TypeSafe/Jev, and to introduce a server-side `usage_policy: compatible | strict`, supporting cross-upstream routing and fallback without fabricating zero values for any unknown tokens.

---

## Requirements

### Requirement 1: Native protocol mapping for OpenRouter and TypeSafe/Jev
- **Acceptance Criteria 1.1**: Support `BackendKind::Openrouter` (default endpoint `https://openrouter.ai/api/alpha/decisions`) and `BackendKind::Typesafe` (default endpoint `https://api.typesafe.ai/v1/systemone`).
- **Acceptance Criteria 1.2**: Map the OpenAI question array in order to internal stable keys `q0..qN-1` (supporting clients that omit or duplicate question `name`s), and map `predicate` to `noul`, `choice` to a `criteria` object, and `score` to a `criteria` array.
- **Acceptance Criteria 1.3**: Restore upstream `answers[q0..qN-1]` into the OpenAI answer array in the original question order and with the original `name`s, and continue to enforce the strict probability sum (`0.0001` tolerance), max-probability selection, and `score`-weighted expectation validation.
- **Acceptance Criteria 1.4**: For per-question `refusal` or unknown answer types not defined by the upstream, return `502 upstream_protocol_error`; do not fabricate negative answers and do not blindly fall back.

### Requirement 2: Transparent usage compatibility policy (`usage_policy`)
- **Acceptance Criteria 2.1**: The server-side `config.yaml` provides `usage_policy: compatible` (default) and `usage_policy: strict`; this option is not a client request parameter.
- **Acceptance Criteria 2.2**: In `compatible` mode, retain the `input_tokens`, `output_tokens`, and known details actually provided by the upstream, omit unknown details (never fill `0`), and add a `usage.decora` extension field indicating `provider`, `input_tokens_basis`, `total_tokens_basis`, `details_complete`, and optional `provider_cost`.
- **Acceptance Criteria 2.3**: TypeSafe's `input_tokens_basis` is explicitly marked as `"billable_input"`; OpenRouter's is marked as `"provider_reported"`.
- **Acceptance Criteria 2.4**: In `strict` mode, exclude upfront, before sending the request, any upstream candidate that cannot provide complete and exact OpenAI usage; if no compatible candidate exists, return `400 unsupported_usage`.
- **Acceptance Criteria 2.5**: The native OpenAI backend strictly validates the complete OpenAI `usage` structure in every mode.