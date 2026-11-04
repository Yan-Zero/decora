# 协议依据与兼容边界

核对日期：**2026-11-04**。实现以官方 schema 为准，不把名称相似的第三方网关、博客或聊天 structured output 当作原生决策接口。

## 1. 官方来源

| 提供方 | 文档 | 机器可读 schema | 原生 POST 入口 |
|---|---|---|---|
| OpenAI | [指南](https://developers.openai.com/api/docs/guides/decisions)、[创建接口](https://developers.openai.com/api/reference/resources/decisions/methods/create) | [官方 openai-openapi](https://raw.githubusercontent.com/openai/openai-openapi/master/openapi.yaml) | `https://api.openai.com/v1/decisions` |
| OpenRouter | [Alpha Decisions API](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-questions-and-answers-request) | [官方 OpenAPI](https://openrouter.ai/openapi.json) | `https://openrouter.ai/api/alpha/decisions` |
| TypeSafe / Jev | [官方 API](https://docs.typesafe.ai/api) | [官方 OpenAPI](https://api.typesafe.ai/openapi.json) | `https://api.typesafe.ai/v1/systemone` |

其他依据：[OpenAI 图片规则](https://developers.openai.com/api/docs/guides/images-vision)、[缓存定义](https://developers.openai.com/api/docs/guides/prompt-caching)。所有入口采用服务端配置的 Bearer key；不转发调用者凭据。

下载的原始资料在本机 `reference/` 下（不进 git，避免把巨大的官方全量 schema 纳入小项目）。本次 SHA256 为

```text
OpenAI openapi.yaml
2213c5372faf662dd32dc91b2b2acdcf455df609c6fe9ab3a3413674f8b46449
OpenRouter openapi.json
07b8b1ac91ba9e522540417420cd74004db5246d5341e5c86d68443e4438af82
TypeSafe openapi.json
3ca93b6e6b9aa8d7820531b78c66ed7a9798b11e6e163ad96b140fb3d75fbcf4
```

这些在线入口可能更新；扩展 adapter 边界时需要重新核对，而不是照旧假定。已再次核对 OpenRouter 的 operation-level server 为 `https://openrouter.ai`，覆盖全局 `/api/v1` server，因此 Alpha Decisions 的正确入口是 `/api/alpha/decisions`，不重复拼接 `/api/v1`。

## 2. OpenAI 公开合同

### 请求

必需顶层字段是 `model`、`input` 和 `questions`。Decora 的 `model` 必须是配置中的公开 alias；OpenAI 官方当前列出的原生模型是 `gpt-6-luna`，本项目没有通过付费调用验证账户权限。

- `input`：文字字符串或有序消息数组。支持 user、developer、system、assistant。
- user 内容：字符串或 `input_text / input_image / input_json` 数组。
- developer/system 内容：字符串或 `input_text` 数组。
- assistant 内容：字符串或 `output_text / refusal` 内容数组，允许混合；`phase` 可为 `commentary / final_answer / null`。
- `input_json.data` 必须为对象或对象数组；嵌套值可为任意 JSON。每块最多 **32 层容器、10,000 个值、256 KiB 紧凑 UTF-8 JSON**；实现将根容器也计入层数/值数。
- `questions`：**1–400** 项，按序返回答案。每题 `instructions` 必需，`name` 可省略；名字允许重复、空字符串，但不能显式为 null。
- 题目 instructions：字符串或非空 `input_text / input_image / input_json` 内容数组。题目中的 text/image 不支持缓存断点。
- `predicate`：只需要 instructions，估计该陈述为真的概率。
- `choice`：还需要 **2–255** 个唯一选项，`{value: string|boolean, description?: string}`。布尔 `true` 与字符串 `"true"` 是两个不同值。
- `score`：还需要 **2–10** 个有序等级，`{label: string, description?: string}`。等级值为位置 `0..N-1`，不把 label 解析成数值。label 无需唯一。
- `safety_identifier`、`prompt_cache_key` 可选/可为 null，分别最多 128/64 个字符。
- `prompt_cache_options` 可选/可为 null；支持 `mode: implicit|explicit|null`、`ttl: 30m`。共享输入的 text/image 可标记 `prompt_cache_breakpoint: {mode: explicit}` 或 null。
- 无 `stream`、`messages` 顶层聊天参数、`provider` 或 `upstream_url` 参数；未知字段不会静默丢弃。

官方 schema 另包含函数和自定义工具调用历史。本版明确返回 `unsupported_tool_history`，不把工具调用扁平化为 user 文本。某些尚未实现的内容类型返回 `unsupported_content`。

### 图片

- 图片仅接受 `input_image.image_url`，不支持 `file_id`。
- 支持 HTTP(S) URL 或 PNG/JPEG/WEBP/GIF 的 base64 data URL；检查 data URL 的 MIME 声明和 base64 编码，但不解析图片二进制来判断动画/真实格式/尺寸。
- 全部共享输入与全部题目 instructions 的图片**合计**计数。官方通用上限 1500 张和 512 MB；服务默认 32 张，最大可配 1500 张。
- 网关 JSON body 默认 8 MiB，最大可配 64 MiB；比官方容量更保守。远程 URL 不会由 Decora 下载，远端图片内容大小与模型 patch/context 等限制由上游检查。JSON 数据中的 `image_url` 字段只是普通数据，不会计为或加载为图片。

### 响应

顶层只有 `model`、`answers`、`usage`。Decora 将 `model` 输出为公开请求 alias；后端的模型 ID 不向外暴露。这是公开 alias 的网关约定，而非透传 OpenAI 已解析的模型名称。

答案保持题目顺序，`name` 必需，未命名题目为 null。

| 类型 | 必需的答案字段（除 type/name） |
|---|---|
| predicate | `probability` |
| choice | `choice`、`confidence`、`probabilities: [{value, probability}]` |
| score | `score`、`confidence`、`probabilities: [{value: integer, label, probability}]` |
| refusal | 无；官方合同没有 refusal 理由文本 |

`refusal` 可以替代任意一题的结果，其他题仍可正常回答。网关保留 HTTP 200，不为寻求“愿意回答的模型”而回退。

校验规则

- 概率与 confidence 为有限的 `[0,1]` 数值，confidence 不强制等于最大选项概率。
- choice 的概率必须无遗漏、无重复地覆盖原始有类型的选项，选中项是最大概率项（允许并列）。
- score 的概率必须无遗漏、无重复地覆盖原始等级索引与 label，score 等于索引的概率加权期望。
- 概率和及 score 期望的绝对误差容忍为 **0.0001**，不会重归一化、裁剪或重算上游答案。更大误差返回 `upstream_protocol_error`。

**OpenAI 原生 usage** 的所有字段都必需；本版 OpenAI adapter 始终按此严格校验，跨上游的默认兼容策略见下一节。

```json
{
  "input_tokens": 100,
  "input_tokens_details": {"cached_tokens": 10, "cache_write_tokens": 20},
  "output_tokens": 4,
  "output_tokens_details": {"reasoning_tokens": 2},
  "total_tokens": 104
}
```

这是合同示例，不是真实 API 调用记录。计数必须是非负、在官方 schema 的 int32 上界内的整数；总 token 为输入与输出之和，缓存读写之和不超过输入，推理 token 不超过输出。缓存指南明确读写计数不重叠，并均包含在 input_tokens 中。缺字段或非法数值报错，不把未知缓存、推理 token 视为 0。

## 3. 默认兼容策略与能力矩阵

三个 adapter 都实现了真实 HTTP transport，端到端 mock 检查了各自请求路径、Bearer 凭据、state/questions 映射、核心答案及 usage。接口遵循 OpenAI Decisions 的核心输入/答案结构；**usage 在默认 compatible 模式下是明确的部分兼容合同**，不是声称所有元数据完全同构。

| 能力 | OpenAI 原生 | OpenRouter 原生 | TypeSafe/Jev 原生 |
|---|---|---|---|
| 默认模式 HTTP 调用 | **支持原生结构** | **支持文本/角色/JSON/typed choice 三题型转换** | 同左 |
| usage_policy: strict | 支持完整 OpenAI usage | 提前排除，unsupported_usage | 提前排除，unsupported_usage |
| 纯文本 predicate | 原样，probability | predicate→noul，noul→probability | 同左 |
| 纯文本 choice，字符串值 | 原样，包括 descriptions | choices→criteria 对象；概率对象→数组 | 同左 |
| 布尔/字符串混合 choice | 支持原生类型区别 | 全组内部索引键，保留原值/描述，答案准确还原类型 | 同左 |
| 纯文本 score | 原样，有序 labels/description | levels→criteria 数组；legend/概率索引→原标签 | 同左 |
| 重复/省略 name | 支持 | 内部 q0/q1 键，还原原始名字和题序 | 同左 |
| 角色消息 / JSON parts | 原结构及原生角色约束 | 默认文本编码保留角色/顺序/JSON；input strict 可禁止降级 | 同左 |
| 原生角色指令层级 | 由 OpenAI 执行 | 文本标签不提供原生层级保证 | 同左 |
| input / question 图片 | 支持；共享总计数限制 | 提前过滤，可选择其他兼容候选 | 同左，不启用 danger-zone |
| refusal | 支持，保留，绝不回退 | 官方答案 union 无 refusal；不虚构映射，未知类型报错且不回退 | 同左 |
| 缓存与 safety 参数 | 支持，需候选能力声明 | 提前过滤，不静默丢弃 | 同左 |
| usage 明细 | 完整严格校验 | 保留上游提供的明细，未知省略 | 默认只有 input/output，未知明细省略 |
| usage 口径标记 | 原生 OpenAI 合同，无扩展 | usage.decora 标记 provider_reported | usage.decora 标记 billable_input |
| 校准质量保证 | 透传原生估计，不作额外保证 | 同左，无聊天 JSON 代替 | 同左 |

### usage_policy: compatible（默认）

OpenRouter 官方 `DecisionsResponse.usage` 必需 input/output token，可选 cost 和 `input_tokens_details.cached_tokens`；该 details 对象存在时官方要求 cached_tokens。TypeSafe 官方 `Usage` 只有 input/output token，input_tokens 明确是 **billable input tokens**。这些缺失不会阻止核心题型调用：

1. input/output 使用上游真实整数，非负且在公开接口的 int32 上界内；total 使用两数之和。上游若额外返回 total，必须与该和一致，不能静默覆盖矛盾统计。
2. 返回实际提供的 cached_tokens、cache_write_tokens、reasoning_tokens；未知细分字段省略，不填零。只在有已知明细时输出对应 details 对象。所有已知计数都检查非负整数及上界；OpenAI/OpenRouter 的缓存计数按总输入口径检查包含关系。TypeSafe 的 billable input 未承诺包含全部缓存 token，不对其套用 OpenAI 缓存上界，避免跨口径错误比较。
3. 新增 `usage.decora` 元数据，暴露固定提供方名称而非内部 ID、URL、凭据。含 `provider`、`input_tokens_basis`、`total_tokens_basis`、`details_complete`；存在 cost 时保留为 `provider_cost`。不透传任意上游元数据或错误正文。
4. OpenAI adapter 本来保证完整 usage，因此始终严格校验自己的官方合同；compatible 不是放宽其非法响应的开关。

TypeSafe 的兼容响应示例

```json
{
  "input_tokens": 120,
  "output_tokens": 12,
  "total_tokens": 132,
  "decora": {
    "provider": "typesafe",
    "input_tokens_basis": "billable_input",
    "total_tokens_basis": "billable_input_plus_output",
    "details_complete": false
  }
}
```

这里的 132 是**计费输入加输出**，不能用于宣称 OpenAI 精确总 token、完整上下文长度或跨上游等口径成本。OpenRouter 对应标记为 `provider: openrouter`、`input_tokens_basis: provider_reported`、`total_tokens_basis: provider_input_plus_output`。`details_complete` 仅说明三个已知细分字段是否都存在，不表示不同上游计数口径相同，也不是校准或计费认证。

### usage_policy: strict（可选）

要求完整 OpenAI usage 与计数口径，不添加兼容扩展。由于官方合同缺少明细或 input 语义不同，运行时在网络调用前排除 OpenRouter/TypeSafe；若 alias 只剩这些候选，返回 `400 unsupported_usage`。若有兼容 OpenAI 候选则继续路由它。默认 compatible 和 strict 使用完全相同的概率、confidence、评分期望、拒答和重试规则。

### 原生 HTTP 转换范围

`adapter::native_body`、`native_answers`、`native_response` 实际用于网关 HTTP 请求/响应链路，不是只有纯函数样例。

- 原生请求使用 `state` 与命名 `questions`，不是 OpenAI 的 `input/questions[]`，也不是 chat completions。字符串 input/instructions 原样使用；默认还支持经过显式文本编码的角色消息和 text/JSON 内容数组，详见下节。
- score criteria 无说明时使用 label 字符串，有说明时使用官方允许的 `{label, description}` 结构化指导；返回 legend 必须精确对应该 criteria。
- OpenRouter 把 choice/score 的 confidence、probabilities（score 还包括 legend）声明为可选，但完整核心答案需要这些信息。缺失时返回协议错误，不合成 confidence 或概率；本版也不猜测缺失 legend 的映射。
- TypeSafe 图片依赖 assets 和 `TypeSafe-Danger-Zone-Images: true`；OpenRouter 图片存在 provider 和引用位置差异。本版不做这些跨协议多模态转换，提前过滤，不会在不支持的上游上付费试错。
- 不实现 box，因为 OpenAI 目标合同没有这个题型。官方未定义的 native refusal 不映射成 0/false，不盲目换模型。

若依赖完整 OpenAI SDK 的固定 usage 明细，请选择 strict；可处理部分 usage 的客户端应使用默认 compatible，并读取 `usage.decora` 判断统计口径。

## 4. 角色、JSON 与 typed choice 的显式适配

### 输入策略及能力

`native_input_policy: textual` 为默认，只作用于 OpenRouter/TypeSafe；OpenAI 始终保留原始 input/questions 结构。

- 消息数组按官方 OpenAI 已验证 schema 接收，角色仅为 system/developer/user/assistant，保留原顺序、标签、phase 及 content 的字符串/数组形态。不会添加自定义角色，也不擅加 Decisions 不支持的 name 等消息字段。
- 字符串内容，包括空字符串，原样保留；内容数组中的 input_text/output_text/refusal 及 input_json.data 作为数据完整编码。允许空消息数组、空 content 数组、Unicode 与嵌套 JSON；仍受原请求的长度/JSON 深度等限制。
- instructions 是字符串时原样使用；是 text/JSON 内容数组时编码到 native questions.qN.instructions。predicate/choice/score 共用此规则，题内 JSON 不丢失。
- `capabilities.messages / json_input / typed_choices` 默认均为 true，可逐项设 false 禁止该候选承接对应请求。
- `native_input_policy: strict` 在网络调用前禁止消息/内容数组的文本降级，返回 unsupported_textual_lowering 或选择其他兼容候选。它不禁止可逆 typed choice 映射。此开关与 usage_policy 完全独立。

### 明确的编码与语义边界

消息数组转换为以下 JSON 对象的**字符串序列化**，置于 native state；题目内容使用相同封装，kind 为 question_content，置于 native instructions。

```json
{
  "decora_encoding": "openai_decisions_text_v1",
  "kind": "messages",
  "data": [{"role":"user","content":[{"type":"input_json","data":{"flag":false,"nested":[null,1,"中文"]}}]}]
}
```

JSON 转义明确 data 边界，保留原数据的 JSON 类型和语义，而非拼接未经转义的分隔标签。对象键顺序不作为 JSON 语义承诺；不保留用户原始 JSON 的空白/字节格式。角色标签及顺序仍可被模型读取，但**上游没有原生执行 OpenAI 的角色指令层级**；只保留数据不等于同等指令约束。该编码也不是 prompt injection 安全隔离，模型仍可能遵从数据中的恶意指令。需要原生角色层级时应选择 OpenAI，或用 input strict 排除这种转换。

真实 input_image 在编码前单独检查，即使管理员把 native 的 images 声明为 true，也不能绕过 adapter 的图片边界。共享输入和题目图片都提前排除，不把图片 URL 当已分析的视觉证据。input_json.data 中叫 role/type/image_url 的普通键保留为数据，不会触发图片处理或创建新角色。缓存断点和 safety 参数仍不做文本降级。

### 布尔/字符串选项的可逆映射

只要一组 choice 包含任意布尔值，该组所有选项统一使用 `decora_choice_0...N-1` 内部键，criteria 值为 `{value: 原始有类型JSON值, description?: 原始说明}`。示例：

```json
{
  "decora_choice_0": {"value":true,"description":"布尔真"},
  "decora_choice_1": {"value":"true","description":"字符串真"},
  "decora_choice_2": {"value":false,"description":"布尔假"},
  "decora_choice_3": {"value":"decora_choice_0","description":"与内部键同名的原始字符串"}
}
```

所有值都走同一索引映射，因此 `true` 与 `"true"`、`false` 与 `"false"`、等于内部键的原始字符串及空字符串均不碰撞。false 是明确的 bool 值，不是 missing/default。response.choice 和 probabilities 的键必须对应内部选项，随后还原原始 value 类型与题序，再验证覆盖、概率和及最高概率选择；不接受未知键、缺项或原生响应中擅自返回的布尔 choice。纯字符串组保持已有字符串键映射以减少不必要变换。

## 5. 错误和路由的最小合同

- 入站 Bearer 缺失/无效：401，统一错误体和 `WWW-Authenticate: Bearer`。
- 不存在的 alias：404；字段错误/能力 unsupported：400；body 超限：413；非 JSON：415。
- 上游 400/404/422 等：对外 400 `upstream_client_error`，不透传敏感正文；上游 401/403 是服务端问题，对外 502。
- 上游 429：按 Retry-After 等待，或对外 429 并保留建议等待秒数。
- 兼容上游耗尽：503；总超时：504；非法上游 JSON/概率/usage：502。
- HTTP 重定向不跟随；HTTP 客户端成功头之后的响应体传输错误不重放，降低重复付费和语义变更风险。
- priority/weight 是服务端配置；用户输入不参与选择任意 URL。健康检查只返回固定存活状态。
