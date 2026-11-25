# Design: OpenRouter and TypeSafe/Jev Native Paths and Transparent Usage Compatibility Policy (`native-upstreams-and-usage`)

## 1. Native request and answer mapping (`src/adapter.rs`)

- **Question key isolation**: Because OpenAI `questions` is an ordered array that allows anonymous or same-name questions, whereas OpenRouter/TypeSafe `questions` is a JSON object, the adapter uniformly uses `q0, q1, ..., q{N-1}` as upstream keys, and restores the original `q.name()` during the response restoration stage.
- **Question type mapping**:
  - `Predicate` ↔ `type: "noul"`, with the response `noul` field mapped back to `probability`.
  - `Choice` ↔ `type: "choice"`, with `criteria` a map from option to description, and the response `probabilities` mapped back to a `[{value, probability}]` array.
  - `Score` ↔ `type: "score"`, with `criteria` an ordered list of levels; after verifying that the response `legend` matches the request `criteria` item by item, map back to `[{value: i, label, probability}]`.

## 2. Usage policy and the `usage.decora` structure

- **`UsagePolicy::Strict`**: intercepts non-OpenAI backends at the `adapter::compatible` stage and returns `unsupported_usage`.
- **`UsagePolicy::Compatible`**:
  - `native_response` extracts `input_tokens` and `output_tokens` (non-negative `i32`), computes and checks `total_tokens = input + output`.
  - Extract the existing `input_tokens_details` (OpenRouter requires `cached_tokens`) and `output_tokens_details` when present; missing fields are omitted.
  - Write `usage.decora`:
    - TypeSafe: `{"provider": "typesafe", "input_tokens_basis": "billable_input", "total_tokens_basis": "billable_input_plus_output", "details_complete": ...}`
    - OpenRouter: `{"provider": "openrouter", "input_tokens_basis": "provider_reported", "total_tokens_basis": "provider_input_plus_output", "details_complete": ..., "provider_cost": ...}`
  - State honestly at the beginning of `README.md` and `docs/PROTOCOL.md` that `usage` in `compatible` mode is a known contract exception, not a strictly complete OpenAI usage schema.