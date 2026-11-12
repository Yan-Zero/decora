# 需求文档：Google Gemini 原生 Decisions 后端支持（`gemini-backend`）

## 背景与目标

Google 发布了原生 **Gemini Decisions API**（模型 `gemini-decision-1`，端点 `POST https://generativelanguage.googleapis.com/v1beta/decisions`），通过单次前向传播（single forward pass）直接计算 `noul`（二元真假概率）、`choice`（多选概率分布）和 `score`（有序等级加权评分），不解码输出文本 token（`total_output_tokens` 恒为 `0`）。

本 Spec 的目标是在不改变 Decora 固定对外合同（`POST /v1/decisions`、OpenAI 形状的 `model / input / questions` 请求与 `predicate / choice / score / refusal` 核心答案）的前提下，新增 `kind: gemini` 原生后端支持，并严格遵守 `.kiro/steering/protocol-contract.md` 与 `.kiro/steering/security.md` 中的**不伪造数据**与**真实能力筛选**原则。

---

## 官方协议核对结论（基于 `https://aistudio.google.com/docs/decisions-api`）

1. **入口与鉴权**：
   - 固定官方入口：`POST https://generativelanguage.googleapis.com/v1beta/decisions`
   - 鉴权方式：HTTP 请求头 `x-goog-api-key: $GEMINI_API_KEY`（不使用 `Authorization: Bearer`）。
2. **请求顶层结构**：
   - 必需字段：`model`（如 `"gemini-decision-1"`）、`input`（字符串或内容块，**注意字段名为 `input` 而非 OpenRouter 的 `state`**）、`questions`（`1..=1000` 个题目的键值映射，键需满足 `1..=64` 位 ASCII `[A-Za-z0-9_-]+`）。
3. **题型与 `criteria` 约束差异**：
   - `noul`：对应 OpenAI `predicate`，包含 `type: "noul"` 与文本 `instructions`。
   - `choice`：`criteria` 为选项键到描述字符串的映射。**关键约束**：Gemini 要求选项键为 `1..=128` 字符的纯 ASCII（禁止首尾空格、非 ASCII 字符如中文、空串及控制字符），且每个描述值必须为**字符串**（无描述时为空字符串 `""`，不支持 `null` 或嵌套对象）。
   - `score`：`criteria` 必须为 **`2..=255` 个非空字符串的有序数组（`string[]`）**，不支持 `{label, description}` 对象。
4. **图片、缓存、安全标识与拒答（Refusal）边界**：
   - **图片**：Gemini 的题目 `instructions` **仅支持纯文本，不支持题目内图片**；且共享 `input` 中的图片块强制要求提供真实文件格式的 `mime_type`。由于 OpenAI `input_image.image_url` 为远程 HTTP(S) URL 时不携带 `mime_type`，而 Decora 安全策略禁止网关下载远程图片或猜测/伪造未知 MIME 类型，且不支持题目内图片降级，因此 `kind: gemini` 对真实 `input_image` 统一按固有能力边界在请求前筛选并返回 `unsupported_native_images`。
   - **缓存与安全标识**：Gemini Decisions API 官方明确不支持 context caching（`prompt_cache`），也不支持 OpenAI 的 `safety_identifier`。与 OpenRouter/TypeSafe 一致，提前筛选并返回 `unsupported_capability` 或 `unsupported_native_mapping`。
   - **Refusal（逐题拒答）**：Gemini 官方响应无逐题 `refusal` 答案类型（安全拦截直接返回整批 HTTP `400 Bad Request`）。遇到上游 400 按现有规则映射为对外 `400 upstream_client_error` 且不回退；遇到未定义答案类型返回 `502 upstream_protocol_error`。
5. **响应顶层与 Usage 口径**：
   - Gemini 响应顶层仅包含 `answers` 与 `usage`，**不包含顶层 `model` 字段**（Decora 对外响应的 `model` 始终填写客户端请求的公开 alias）。
   - Gemini `usage` 字段为：
     - `total_input_tokens`（非负整数，计费输入 token）
     - `total_output_tokens`（非负整数，官方规定恒为 `0`）
     - `total_tokens`（非负整数，等于 `total_input_tokens + total_output_tokens`）
     - `input_tokens_by_modality`（可选数组：`[{"modality": string, "tokens": integer}]`）

---

## 需求列表

