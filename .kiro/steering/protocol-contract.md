---
inclusion: fileMatch
fileMatchPattern: "{src/adapter.rs,src/protocol.rs,src/gateway.rs,tests/offline.rs,docs/PROTOCOL.md}"
---

# 协议合同要点（编辑协议相关文件时自动加载）

完整依据见 #[[file:docs/PROTOCOL.md]]。官方 schema 在 `reference/`（本机，未入库）；扩展 adapter 前应重新核对官方文档，不要照旧假定。

## OpenAI 请求限制

- `questions` 1–400 项；`name` 可省略/重复/为空串，但不能显式 null。
- `choice`：2–255 个唯一选项，值为 string 或 bool（`true` 与 `"true"` 不同）。
- `score`：2–10 个有序等级，按**位置 0..N-1** 计分，不解析 label 数值，label 可重复。
- 角色仅 system / developer / user / assistant；`phase` 为 commentary / final_answer / null。
- `input_json.data` 须为对象或对象数组：≤32 层、≤10,000 个值、≤256 KiB。
- 图片：仅 `image_url`（HTTP(S) 或 PNG/JPEG/WEBP/GIF 的 data URL），不接受 `file_id`；input 与全部题目 instructions 合计默认 ≤32 张，可配至 1500；body 默认 8 MiB，最大 64 MiB。
- `safety_identifier` ≤128 字符，`prompt_cache_key` ≤64 字符。

## 答案校验（不得放宽）

- 概率、confidence 为有限 `[0,1]`；confidence 不要求等于最大概率。
- choice 概率须无遗漏、无重复覆盖原始有类型选项，选中项为最大概率项（可并列）。
- score 概率须覆盖全部索引与 label，score = 概率加权期望；和与期望容差 **0.0001**，超出返回 `upstream_protocol_error`；不重归一化。
- OpenAI 拒答以 HTTP 200 `type: refusal` 保留，其他题照常返回，**绝不因拒答回退**。

## 内部映射不变量

- OpenRouter/Jev：predicate→`noul`，choice→criteria 对象，score→criteria 数组；重复/省略 name 用内部 `q0/q1…` 键并还原。
- 含布尔的 choice 组：整组改用 `decora_choice_0…N-1` 内部键，criteria 保留原始 JSON 类型的 value/description；还原时不得碰撞（等于内部键的字符串、空字符串、`false`）。纯字符串组保持原字符串键。
- 消息/JSON/text 数组被序列化为一个字符串：`{"decora_encoding":"openai_decisions_text_v1","kind":"messages"|"question_content","data":...}`。保留 JSON 语义，不承诺空白和键顺序。
- 角色标签 ≠ 原生指令层级；JSON 边界 ≠ prompt injection 隔离。文档中不得暗示相反。
- JSON 数据里的 `role/type/image_url` 键只是普通数据。
- 真实 `input_image` 在编码前独立排除 OpenRouter/Jev，`capabilities.images: true` 不能扩大 adapter 固有能力。

## usage

- OpenAI 原生：所有字段必需、严格校验（缓存读写不重叠且都含于 input；推理 ≤ output；total = input+output；int32 上界）。
- compatible：保留真实已知值，未知省略不填 0；`usage.decora` 标记 `provider / input_tokens_basis / total_tokens_basis / details_complete`，有 cost 时为 `provider_cost`。TypeSafe 为 `billable_input`，不套用 OpenAI 缓存包含关系。
- strict：请求前排除 OpenRouter/Jev，只剩它们则 `400 unsupported_usage`。
- 概率、confidence、重试规则在两种模式下完全相同。

## 重试与回退

- 仅连接失败、发送阶段超时、HTTP 429/500/502/503/504 可回退；`max_attempts` 是整个请求的**总调用数**。
- 429 的 `Retry-After`（整数秒或 HTTP 日期）在尝试下一个候选前等待；若超出剩余总预算，立即返回 429 并带建议等待秒数。
- 客户端错误、上游 401/403、拒答、非法概率/usage、重定向、成功头之后的 body 传输失败**不回退**。
- 上游 400/404/422 → 对外 400 `upstream_client_error`；上游 401/403 → 对外 502；不回显上游正文。
