# 设计文档：Google Gemini 原生 Decisions 后端支持（`gemini-backend`）

## 1. 架构概览

本次扩展保持 Decora 现有的四层职责划分不变：
1. **`src/config.rs`**：在 `BackendKind` 中新增 `Gemini` 变体，提供默认端点 `https://generativelanguage.googleapis.com/v1beta/decisions`。
2. **`src/adapter.rs`**：按 `BackendKind` 区分上游原生请求构造（`native_body_for`）与原生响应解析（`native_response`），复用相同的核心答案校验与能力边界检查。
3. **`src/gateway.rs`**：出站请求时根据 `BackendKind` 选择鉴权头（`Gemini` 使用 `x-goog-api-key`，其余后端使用 `Authorization: Bearer`）。
4. **`tests/offline.rs` 与文档**：新增 loopback mock 端到端回归测试，并同步更新 `README.md`、`docs/PROTOCOL.md`、`config.example.yaml`、`HANDOFF.md`。

---

## 2. 详细设计

### 2.1 `src/config.rs`：后端种类与默认端点

```rust
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Openai,
    Openrouter,
    Typesafe,
    Gemini,
}

impl BackendKind {
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1/decisions",
            Self::Openrouter => "https://openrouter.ai/api/alpha/decisions",
            Self::Typesafe => "https://api.typesafe.ai/v1/systemone",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/decisions",
        }
    }
}
```

### 2.2 `src/gateway.rs`：出站鉴权头分派

在 `Gateway::execute` 中构造 `reqwest::RequestBuilder` 时：
- 当 `backend.config.kind == BackendKind::Gemini` 时，调用 `.header("x-goog-api-key", &backend.key)`。
- 当 `backend.config.kind` 为 `Openai | Openrouter | Typesafe` 时，保持 `.bearer_auth(&backend.key)`。

### 2.3 `src/adapter.rs`：请求映射（`native_body_for`）

为保持向后兼容，可保留 `native_body(req, model)` 或新增 `native_body_for(req, model, kind: BackendKind)`：

1. **顶层输入字段**：
   - `Openrouter` / `Typesafe`：保持现有 `"state": textual(&req.input, "messages")`（现有测试与契约已验证）。
   - `Gemini`：使用 `"input": textual(&req.input, "messages")`。当 `req.input` 为字符串时直接传递该字符串；当 `req.input` 为角色消息数组时，序列化为 `{"decora_encoding":"openai_decisions_text_v1","kind":"messages","data":...}` 字符串。

2. **题目转换（`questions` 字典，`q0..qN-1`）**：
   - **`predicate` -> `noul`**：
     `{"type": "noul", "instructions": textual(q.instructions(), "question_content")}`。
   - **`choice`**：
     - `Openrouter` / `Typesafe`：保持现有逻辑（纯字符串组使用原字符串键与 `choice.description`；含布尔组使用 `decora_choice_0..N-1` 与 `{"value": ..., "description": ...}`）。
     - `Gemini`：由于 Gemini 限制选项键必须为 `1..=128` 字符的纯 ASCII（禁止中文、空串、首尾空白等）且 `criteria` 的值必须是字符串，Gemini 统一使用 `decora_choice_0..N-1` 作为选项键，并将每个选项的 `criteria` 值序列化为紧凑 JSON 字符串：
       - 无描述：`json!({"value": choice.value}).to_string()`
       - 有描述：`json!({"value": choice.value, "description": d}).to_string()`
       这样既满足 Gemini 对纯 ASCII 键和字符串描述的要求，又能在模型评测时完整保留原始选项值语义，并在返回时通过 `decora_choice_0..N-1` 索引 100% 无损还原任意 `ChoiceValue`（含中文、空串、布尔 `true`/`false` 及与内部键同名的字符串）。
   - **`score`**：
     - `Openrouter` / `Typesafe`：保持现有逻辑（无 description 时为 `l.label` 字符串，有 description 时为 `{"label": l.label, "description": d}`）。
     - `Gemini`：Gemini 要求 `score.criteria` 为 `2..=255` 个**非空字符串**（`string[]`）。为防止空 `label` 违反 Gemini 非空约束并同时保留 `label` 与 `description`：
       - 当 `description` 为 `None` 且 `!l.label.is_empty()` 时，直接使用 `l.label` 字符串；
       - 当 `l.label.is_empty()` 或 `description.is_some()` 时，使用紧凑 JSON 字符串 `json!({"label": l.label, "description": l.description}).to_string()`（始终非空）。
       响应中的 `legend` 仍按 `body["questions"][&key]["criteria"][i]` 逐项严格比对，然后还原为 OpenAI 合同要求的 `{"value": i, "label": l.label, "probability": ...}`。

### 2.4 `src/adapter.rs`：响应与 Usage 解析（`native_response`）

1. **顶层 `model` 字段校验**：
   - `Openrouter` / `Typesafe`：继续要求上游响应包含非空字符串 `value["model"]`。
   - `Gemini`：官方响应顶层仅有 `answers` 和 `usage`（不含 `model`）；若 `value.get("model")` 存在，则要求其为非空字符串；若不存在，则直接采用 `&req.model` 传入 `DecisionResponse::parse_with_usage`（`DecisionResponse::parse_with_usage` 最终都会将对外 `model` 设为 `req.model`）。

2. **Gemini `usage` 解析（`usage_policy: compatible`）**：
   - 读取必需字段 `total_input_tokens`、`total_output_tokens`、`total_tokens`，均须为非负 `i32` 整数。
   - 校验 `total_input_tokens.checked_add(total_output_tokens) == Some(total_tokens)`，不满足则返回 `502 upstream_protocol_error`。
   - 若存在 `input_tokens_by_modality`：
     - 必须为数组；每个元素必须为对象，包含非空字符串 `modality` 与非负 `i32` 整数 `tokens`；
     - 校验通过后写入 `usage.decora.input_tokens_by_modality`。
   - 生成顶层兼容 `usage`：
     ```json
     {
       "input_tokens": total_input_tokens,
       "output_tokens": total_output_tokens,
       "total_tokens": total_tokens,
       "decora": {
         "provider": "gemini",
         "input_tokens_basis": "total_input_tokens",
         "total_tokens_basis": "total_input_plus_output",
         "details_complete": false
       }
     }
     ```
     （若存在 `input_tokens_by_modality` 则一并放入 `usage.decora`）。
   - 不伪造 `input_tokens_details` 或 `output_tokens_details`。

---

## 3. 故障与边界行为

- **重试与回退**：与 OpenRouter/TypeSafe 完全一致。支持在同一公开 alias 下配置 `[gemini-primary, jev-official, openai-primary]` 等跨协议回退链。
- **安全与隐私**：`GEMINI_API_KEY` 只通过 `api_key_env` 环境变量名引用，启动时校验非空可打印 ASCII；日志不打印任何请求内容、响应体、URL 或密钥。
