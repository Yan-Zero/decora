# Project Structure

```
Cargo.toml / Cargo.lock
config.example.yaml     # Configuration example (no secrets); config.yaml is local and not committed
README.md               # User-facing external contract, multi-dialect endpoints, configuration, security defaults
HANDOFF.md              # Development handoff notes: decisions, verification results, open tasks
docs/PROTOCOL.md        # Official references, multi-frontend/multi-backend capability matrix, internal conversion and encoding
reference/              # Locally downloaded official OpenAPI (ignored by .gitignore, not committed)
src/
  main.rs               # Executable entry point: reads config.yaml (or a single path argument), fixes decora=info logging
  lib.rs                # Module declarations; exports Config / Gateway
  config.rs             # Configuration and credential parsing and validation
  protocol.rs           # Canonical Decision IR and OpenAI request/response validation
  dialect.rs            # Multi-frontend Decisions dialect encoding/decoding (OpenAI / Gemini / TypeSafe / OpenRouter inbound parsing and outbound response formatting)
  adapter.rs            # Outbound capability boundaries and native OpenAI / Gemini / OpenRouter / TypeSafe upstream protocol mapping
  gateway.rs            # Multi-endpoint HTTP routing, Bearer/x-goog-api-key authentication, candidate ordering, retries/fallback, timeouts
  error.rs              # Unified external error body and error codes
tests/offline.rs        # Offline integration tests (loopback mock upstream)
```

## Responsibility Boundaries

- Changes to the canonical intermediate representation (Canonical IR), mathematical probability validation, or OpenAI contract → `protocol.rs`.
- Changes to multi-frontend inbound request parsing or outbound response formatting (`/v1/decisions`, `/v1beta/decisions`, `/v0/decisions`, `/api/v1/decisions`) → `dialect.rs` (and synchronize README / PROTOCOL.md).
- Changes to outbound upstream mapping, capability filtering, usage normalization, or typed choice / score mapping → `adapter.rs`.
- Changes to routing, retries, timeouts, inbound authentication, or concurrency → `gateway.rs`.
- Changes to configuration fields → `config.rs` + `config.example.yaml` + README; when removing a field, remove it from the example too, or the example-config test will fail.
- Add error codes in `error.rs` and update the README list of common error codes.

## Documentation Synchronization

Behavior or contract changes must also update `README.md` and `docs/PROTOCOL.md`, and append conclusions and actual verification results to `HANDOFF.md` (include real command output; do not list checks that were not run). The descriptions of the usage exception must be consistent in all three locations.

## Commit Convention

Commit messages are concise and describe the intent of the change (example: `Refactor multi-dialect protocol architecture and add Gemini and TypeSafe inbound endpoints`). Commit or push only when explicitly requested by the user. Before committing, ensure staged content excludes `config.yaml`, `.env`, logs, `target/`, and `reference/`; prefer staging by filename.
