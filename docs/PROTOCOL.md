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

这些在线入口可能更新；将来启用 adapter 或扩展边界时需要重新核对，而不是照旧假定。

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

usage 的所有字段都必需

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

## 3. 能力矩阵

“转换可测”与“严格 API 可用”分开列出，避免将 partial adapter 包装为生产支持。

| 能力 | OpenAI 原生 | OpenRouter 原生转换 | TypeSafe/Jev 原生转换 |
|---|---|---|---|
| 严格 `/v1/decisions` 可调用 | **支持**，需账户权限 | **不可用：unsupported_usage** | **不可用：unsupported_usage** |
| 纯文本 predicate | 原样，probability | predicate→noul，noul→probability | 同左 |
| 纯文本 choice，字符串值 | 原样，包括 descriptions | choices→criteria 对象；概率对象→数组 | 同左 |
| choice 布尔值 | 支持，保留类型区别 | 明确 unsupported_boolean_choice | 同左 |
| 纯文本 score | 原样，有序 labels/description | levels→criteria 数组；legend/概率索引→原标签 | 同左 |
| 重复/省略 name | 支持 | 内部 q0/q1 键，还原原始名字和题序 | 同左 |
| 角色消息 / JSON parts | 支持对应角色约束 | 本版不映射，unsupported_native_mapping | 同左 |
| input / question 图片 | 支持；共享总计数限制 | 不映射，不猜测 provider 能力 | 不映射，不启用 danger-zone |
| refusal | 支持，保留，绝不回退 | 官方答案 union 无 refusal；不虚构映射 | 同左 |
| 缓存与 safety 参数 | 支持，需候选能力声明 | 不静默丢弃，映射函数报 unsupported | 同左 |
| 完整 usage | 支持且严格校验 | **缺 cache_write_tokens/reasoning_tokens** | **缺 cache/read/write/reasoning 明细** |
| 校准质量保证 | 透传原生估计，不作额外保证 | 同左，无聊天 JSON 代替 | 同左 |

### 为什么现在禁用后两者

OpenRouter 官方 `DecisionsResponse.usage` 只有必需的 input/output token，可选 cost 和 cached_tokens；TypeSafe 官方 `Usage` 只有 input/output token，而且 input_tokens 的语义是 billable input tokens。即使能计算两数之和，也无法据此推断缓存写入和推理 token，更不能保证 billable input 与 OpenAI 总输入的口径完全相同。

本版没有引入“猜 0”“假定缓存关闭”“把 JSON 内一个小数解释为概率”或非标准 `usage: null`。运行时能力检查会在任何网络调用之前排除这两类候选；若 alias 只剩这种候选，则返回 `400 unsupported_usage`。若另有兼容 OpenAI 候选则正常选择兼容候选。

### 原生转换实现的范围

`adapter::native_body` 和 `adapter::native_answers` 是公开、可测试的协议转换函数，**不是可绕过 usage 限制的网络入口**。

- 原生请求使用 `state` 与命名 `questions`，不是 OpenAI 的 `input/questions[]`，也不是 chat completions。
- score criteria 无说明时使用 label 字符串，有说明时使用官方允许的 `{label, description}` 结构化指导；返回 legend 必须精确对应该 criteria。
- OpenRouter 把 choice/score 的 confidence、probabilities 声明为可选，但 OpenAI 必需。缺失时返回协议错误，不合成 confidence 或概率。
- TypeSafe 图片依赖 assets 和 `TypeSafe-Danger-Zone-Images: true`；OpenRouter 图片还存在 provider 与引用位置差异。本版不尝试跨协议降级这些多模态语义。
- 不实现 box，因为 OpenAI 目标合同没有这个题型。

未来若上游提供完整且有明确语义的 usage，应先更新官方依据与测试，再允许路由；若想要一个保留各上游 usage 原貌的跨厂商 API，那是另一个公开合同，不应伪装成 OpenAI Decisions。

## 4. 错误和路由的最小合同

- 入站 Bearer 缺失/无效：401，统一错误体和 `WWW-Authenticate: Bearer`。
- 不存在的 alias：404；字段错误/能力 unsupported：400；body 超限：413；非 JSON：415。
- 上游 400/404/422 等：对外 400 `upstream_client_error`，不透传敏感正文；上游 401/403 是服务端问题，对外 502。
- 上游 429：按 Retry-After 等待，或对外 429 并保留建议等待秒数。
- 兼容上游耗尽：503；总超时：504；非法上游 JSON/概率/usage：502。
- HTTP 重定向不跟随；HTTP 客户端成功头之后的响应体传输错误不重放，降低重复付费和语义变更风险。
- priority/weight 是服务端配置；用户输入不参与选择任意 URL。健康检查只返回固定存活状态。
