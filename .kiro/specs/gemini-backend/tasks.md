# 任务清单：Google Gemini 原生 Decisions 后端支持（`gemini-backend`）

- [x] 1. 在 `src/config.rs` 中新增 `BackendKind::Gemini` 及默认官方端点 `https://generativelanguage.googleapis.com/v1beta/decisions`。
- [x] 2. 在 `src/adapter.rs` 中实现 `BackendKind::Gemini` 的原生请求转换（顶层 `input`、ASCII 安全的 `choice` 键与字符串 `criteria`、非空字符串数组 `score.criteria`）及能力边界筛选（拒绝真实图片、缓存与 safety 参数，以及 `strict` usage 模式）。
- [x] 3. 在 `src/adapter.rs` 中实现 `BackendKind::Gemini` 的原生响应与 `usage` 解析（兼容无顶层 `model` 响应，校验 `total_input_tokens / total_output_tokens / total_tokens` 及可选 `input_tokens_by_modality`，生成 `usage.decora` 标记，不伪造未知明细）。
- [x] 4. 在 `src/gateway.rs` 中为 `BackendKind::Gemini` 分派 `x-goog-api-key` 请求头并调用对应 adapter 转换函数。
- [x] 5. 在 `tests/offline.rs` 中新增 Gemini 原生后端的端到端 loopback mock 测试（含 `x-goog-api-key` 鉴权、角色消息/JSON/中文与布尔混合 choice/score 往返、跨协议一致性回归、`usage.decora` 与 `input_tokens_by_modality` 校验、`strict` 模式拦截、图片拦截与跨后端故障回退）。
- [x] 6. 同步更新 `config.example.yaml`、`README.md`、`docs/PROTOCOL.md`、`.kiro/steering/` 相关说明及 `HANDOFF.md`，并按顺序运行 `cargo fmt --check`、`cargo check --locked`、`cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings` 全部通过。
