# 设计文档：OpenRouter 与 TypeSafe/Jev 原生链路及透明 Usage 兼容策略（`native-upstreams-and-usage`）

## 1. 原生请求与答案转换（`src/adapter.rs`）

- **题目键隔离**：由于 OpenAI `questions` 是有序数组且允许匿名或同名题目，而 OpenRouter/TypeSafe `questions` 是 JSON 对象，适配器统一使用 `q0, q1, ..., q{N-1}` 作为上游键，在响应还原阶段填回原始 `q.name()`。
- **题型映射**：
  - `Predicate` ↔ `type: "noul"`，响应 `noul` 字段映射回 `probability`。
  - `Choice` ↔ `type: "choice"`，`criteria` 为选项到描述的映射，响应 `probabilities` 映射回 `[{value, probability}]` 数组。
  - `Score` ↔ `type: "score"`，`criteria` 为有序等级列表，校验响应 `legend` 与请求的 `criteria` 逐项一致后映射回 `[{value: i, label, probability}]`。

## 2. Usage 策略与 `usage.decora` 结构

- **`UsagePolicy::Strict`**：在 `adapter::compatible` 阶段拦截非 OpenAI 后端，返回 `unsupported_usage`。
- **`UsagePolicy::Compatible`**：
  - `native_response` 提取 `input_tokens` 与 `output_tokens`（非负 `i32`），计算并核对 `total_tokens = input + output`。
  - 按需提取已存在的 `input_tokens_details`（OpenRouter 要求包含 `cached_tokens`）与 `output_tokens_details`，缺失字段直接省略。
  - 写入 `usage.decora`：
    - TypeSafe：`{"provider": "typesafe", "input_tokens_basis": "billable_input", "total_tokens_basis": "billable_input_plus_output", "details_complete": ...}`
    - OpenRouter：`{"provider": "openrouter", "input_tokens_basis": "provider_reported", "total_tokens_basis": "provider_input_plus_output", "details_complete": ..., "provider_cost": ...}`
  - 在 `README.md` 与 `docs/PROTOCOL.md` 开头诚实声明：`compatible` 模式下的 `usage` 是已知合同例外，并非严格完整的 OpenAI usage schema。
