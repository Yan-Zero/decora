# 设计文档：统一外部 Decisions 合同与内置角色/JSON/有类型选项转换（`unified-contract-and-lowering`）

## 1. 移除转换控制面（`src/config.rs` & `src/gateway.rs`）

- 删除 `NativeInputPolicy` 枚举及 `Config::native_input_policy`。
- 删除 `Capabilities` 中的 `messages`、`json_input`、`typed_choices` 布尔开关。
- `Gateway::execute` 对非 OpenAI 后端直接调用统一的内部转换函数，无需传入策略参数。

## 2. 内部文本封包与选项键重映射（`src/adapter.rs`）

- **真实能力前置校验（`validate_native_request`）**：
  - 遍历 `req.input` 消息 part 与 `req.questions[i].instructions` part，仅当外层内容块 `part["type"] == "input_image"` 时判定为真实图片并返回 `unsupported_native_images`（忽略 `input_json.data` 内部同名键）。
  - 检查 `prompt_cache_breakpoint`、`safety_identifier`、`prompt_cache_key`、`prompt_cache_options`，存在则返回 `unsupported_native_mapping`。
- **结构化输入编码（`textual`）**：
  - 若 `value` 为 JSON 字符串，原样克隆；
  - 若 `value` 为数组，序列化为 `json!({"decora_encoding":"openai_decisions_text_v1","kind":kind,"data":value}).to_string()`。
- **混合类型选项键（`choice_key`）**：
  - 若选项组含任意 `ChoiceValue::Bool(_)`，第 `i` 项键固定为 `format!("decora_choice_{i}")`，`criteria` 写入 `{"value": choice.value, "description"?: ...}`；
  - 若全为字符串，直接使用原字符串作为键。
  - 在 `native_answers` 中通过 `(0..choices.len()).find(|i| choice_key(choices, *i) == selected)` 定位选中项并还原原始 `ChoiceValue`。
