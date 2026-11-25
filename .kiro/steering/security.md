# Security and Privacy Constraints

These are hard project constraints and must not be weakened when changing code; explain the risks to the user first if an adjustment is truly necessary.

- **Logging**: Record only fixed messages, attempt counts, and fixed error codes. Never log inputs, questions, response bodies, keys, upstream URLs, aliases, or YAML contents. `main.rs` fixes `decora=info` and does not accept `RUST_LOG`, preventing reqwest/hyper debug leakage.
- **Error body**: Use the uniform `{"error":{"message","type","param":null,"code"}}` shape; do not echo upstream error bodies.
- **Credentials**: Backend and inbound keys are supplied only through environment variables; parse all at startup, refuse startup if missing, blank, or containing newlines, and never output their contents. Compare inbound Bearer credentials in constant time using `subtle`. Send only backend credentials upstream; never forward the caller's Bearer token or any other header.
- **Outbound HTTP**: Share a reqwest connection pool; verify certificates; **disable redirect following and automatic environment proxy discovery**. Endpoints default to official HTTPS; URLs must not contain user info, query, or fragment. HTTP is allowed only for explicitly enabled loopback **IP addresses** (for tests). Clients cannot specify the upstream URL, backend model, or provider parameters.
- **Listening**: Loopback by default; public listening requires both `allow_public_bind: true` and `bearer_key_env`; no inbound TLS, so an external reverse proxy is required.
- **Resource limits**: Inbound/response body size limits, image-count limits, immediate 503 when `max_in_flight` is full, and attempt/request timeouts.
- **Replay risk**: Retrying after a timeout may cause duplicate upstream charges. Decisions has no verified idempotency token; set `max_attempts: 1` when duplicate execution must be prevented.
- **Testing**: Access only random-port loopback mocks; never read real keys or call paid APIs. Any real call requires explicit user authorization.
- **File access**: Do not read or echo `config.yaml` or `.env*` contents; refer only to variable names. Do not commit these files.
