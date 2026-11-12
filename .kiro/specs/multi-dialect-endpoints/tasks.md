# 任务清单：多方言入站端点与对称中心辐射式协议架构（`multi-dialect-endpoints`）

- [x] 1. 在 `src/protocol.rs` 中扩展 `UsageProvider::Openai`、`InputTokenBasis::OpenaiInputTokens`、`TotalTokenBasis::OpenaiInputPlusOutput` 以支持非 OpenAI 入站端点调用 OpenAI 后端时的透明来源标记。
- [x] 2. 在 `src/adapter.rs` 中提取 `choice_key_for`、`score_criterion` 与 `non_negative_i32` 辅助函数，消除 `native_answers_for` 对 `native_body_for` 的冗余调用及重复的 usage 解析逻辑。
- [x] 3. 新增 `src/dialect.rs` 并在 `src/lib.rs` 中导出，实现 `Dialect`（`Openai`、`Gemini`、`Typesafe`、`Openrouter`）的严格入站解析 `parse_request`（含 Gemini `inline_data` 多模态归一化）与出站响应格式化 `format_response`（含 `upstream_refusal_unrepresentable` 保护）。
- [x] 4. 在 `src/gateway.rs` 中挂载 `/v1/decisions`、`/v1beta/decisions`、`/v0/decisions`、`/api/v1/decisions` 四个端点，并在 `guard` 中支持 `Authorization: Bearer` 与 `x-goog-api-key` 互斥定长鉴权。
- [x] 5. 在 `tests/offline.rs` 中新增多方言入站与跨方言路由回归测试（Gemini/TypeSafe/OpenRouter 入站跨四家后端路由回退、Gemini 入站 `inline_data` 路由至 OpenAI 视觉后端、方言 Schema 越界拦截、`x-goog-api-key` 入站鉴权与双头冲突拒绝、非 OpenAI 入站遇到上游 `refusal` 返回 `502 upstream_refusal_unrepresentable`）。
- [x] 6. 同步更新 `README.md`、`docs/PROTOCOL.md`、`HANDOFF.md` 与 `.kiro/specs/multi-dialect-endpoints/tasks.md`，运行 `fmt / check / test / clippy / audit` 全量通过并提交推送。
