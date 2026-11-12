# 任务清单：OpenRouter 与 TypeSafe/Jev 原生链路及透明 Usage 兼容策略（`native-upstreams-and-usage`）

- [x] 1. 在 `src/config.rs` 中添加 `BackendKind::Openrouter`、`BackendKind::Typesafe` 及服务端 `UsagePolicy`（`compatible` / `strict`）配置。
- [x] 2. 在 `src/adapter.rs` 中实现 OpenRouter 与 TypeSafe/Jev 的三题型原生请求构造与响应答案还原，校验 `legend` 与概率分布。
- [x] 3. 在 `src/adapter.rs` 与 `src/protocol.rs` 中实现 `compatible` / `strict` usage 校验逻辑与 `usage.decora` 口径标记（TypeSafe `billable_input`、OpenRouter `provider_reported` 与 `provider_cost`）。
- [x] 4. 在 `src/gateway.rs` 中接入跨协议路由与回退（如 OpenRouter 503 自动回退至 TypeSafe）。
- [x] 5. 在 `tests/offline.rs` 中补充三类上游端到端往返、跨协议回退与 usage 策略测试，并同步更新 `README.md`、`docs/PROTOCOL.md`、`config.example.yaml`。
