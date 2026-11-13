# Decora 产品概述

Decora 是 Rust 实现的独立**多方言 Decisions 协议网关**。所有公开 model alias 同时支持四类官方形状的入站端点（另有 `GET /healthz`）：

- `POST /v1/decisions`：OpenAI Decisions 方言（`model / input / questions[]`，题型 `predicate / choice / score`）
- `POST /v1beta/decisions`：Google Gemini Decisions 方言（`model / input / questions{}`，题型 `noul / choice / score`）
- `POST /v0/decisions`：TypeSafe (Jev) Decisions 方言（`model / state / questions{}`，题型 `noul / choice / score`）
- `POST /api/v1/decisions`：OpenRouter Decisions 方言（`model / state / questions{}`，题型 `noul / choice / score`）

网关采用 **“多前端方言（Dialect） ↔ 内部标准中间表示（Canonical Decision IR） ↔ 多后端适配器（Adapter）+ 同方言原生直通（Same-Dialect Passthrough）”** 架构，使任意 Decisions 客户端都能零改造接入：当选中的后端与入站方言相同时直接透传（仅替换 `model`），当回退或路由至异构后端时通过中间表示自动转换。

## 核心原则（修改前必须理解）

1. **各端点外部合同严格固定**：每个入站端点只接受其对应官方方言的请求形状，不混用其他方言专有字段（例如向 `/v1/decisions` 发送 `state/criteria` 或向 `/v0/decisions` 发送 `questions` 数组一律返回 `400 invalid_request`），也不提供任何客户端“转换策略”开关。
2. **同方言原生直通（Same-Dialect Passthrough）与异构自动转换**：
   - 当入站端点方言与选中的出站后端 `kind` 一致时（`/v1/decisions` ↔ `openai`、`/v1beta/decisions` ↔ `gemini`、`/v0/decisions` ↔ `typesafe`、`/api/v1/decisions` ↔ `openrouter`），网关在完成 Schema 与数学不变量校验后**直接透传原始请求体**（仅将 `model` 替换为后端模型 ID，不重命名 `q0/q1`、不包裹 `decora_choice_*`），并将验证通过的**原生响应原样返回**（不注入 `usage.decora`，保留原生 `cost` / `input_tokens_by_modality` 等字段，且同方言直通不受 `usage_policy: strict` 拦截）。
   - 当入站方言与选中的出站后端不一致时（跨方言路由或故障回退），由 `dialect` 与 `adapter` 通过内部标准中间表示自动完成无损转换。
3. **不伪造数据**：未知 usage 明细省略而不是填 0；不重归一化、裁剪或重算上游概率；不合成缺失的 confidence；未知答案类型报协议错误；若 OpenAI 上游返回逐题 `refusal` 而当前入站方言（Gemini / TypeSafe / OpenRouter）不支持逐题拒答表达，明确返回 `502 upstream_refusal_unrepresentable`，绝不把拒答伪造成 `0.0` 或其他概率。
4. **已知合同例外（必须诚实标注）**：默认 `usage_policy: compatible` 在跨方言转换时可省略目标方言未知的 usage 明细并增加 `usage.decora` 扩展，**不是严格完整的原生 usage schema**。不得在文档或代码注释中宣称“跨方言响应完全一致”。`strict` 是服务端选项，不是客户端参数。
5. **公开 `model` 返回请求 alias**，不暴露内部后端 ID、URL 或凭据。入站鉴权同时支持 `Authorization: Bearer <token>` 与 `x-goog-api-key: <token>`（单次请求只允许提供一种有效凭据，且绝不向上游转发）。
6. **能力筛选仍然真实**：图片、题型、题数、缓存、safety 参数、usage 要求会在发起网络调用前筛选候选；图片不会被编码成文本冒充已分析。

## 范围边界

- 无数据库、UI、多租户、计费、持久化；无入站 TLS（需外部反向代理）。
- 工具调用历史明确 `unsupported_tool_history`；不实现 `box` 题型；不支持 `file_id` 图片。
- 离线验证只用 loopback mock，没有花费过真实 API 额度。不要声称真实账户可用性或概率已校准。

## 语言约定

README、docs、注释、提交信息、错误说明均使用**中文**；代码标识符使用英文。回复用户时也使用中文。
