# Tasks: OpenRouter and TypeSafe/Jev Native Paths and Transparent Usage Compatibility Policy (`native-upstreams-and-usage`)

- [x] 1. Add `BackendKind::Openrouter`, `BackendKind::Typesafe`, and the server-side `UsagePolicy` (`compatible` / `strict`) configuration in `src/config.rs`.
- [x] 2. Implement native request construction and response answer restoration for the three question types for OpenRouter and TypeSafe/Jev in `src/adapter.rs`, validating `legend` and probability distributions.
- [x] 3. Implement the `compatible` / `strict` usage validation logic and the `usage.decora` basis markers (TypeSafe `billable_input`; OpenRouter `provider_reported` and `provider_cost`) in `src/adapter.rs` and `src/protocol.rs`.
- [x] 4. Wire cross-protocol routing and fallback in `src/gateway.rs` (e.g., automatically fall back from OpenRouter 503 to TypeSafe).
- [x] 5. Add end-to-end round-trip tests for the three upstream types, cross-protocol fallback, and usage policy tests in `tests/offline.rs`, and update `README.md`, `docs/PROTOCOL.md`, and `config.example.yaml` accordingly.