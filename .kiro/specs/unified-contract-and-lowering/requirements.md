# 需求文档：统一外部 Decisions 合同与内置角色/JSON/有类型选项转换（`unified-contract-and-lowering`）

## 背景与目标

协议转换是网关的内部职责，客户端不应感知上游是 OpenAI、OpenRouter 还是 TypeSafe/Jev，也不应配置或切换任何“输入转换策略”。本 Spec 的目标是固定唯一的外部 OpenAI Decisions 请求与核心答案合同，自动在网关内部完成角色消息、`input_json` 及布尔/字符串混合 `choice` 的无损编码与还原，并移除所有转换策略控制面。

---

## 需求列表

### 需求 1：固定单一外部请求合同与移除转换策略开关
- **验收标准 1.1**：所有公开 model alias 均只接受标准 OpenAI `POST /v1/decisions` 请求（`model / input / questions`）；客户端若提交上游专有的 `state`、`criteria`、命名 `questions` 对象或任何转换策略字段，一律返回 `400 invalid_request` 且不发起上游调用。
- **验收标准 1.2**：从服务端配置中彻底移除 `native_input_policy` 以及 `capabilities.messages / json_input / typed_choices` 开关；旧配置文件若残留上述字段将被 `deny_unknown_fields` 显式拒绝。

### 需求 2：角色消息与结构化内容的内部无损编码
- **验收标准 2.1**：OpenAI 原生后端保持原始 `input` 与 `questions` 结构不变。
- **验收标准 2.2**：OpenRouter/Jev 后端对字符串 `input` 和字符串 `instructions` 原样传递；对角色消息数组与 `input_text / input_json` 内容数组自动编码为单字符串 JSON 封包 `{"decora_encoding":"openai_decisions_text_v1","kind":"messages"|"question_content","data":...}`，保留全部四种合法角色、顺序、`phase`、空内容、Unicode 与嵌套 JSON。
- **验收标准 2.3**：在文档中明确声明该文本编码仅保留角色标签与数据边界，不等于上游原生执行 OpenAI 角色指令层级，也不是 prompt injection 隔离。

### 需求 3：布尔与字符串混合 `choice` 的无碰撞还原
- **验收标准 3.1**：当 `choice` 选项组包含布尔值（如同时包含 `true`、`"true"`、`false`、`"false"`）时，整组选项统一改用内部键 `decora_choice_0..N-1`，在 `criteria` 中保留原始 JSON 类型的 `value` 与可选 `description`。
- **验收标准 3.2**：响应解析时按内部索引键准确还原选中项 `choice` 与 `probabilities` 中的原始有类型 `value`；布尔 `false` 不被当作缺省值，且即使客户端提供名为 `"decora_choice_0"` 的字符串选项或空字符串选项也不会发生碰撞。

### 需求 4：真实多模态与能力边界保持独立筛选
- **验收标准 4.1**：真实 `input_image`（无论在共享 `input` 还是题目 `instructions` 中）在任何文本编码前独立检查，排除不支持原生图片的 OpenRouter/Jev 候选，绝不把图片 URL 编码进文本冒充已分析图片。
- **验收标准 4.2**：`input_json.data` 中出现的 `"image_url"`、`"type"`、`"role"` 键仅作为普通 JSON 数据处理，不触发图片拦截或消息角色伪造。