### 需求 1：配置与后端类型扩展（`BackendKind::Gemini`）
- **验收标准 1.1**：`BackendKind` 新增 `gemini`（YAML 中为 `kind: gemini`），默认官方端点为 `https://generativelanguage.googleapis.com/v1beta/decisions`。
- **验收标准 1.2**：配置校验与现有后端一致：验证 `api_key_env` 环境变量名、HTTPS（或显式允许的 loopback IP HTTP）端点规则、题型与 `max_questions`（`1..=400`）范围。
- **验收标准 1.3**：网关向 `kind: gemini` 发送请求时，使用 `x-goog-api-key` 请求头传递该后端解析出的密钥，不发送 `Authorization: Bearer`，且绝不转发客户端入站凭据。

### 需求 2：固定外部合同与内部无损转换
- **验收标准 2.1**：客户端对绑定了 `kind: gemini` 的公开 alias 仍发送完全相同的 OpenAI `POST /v1/decisions` 请求（支持字符串或角色消息数组 `input`、题内 `input_text / input_json`、中文/空串/布尔混合 `choice`、带可选描述的 `score`）。
- **验收标准 2.2**：Gemini 请求体顶层使用 `"input"` 字段承载共享输入（字符串原样传递；角色消息数组使用现有的 `openai_decisions_text_v1` JSON 文本编码封装为字符串）。
- **验收标准 2.3**：题目键固定使用 ASCII 安全的 `q0..qN-1`。
- **验收标准 2.4**：由于 Gemini `choice.criteria` 仅允许非空纯 ASCII 键与字符串描述值，且 OpenAI `choice` 允许中文（如 `"售后"`）、空字符串、首尾空格、布尔值（`true` / `"true"`），Gemini 后端对 `choice` 统一使用内部 ASCII 键 `decora_choice_0..N-1`，将原始有类型 `value` 与可选 `description` 编码为字符串描述，并在解析响应时按索引映射精确还原原始 `ChoiceValue`（包括 `false`、`"false"`、中文、空串及与内部键同名的字符串）。
- **验收标准 2.5**：由于 Gemini `score.criteria` 要求非空字符串数组 `string[]`，且 OpenAI `score.levels` 允许空 label 或带 `description` 的等级，Gemini 后端将每个等级序列化为非空字符串 criteria，并在校验响应 `legend` 一致性后，按位置 `0..N-1` 精确还原原始 `label` 与概率分布。

### 需求 3：能力筛选与安全边界
- **验收标准 3.1**：包含真实 `input_image`（共享 `input` 或题目 `instructions`）的请求在编码前排除 `kind: gemini` 候选；若无其他兼容候选则返回 `400 unsupported_native_images`（即使配置了 `capabilities.images: true` 也不扩大 adapter 固有能力）。
- **验收标准 3.2**：`input_json.data` 内部的 `"image_url"`、`"type"`、`"role"` 键仍视为普通 JSON 数据，不触发图片拦截。
- **验收标准 3.3**：携带 `prompt_cache_*` 或 `safety_identifier` 的请求在发往 Gemini 前被排除。

### 需求 4：Usage 兼容与严格模式遵循“不伪造数据”原则
- **验收标准 4.1**：在 `usage_policy: strict` 下，`kind: gemini` 在发请求前被排除；若无兼容候选则返回 `400 unsupported_usage`。
- **验收标准 4.2**：在 `usage_policy: compatible` 下，严格校验 Gemini `usage` 中的 `total_input_tokens`、`total_output_tokens`、`total_tokens` 为非负 `i32` 整数且 `total_tokens == total_input_tokens + total_output_tokens`；映射为顶层 `input_tokens`、`output_tokens`、`total_tokens`，省略未知的 `input_tokens_details` 与 `output_tokens_details`（不伪造 0）。
- **验收标准 4.3**：在 `usage.decora` 中标记：
  - `"provider": "gemini"`
  - `"input_tokens_basis": "total_input_tokens"`
  - `"total_tokens_basis": "total_input_plus_output"`
  - `"details_complete": false`
  - 若上游返回合法的 `input_tokens_by_modality`（每项含非空字符串 `modality` 与非负 `i32` `tokens`），原样保留在 `usage.decora.input_tokens_by_modality` 中；若格式非法则返回 `502 upstream_protocol_error`。
- **验收标准 4.4**：Gemini 响应无需包含顶层 `model` 字段；若包含则必须为非空字符串。最终对外响应的 `model` 始终为客户端请求的公开 alias。
