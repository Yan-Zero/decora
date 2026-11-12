# 设计文档：多方言入站端点与对称中心辐射式协议架构（`multi-dialect-endpoints`）

## 1. 中心辐射式（Hub-and-Spoke）三阶段流水线

```text
  [OpenAI Client]       POST /v1/decisions      ──┐
  [Gemini Client]       POST /v1beta/decisions  ──┼─► Dialect::parse_request
  [TypeSafe Client]     POST /v0/decisions      ──┤         │
  [OpenRouter Client]   POST /api/v1/decisions  ──┘         ▼
                                              Canonical IR (DecisionRequest, Requirements)
                                                            │
                                                            ▼
                                              Gateway::execute (路由 + Adapter + 数学不变量校验)
                                                            │
                                                            ▼
                                              Canonical IR (DecisionResponse)
                                                            │
                                                            ▼
                                              Dialect::format_response ──► 返回对应方言响应
```

## 2. 模块设计与职责重构

### 2.1 `src/dialect.rs`（新增）
- 定义 `#[derive(Clone, Copy, PartialEq, Eq)] pub enum Dialect { Openai, Gemini, Typesafe, Openrouter }`。
- **`Dialect::parse_request(self, value: Value, max_images: usize) -> Result<(DecisionRequest, Requirements), ApiError>`**：
  - `Dialect::Openai`：直接委托 `DecisionRequest::parse(value, max_images)`。
  - `Dialect::Gemini`：
    - 校验顶层仅含 `["model", "input", "questions"]` 且均存在。
    - 若 `input` 为字符串，原样保留；若 `input` 为非空数组（`<= 16384`），将每项 `{"text": s}` 转换为 `{"type": "input_text", "text": s}`，将 `{"inline_data": {"mime_type": m, "data": d}}`（校验 `m` 不含 `;`）转换为 `{"type": "input_image", "image_url": format!("data:{m};base64,{d}")}`，包裹为单条 `role: "user"` 消息交由 `DecisionRequest::parse` 统一执行 base64、MIME 与图片上限校验。
    - 校验 `questions` 为非空对象（`1..=400`），每个键 `k` 满足 `1..=64` 位 ASCII `[A-Za-z0-9_-]`；
      - `type == "noul"` -> `Question::Predicate { name: Some(k), instructions }`（`instructions` 必须为字符串）；
      - `type == "choice"` -> 校验 `criteria` 为 `2..=255` 项对象，键为 `1..=128` 位无首尾空白纯 ASCII 字符串，值为描述字符串 `s`（保持 `description: Some(s)` 以便响应还原）；
      - `type == "score"` -> 校验 `criteria` 为 `2..=10` 项非空字符串数组，映射为 `ScoreLevel { label, description: None }`。
  - `Dialect::Typesafe | Dialect::Openrouter`：
    - 校验顶层仅含 `["model", "state", "questions"]` 且均存在，`state` 必须为字符串。
    - 校验 `questions` 为非空对象（`1..=400`），每个键 `k` 为非空字符串：
      - `type == "noul"` -> `Question::Predicate { name: Some(k), instructions }`（`instructions` 必须为字符串）；
      - `type == "choice"` -> 校验 `criteria` 为 `2..=255` 项对象，键为字符串，值为 `null`（`description: None`）或字符串 `s`（`description: Some(s)`）；
      - `type == "score"` -> 校验 `criteria` 为 `2..=10` 项数组，元素为字符串 `label`（`description: None`）或 `{"label": s, "description"?: d}`（`description` 不能为 `null`）。
- **`Dialect::format_response(self, req: &DecisionRequest, resp: DecisionResponse) -> Result<Value, ApiError>`**：
  - `Dialect::Openai`：直接序列化 `resp`。
  - `Dialect::Gemini | Dialect::Typesafe | Dialect::Openrouter`：
    - 遍历 `resp.answers.iter().zip(&req.questions)`：
      - 若遇到 `Answer::Refusal`，立即返回 `ApiError::new(StatusCode::BAD_GATEWAY, "upstream_refusal_unrepresentable", "当前入口协议不支持逐题 refusal 表达；未伪造概率。")`。
      - `Answer::Predicate { probability, .. }` -> `{"type": "noul", "noul": probability}`。
      - `Answer::Choice { choice, confidence, probabilities, .. }` -> 将 `choice` 与 `probabilities` 还原为以选项字符串为键的对象。
      - `Answer::Score { score, confidence, probabilities, .. }` -> 将 `probabilities` 转换为 `"0"..` 索引对象，并构造 `legend`（Gemini 下为字符串 `l.label`；TypeSafe/OpenRouter 下若 `l.description` 为 `Some(d)` 则为 `{"label": l.label, "description": d}`，否则为 `l.label`）。
    - 构造方言对应的 `usage`：
      - 当 `resp.usage.decora` 为 `None`（即上游为 OpenAI 原生后端）且当前入站方言非 OpenAI 时，自动补齐 `decora` 元数据：
        `{"provider": "openai", "input_tokens_basis": "openai_input_tokens", "total_tokens_basis": "openai_input_plus_output", "details_complete": true}`。
      - `Dialect::Gemini` 输出 `total_input_tokens`、`total_output_tokens`、`total_tokens`、可选 `input_tokens_by_modality` 及 `decora`。
      - `Dialect::Typesafe | Dialect::Openrouter` 输出 `input_tokens`、`output_tokens`、`total_tokens`、可选 `input_tokens_details / output_tokens_details` 及 `decora`。

### 2.2 `src/adapter.rs` 精简与去重
- 提取 `choice_key_for(choices: &[ChoiceOption], index: usize, kind: BackendKind) -> String` 与 `score_criterion(level: &ScoreLevel, kind: BackendKind) -> Value` 两个纯函数。
- `native_answers_for` 直接使用 `choice_key_for` 和 `score_criterion` 校验 `legend`，消除原先在响应解析阶段重新构造整份 `native_body_for` 的冗余开销。
- 提取统一的 `non_negative_i32(source: &Map<String, Value>, key: &str) -> Result<i32, ApiError>` 辅助函数，消除 Gemini 与 OpenRouter/TypeSafe 之间的重复闭包。

### 2.3 `src/gateway.rs` 多端点挂载与双轨鉴权
- 在 `Gateway::router` 中挂载四个入站路由：
  - `POST /v1/decisions` -> `Dialect::Openai`
  - `POST /v1beta/decisions` -> `Dialect::Gemini`
  - `POST /v0/decisions` -> `Dialect::Typesafe`
  - `POST /api/v1/decisions` -> `Dialect::Openrouter`
- 在 `guard` 中提取辅助函数校验 `Authorization: Bearer <key>` 或 `x-goog-api-key: <key>`（互斥、单值、定长比较 `ct_eq`）。
