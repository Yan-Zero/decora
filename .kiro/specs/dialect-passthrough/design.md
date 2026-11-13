# 设计文档：同方言原生直通（`dialect-passthrough`）

## 1. 方言与后端匹配关系（`src/dialect.rs`）

在 `Dialect` 上新增方法：
```rust
impl Dialect {
    pub fn matches_backend(self, kind: BackendKind) -> bool {
        matches!(
            (self, kind),
            (Self::Openai, BackendKind::Openai)
                | (Self::Gemini, BackendKind::Gemini)
                | (Self::Typesafe, BackendKind::Typesafe)
                | (Self::Openrouter, BackendKind::Openrouter)
        )
    }
}
```

## 2. 候选兼容性过滤（`src/adapter.rs`）

- 新增 `compatible_for_dialect(dialect: Dialect, backend: &Backend, req: &DecisionRequest, need: &Requirements, usage_policy: UsagePolicy) -> Result<(), ApiError>`（同时保留原 `compatible` 函数签名默认以 `Dialect::Openai` 调用，确保向后兼容）：
  - 基础能力检查（`max_questions`、`question_types`、`images`、`prompt_cache`、`safety_identifier`）对所有后端一致生效；
  - 若 `backend.kind != BackendKind::Openai`：
    - 仍调用 `validate_native_request(req)`（确保哪怕是 `Gemini -> Gemini` 直通，携带了 `inline_data` 图片也会因原生 Decisions 适配器边界返回 `unsupported_native_images`）；
    - 当 `usage_policy == UsagePolicy::Strict` 时，仅当 `!dialect.matches_backend(backend.kind)` 时返回 `Err(ApiError::unsupported("unsupported_usage"))`；若 `dialect.matches_backend(backend.kind)` 为真，允许该同方言候选通过！

## 3. 直通请求构造与直通响应校验（`src/dialect.rs`）

- **`Dialect::passthrough_body(self, raw_request: &Value, req: &DecisionRequest, backend_model: &str) -> Value`**：
  - `Self::Openai`：调用 `req.openai_body(backend_model)`；
  - `Self::Gemini | Self::Typesafe | Self::Openrouter`：克隆已通过 `parse_request` 严格校验的 `raw_request`，仅将 `body["model"] = json!(backend_model)`，完整保留原始 `questions` 键名、选项键名与 `criteria`。
- **`Dialect::passthrough_response(self, raw_request: &Value, req: &DecisionRequest, upstream_body: Value, usage_policy: UsagePolicy) -> Result<Value, ApiError>`**：
  - `Self::Openai`：调用 `DecisionResponse::parse_with_usage(upstream_body, req, usage_policy)` 并序列化为 `Value`；
  - `Self::Gemini | Self::Typesafe | Self::Openrouter`：
    1. 校验顶层 `model`（Gemini 可选非空字符串，TypeSafe/OpenRouter 必需非空字符串）、`answers`（对象，键集合与 `req.questions` 的 `name` 一一对应）、`usage`（对象）。
    2. 将 `upstream_body["answers"]` 中按原始题目键 `q.name()` 存放的答案重映射为按 `q0..qN-1` 索引的对象，并复用 `DecisionResponse::parse_with_usage` 对所有题目的概率范围、分布和、`choice` 最大概率项、`score` 加权期望及 `legend == raw_request["questions"][name]["criteria"][i]` 进行严格数学校验。
    3. 校验原生 `usage` 守恒（Gemini 校验 `total_input_tokens + total_output_tokens == total_tokens` 与可选 `input_tokens_by_modality`；TypeSafe/OpenRouter 校验 `input_tokens + output_tokens == total_tokens`、可选 `input_tokens_details / output_tokens_details` 及可选非负有限 `cost`）。
    4. 构造输出 JSON：`{"model": req.model, "answers": upstream_body["answers"], "usage": validated_native_usage}`（**不注入 `usage.decora`**）。

## 4. 网关执行流水线（`src/gateway.rs`）

- `Gateway::execute` 接收 `(dialect: Dialect, raw_request: Value, request: DecisionRequest, need: Requirements, deadline: Deadline) -> Result<Value, ApiError>`：
  - 路由阶段调用 `compatible_for_dialect(dialect, ...)`；
  - 在每次 `attempt` 循环中：
    - 若 `dialect.matches_backend(backend.config.kind)`：
      - 发送 `dialect.passthrough_body(&raw_request, &request, &backend.config.model)`；
      - 收到 2xx 响应后调用 `dialect.passthrough_response(&raw_request, &request, value, self.inner.config.usage_policy)` 直接返回；
    - 否则（跨方言路由或故障回退至异构后端）：
      - 发送 `BackendKind::Openai => request.openai_body(...)` 或 `native_body_for(&request, ...)`；
      - 收到 2xx 响应后经 `DecisionResponse::parse` / `native_response` 转换为 Canonical `DecisionResponse`，再调用 `dialect.format_response(&request, resp)` 返回。
