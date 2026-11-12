# 需求文档：OpenRouter 与 TypeSafe/Jev 原生链路及透明 Usage 兼容策略（`native-upstreams-and-usage`）

## 背景与目标

除 OpenAI `/v1/decisions` 外，OpenRouter（`/api/alpha/decisions`）与 TypeSafe/Jev（`/v1/systemone`）也提供原生单次决策接口，但它们采用 `state` + 命名 `questions` 字典 + `noul / choice / score` + `criteria` 协议形状，且返回的 `usage` 不包含完整的 OpenAI 缓存读写与推理细分（TypeSafe 的 `input_tokens` 更是 `billable_input` 口径）。

本 Spec 目标是实现 OpenRouter 与 TypeSafe/Jev 的真实 HTTP 原生适配器，并引入服务端 `usage_policy: compatible | strict`，在不伪造任何未知 token 零值的前提下支持跨上游路由与回退。

---

## 需求列表

### 需求 1：OpenRouter 与 TypeSafe/Jev 原生协议映射
- **验收标准 1.1**：支持 `BackendKind::Openrouter`（默认端点 `https://openrouter.ai/api/alpha/decisions`）与 `BackendKind::Typesafe`（默认端点 `https://api.typesafe.ai/v1/systemone`）。
- **验收标准 1.2**：将 OpenAI 题目数组按序映射为内部稳定键 `q0..qN-1`（支持客户端省略或重复题目 `name`），并将 `predicate` 映射为 `noul`、`choice` 映射为 `criteria` 对象、`score` 映射为 `criteria` 数组。
- **验收标准 1.3**：将上游返回的 `answers[q0..qN-1]` 按原始题目顺序与原始 `name` 还原为 OpenAI 答案数组，并继续执行严格的概率和（`0.0001` 容差）、最大概率选中项及 `score` 加权期望校验。
- **验收标准 1.4**：对上游未定义的逐题 `refusal` 或未知答案类型返回 `502 upstream_protocol_error`，不伪造否定答案、不盲目回退。

### 需求 2：透明 Usage 兼容策略（`usage_policy`）
- **验收标准 2.1**：服务端 `config.yaml` 提供 `usage_policy: compatible`（默认）与 `usage_policy: strict`；该选项不属于客户端请求参数。
- **验收标准 2.2**：在 `compatible` 模式下，保留上游真实提供的 `input_tokens`、`output_tokens` 及已知明细，省略未知明细（绝不填 `0`），并添加 `usage.decora` 扩展字段标明 `provider`、`input_tokens_basis`、`total_tokens_basis`、`details_complete` 及可选 `provider_cost`。
- **验收标准 2.3**：TypeSafe 的 `input_tokens_basis` 明确标记为 `"billable_input"`；OpenRouter 标记为 `"provider_reported"`。
- **验收标准 2.4**：在 `strict` 模式下，发请求前直接排除无法提供完整精确 OpenAI usage 的上游候选；若无兼容候选则返回 `400 unsupported_usage`。
- **验收标准 2.5**：原生 OpenAI 后端在任何模式下均严格校验完整的 OpenAI `usage` 结构。
