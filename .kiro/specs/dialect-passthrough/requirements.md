# Requirements: Same-Dialect Native Passthrough (`dialect-passthrough`)

## Background and Goals

After introducing multi-dialect inbound endpoints (`/v1/decisions`, `/v1beta/decisions`, `/v0/decisions`, `/api/v1/decisions`), every request is first parsed into the internal Canonical IR and then uniformly re-encoded on the outbound side by the `adapter` (for example, rewriting question names to `q0/q1`, rewriting option keys to `decora_choice_0..N-1` under Gemini, and injecting `usage.decora`).

However, when the dialect of the inbound endpoint the client uses and the `kind` of the backend hit by routing **are already the same protocol** (for example `/v1/decisions` -> `openai`, `/v1beta/decisions` -> `gemini`, `/v0/decisions` -> `typesafe`, `/api/v1/decisions` -> `openrouter`), internal key renaming and `usage.decora` injection are an unnecessary round trip.

The goal of this spec is to implement **Same-Dialect Passthrough** while fully preserving the gateway's inbound schema validation, capability-boundary interception, response mathematical probability-conservation validation, and cross-dialect automatic failover.

---

## Requirements List

### Requirement 1: Per-Attempt Dynamic Triggering of Same-Dialect Passthrough
- **Acceptance Criterion 1.1**: Define the isomorphism relationship between inbound dialects and backend kinds:
  - `Dialect::Openai` ↔ `BackendKind::Openai`
  - `Dialect::Gemini` ↔ `BackendKind::Gemini`
  - `Dialect::Typesafe` ↔ `BackendKind::Typesafe`
  - `Dialect::Openrouter` ↔ `BackendKind::Openrouter`
- **Acceptance Criterion 1.2**: When the backend hit by a given attempt matches the inbound `Dialect`, build the outbound request directly from the original JSON the client validated, replacing only the top-level `model` with the administrator-configured `backend.model`:
  - Preserve the caller's original `questions` key names (do not rewrite to `q0, q1`);
  - Preserve the caller's original `choice.criteria` key names and values (do not rewrite to `decora_choice_0` in `Gemini -> Gemini` passthrough);
  - Preserve the caller's original `score.criteria` representation.
- **Acceptance Criterion 1.3**: When the same-dialect primary backend returns 429/5xx or a connection timeout and fails over to a heterogeneous backup (or a heterogeneous primary fails over to a same-dialect backup), the gateway switches dynamically per attempt between "passthrough mode" and "cross-dialect conversion mode" according to the target backend.

### Requirement 2: Mathematical Invariant Validation and Zero-Pollution Pass-Through of Passthrough Responses
- **Acceptance Criterion 2.1**: Same-dialect passthrough is never "blind forwarding". Responses returned by upstream still undergo Decora's strict mathematical and protocol validation:
  - The question key set and question types must strictly match the request;
  - Probabilities for `noul / predicate`, `choice`, and `score` must be finite and within `[0, 1]`; the distribution sum and the `score` weighted expectation tolerance must be `<= 0.0001`; the selected `choice` must be the highest-probability option; and `score.legend` must exactly match the request `criteria`;
  - Each `usage` count field must be a non-negative `i32` and satisfy the input+output conservation.
- **Acceptance Criterion 2.2**: After validation passes, the same-dialect passthrough response keeps the upstream native structure (rewriting only the top-level `model` back to the public `model` alias the client requested):
  - **Do not inject the `usage.decora` extension field**;
  - In `Openrouter -> Openrouter` passthrough, preserve the upstream `usage.cost`, `input_tokens_details`, and `output_tokens_details` verbatim;
  - In `Gemini -> Gemini` passthrough, preserve the upstream `usage.total_input_tokens / total_output_tokens / total_tokens` and the optional `input_tokens_by_modality` verbatim.

### Requirement 3: Same-Dialect Passthrough Support under `usage_policy: strict`
- **Acceptance Criterion 3.1**: When the server is configured with `usage_policy: strict`, same-dialect candidate backends (for example `/v1beta/decisions` routed to `kind: gemini`, or `/v0/decisions` routed to `kind: typesafe`) return exactly the complete native usage that the caller's dialect expects and are therefore **not excluded by `unsupported_usage` filtering**.
- **Acceptance Criterion 3.2**: Under `usage_policy: strict`, cross-dialect non-OpenAI conversion candidates are still excluded before the request is sent (if a route contains only heterogeneous incomplete-usage candidates, it still returns `400 unsupported_usage`).