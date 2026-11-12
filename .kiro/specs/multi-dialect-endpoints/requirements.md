# 需求文档：多方言入站端点与对称中心辐射式协议架构（`multi-dialect-endpoints`）

## 背景与目标

在此前版本中，Decora 已支持将 OpenAI Decisions 请求路由至 OpenAI、Google Gemini、OpenRouter 与 TypeSafe (Jev) 四类原生上游，但入站入口仅提供 OpenAI 形状的 `POST /v1/decisions`，且 `src/protocol.rs` 与 `src/adapter.rs` 将“前端入站格式”与“内部中间表示”耦合在一起。

本 Spec 的目标是：
1. **优化代码分层结构**：引入 `src/dialect.rs` 并精简 `src/adapter.rs`，构建 **“多前端方言（Dialect） ↔ 内部标准中间表示（Canonical Decision IR） ↔ 多后端适配器（Adapter）”** 的对称中心辐射式（Hub-and-Spoke）架构，消除 $M \times N$ 两两转换与重复校验。
2. **原生支持四类官方形状的入站端点**：
   - `POST /v1/decisions`：OpenAI Decisions 方言
   - `POST /v1beta/decisions`：Google Gemini Decisions 方言
   - `POST /v0/decisions`：TypeSafe (Jev) Decisions 方言
   - `POST /api/v1/decisions`：OpenRouter Decisions 方言
3. **支持双轨入站鉴权**：同时支持 `Authorization: Bearer <token>` 与 `x-goog-api-key: <token>`，让不同生态的 SDK/客户端无需改造即可接入 Decora，并跨任意兼容上游路由与故障回退。

---

## 需求列表

### 需求 1：对称中心辐射式（Hub-and-Spoke）代码结构重构
- **验收标准 1.1**：新增 `src/dialect.rs` 模块，定义 `Dialect` 枚举（`Openai`、`Gemini`、`Typesafe`、`Openrouter`），封装入站请求解析 `Dialect::parse_request` 与出站响应格式化 `Dialect::format_response`。
- **验收标准 1.2**：重构并消除 `src/adapter.rs` 中的重复代码（如提取共享的 `score_criterion`、`choice_key_for` 与非负 `i32` token 解析函数），使 `native_answers_for` 无需重复调用完整的 `native_body_for`。
- **验收标准 1.3**：保持 `decora::protocol`、`decora::adapter`、`decora::gateway`、`decora::config` 的现有公开符号签名向后兼容，确保已有全部 38 项离线测试无需妥协即可通过。

### 需求 2：多方言入站端点与严格 Schema 边界
- **验收标准 2.1**：每个入站端点严格绑定其官方方言 Schema，绝不在同一路径上猜测或混用其他方言字段：
  - 向 `/v1/decisions` 发送 `state`、`criteria` 或对象 `questions` 返回 `400 invalid_request`；
  - 向 `/v1beta/decisions` 发送 `state`、数组 `questions` 或 `predicate` 题型返回 `400 invalid_request`；
  - 向 `/v0/decisions` 或 `/api/v1/decisions` 发送 `input`、数组 `questions` 或 `predicate` 题型返回 `400 invalid_request`。
- **验收标准 2.2**：Gemini 入站端点（`POST /v1beta/decisions`）支持：
  - 顶层 `model`、`input`（纯文本，或由 `{"text": ...}` 与 `{"inline_data": {"mime_type": ..., "data": ...}}` 组成的多模态内容数组）、`questions`（`1..=400` 项对象，键为 `1..=64` 位 ASCII `[A-Za-z0-9_-]`）；
  - `noul`、`choice`（`2..=255` 项对象，键为 `1..=128` 位无首尾空格的纯 ASCII 字符串，值为描述字符串）、`score`（`2..=10` 项非空字符串数组 `criteria`）；
  - 响应返回 `model`、以原题目键索引的 `answers` 对象（含 `noul`、`choice + confidence + probabilities`、`score + confidence + probabilities + legend`）及 Gemini 形状的 `usage`（`total_input_tokens / total_output_tokens / total_tokens`，可选 `input_tokens_by_modality` 与 `decora`）。
- **验收标准 2.3**：TypeSafe（`POST /v0/decisions`）与 OpenRouter（`POST /api/v1/decisions`）入站端点支持：
  - 顶层 `model`、`state`（纯文本）、`questions`（`1..=400` 项对象，键为非空字符串）；
  - `noul`、`choice`（`2..=255` 项对象，键为任意非空字符串含中文，值为描述字符串或 `null`）、`score`（`2..=10` 项数组，元素为非空字符串 `label` 或 `{"label": ..., "description"?: ...}`）；
  - 响应返回 `model`、以原题目键索引的 `answers` 对象（`score.legend` 精确还原调用方请求的 `criteria` 项）及 `usage`（`input_tokens / output_tokens / total_tokens`，可选 `input_tokens_details / output_tokens_details` 与 `decora`）。

### 需求 3：跨方言路由、诚实拒答边界与透明 Usage
- **验收标准 3.1**：任意入站方言端点均可路由至任意兼容的后端（`openai`、`gemini`、`openrouter`、`typesafe`），并在主候选返回 429/5xx/连接超时时自动回退至下一优先级候选（例如：Gemini 入站请求因 Gemini 上游 503 自动回退至 OpenAI 或 TypeSafe 后端，并最终以 Gemini 响应格式返回）。
- **验收标准 3.2**：当非 OpenAI 入站端点（`/v1beta/decisions`、`/v0/decisions`、`/api/v1/decisions`）路由至 OpenAI 后端且 OpenAI 后端返回逐题 `type: "refusal"` 时，由于目标方言无逐题拒答结构，网关必须返回 `502 upstream_refusal_unrepresentable`，绝不将拒答伪造为 `0.0` 或其他概率，也不触发跨模型回退。
- **验收标准 3.3**：当非 OpenAI 入站端点在 `usage_policy: compatible` 下路由至 OpenAI 后端时，`usage.decora` 透明标记 `"provider": "openai"`、`"input_tokens_basis": "openai_input_tokens"`、`"total_tokens_basis": "openai_input_plus_output"`、`"details_complete": true`。

### 需求 4：双轨入站鉴权（`Authorization: Bearer` 与 `x-goog-api-key`）
- **验收标准 4.1**：当配置 `bearer_key_env` 时，四个 Decisions 入站端点均接受单一定长比较（`ConstantTimeEq`）的 `Authorization: Bearer <key>` 或 `x-goog-api-key: <key>`。
- **验收标准 4.2**：若单次请求同时携带 `Authorization` 与 `x-goog-api-key`、重复携带同名鉴权头、或凭据不匹配，在读取请求体前立即返回 `401 invalid_api_key`（附 `WWW-Authenticate: Bearer`）。客户端入站凭据绝不转发给任何上游。
