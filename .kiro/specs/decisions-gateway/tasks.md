# Task List: Decora Core Decisions Gateway and Security Baseline (`decisions-gateway`)

- [x] 1. Initialize the Rust 2024 project structure and implement the fixed outward error wrapper and error-code mapping in `src/error.rs`.
- [x] 2. Implement configuration parsing, environment variable name validation, the credential resolver `resolve_secret`, and security boundary validation in `src/config.rs`.
- [x] 3. Implement outward OpenAI Decisions request validation in `src/protocol.rs` (role messages, `input_json` depth/size, `input_image` counting, and data URL validation) plus response probability/expectation/usage validation.
- [x] 4. Implement the Axum routes, constant-time Bearer authentication, concurrency limiting, weighted-priority without-replacement ordering, bounded retries, and `429 Retry-After` handling in `src/gateway.rs`.
- [x] 5. Implement the startup entry point in `src/main.rs` (fixed `decora=info` log level and graceful shutdown) and write the offline loopback-mock integration tests in `tests/offline.rs` plus documentation.