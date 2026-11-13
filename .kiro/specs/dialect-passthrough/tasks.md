# 任务清单：同方言原生直通（`dialect-passthrough`）

- [x] 1. 在 `src/adapter.rs` 中新增 `compatible_for_dialect`，支持在 `usage_policy: strict` 下放行同方言原生后端，同时保持 `compatible` 向后兼容。
- [x] 2. 在 `src/dialect.rs` 中实现 `Dialect::matches_backend`、`Dialect::passthrough_body` 与 `Dialect::passthrough_response`（保留原始题目键、选项键、原生 `usage` 结构，不注入 `usage.decora`，并执行同等严格的数学概率、`legend` 与 token 守恒校验）。
- [x] 3. 在 `src/gateway.rs` 中更新 `Gateway::execute`，按每次尝试（per-attempt）的目标后端动态选择“同方言原生直通”或“跨方言 Canonical IR 转换”。
- [x] 4. 在 `tests/offline.rs` 中更新并新增测试：验证四类方言同方言直通（不改写 `q0/decora_choice_0`、不注入 `usage.decora`、保留原生 `cost` / `input_tokens_by_modality`）、`usage_policy: strict` 下同方言直通放行与跨方言拦截、以及同方言主后端 503 自动切换为跨方言转换回退。
- [x] 5. 更新 `README.md`、`docs/PROTOCOL.md`、`HANDOFF.md` 与 `.kiro/specs/dialect-passthrough/tasks.md`，运行 `fmt / check / test / clippy / audit` 全量通过并提交推送。
