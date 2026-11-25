# Decora — Live WASM Protocol Converter Demo

This static site runs the **real Decora Rust core** (`src/protocol.rs`, `src/dialect.rs`, `src/adapter.rs`) compiled to WebAssembly (`wasm32-unknown-unknown`) directly in the browser—zero backend server, zero API keys, and zero fabricated probabilities.

## Rebuild the WASM Binary

```bash
rustup target add wasm32-unknown-unknown
cargo build --manifest-path wasm/Cargo.toml --target wasm32-unknown-unknown --release
cp wasm/target/wasm32-unknown-unknown/release/decora_wasm.wasm playground/decora_wasm.wasm
```

## Run Locally

```bash
python -m http.server 8080 --directory playground
# Open http://127.0.0.1:8080
```

## Deploy to Cloudflare Pages (Static, Free)

1. Connect the GitHub repository (`Yan-Zero/decora`) in **Cloudflare Dashboard → Workers & Pages → Create → Pages**.
2. Set:
   - **Framework preset**: `None`
   - **Build command**: *(leave blank — prebuilt static assets + `decora_wasm.wasm` are already in `playground/`)*
   - **Build output directory**: `playground`
3. Deploy — Cloudflare Pages serves `playground/` globally with automatic HTTPS and `application/wasm` MIME type (configured via `playground/_headers`).
