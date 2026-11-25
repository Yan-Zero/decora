# Tech Stack and Common Commands

- Language: Rust, edition 2024; local verification toolchain `rustc/cargo 1.95.0`.
- Main dependencies: axum 0.8, reqwest 0.12 (rustls, **no default features**), serde / serde_json, serde-saphyr (YAML), tokio, tracing, subtle (constant-time inbound key comparison), rand, httpdate, base64. Dev: tower.
- Dependency versions are locked by `Cargo.lock`; use `--locked` for all commands. Explain the reason for any new dependency (the project deliberately keeps dependencies minimal) and remind the user to run `cargo audit`.
- Release profile: `lto = "thin"`, `strip = true`.
- Windows + PowerShell environment: separate commands with `;`, not `&&` (Windows PowerShell 5.1 does not support it).

## Commands

```powershell
cargo run --locked --release -- config.yaml   # Start; listens on 127.0.0.1:8787 by default
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo audit --file Cargo.lock                 # Requires cargo-audit to be installed separately
```

## Verification Checklist Before Completing a Code Change

Run in order and ensure all pass: `fmt --check` → `check --locked` → `test --locked` → `clippy --locked --all-targets -- -D warnings`. Baseline: 34 tests (1 unit test + 33 integration tests). Fix the root cause of any test failure; do not skip or delete tests to make them “pass.”

## Configuration and Credentials

- Configuration is YAML (see `config.example.yaml`); `deny_unknown_fields` makes unknown or removed fields fail explicitly.
- Credentials are referenced indirectly by environment variable name only (`api_key_env`, `bearer_key_env`); **plaintext secrets in YAML are unsupported**, and `.env` is not loaded automatically.
- `.gitignore` ignores `config.yaml`, `config.*.local.yaml`, `.env*`, `*.log`, `/logs/`, `/target/`, and `/reference/`.
