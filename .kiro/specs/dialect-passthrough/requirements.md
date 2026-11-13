# 需求文档：同方言原生直通（`dialect-passthrough`）

## 背景与目标

在引入多方言入站端点（`/v1/decisions`、`/v1beta/decisions`、`/v0/decisions`、`/api/v1/decisions`）后，所有请求都会先解析为内部标准中间表示（Canonical IR），再在出站时由 `adapter` 统一重编码（例如将题目名重写为 `q0/q1`、在 Gemini 下将选项键重写为 `decora_choice_0..N-1` 并注入 `usage.decora`）。

然而，当客户端使用的入站端点方言与路由命中的后端 `kind` **原本就属于同一种协议**时（例如 `/v1/decisions` -> `openai`、`/v1beta/decisions` -> `gemini`、`/v0/decisions` -> `typesafe`、`/api/v1/decisions` -> `openrouter`），进行内部键重命名与 `usage.decora` 注入是不必要的折返。

本 Spec 的目标是实现 **同方言原生直通（Same-Dialect Passthrough）**，同时完整保留网关的入站 Schema 校验、能力边界拦截、响应数学概率守恒校验以及跨方言自动故障回退能力。

---

## 需求列表

### 需求 1：按单次尝试（Per-Attempt）动态触发同方言直通
- **验收标准 1.1**：定义入站方言与后端种类的同构匹配关系：
  - `Dialect::Openai` ↔ `BackendKind::Openai`
  - `Dialect::Gemini` ↔ `BackendKind::Gemini`
  - `Dialect::Typesafe` ↔ `BackendKind::Typesafe`
  - `Dialect::Openrouter` ↔ `BackendKind::Openrouter`
- **验收标准 1.2**：当某次尝试（attempt）命中的后端与入站 `Dialect` 匹配时，出站请求直接基于客户端验证通过的原始 JSON 构造，仅将顶层 `model` 替换为管理员配置的 `backend.model`：
  - 保留调用方原始的 `questions` 键名（不重写为 `q0, q1`）；
  - 保留调用方原始的 `choice.criteria` 键名与值（在 `Gemini -> Gemini` 直通时不重写为 `decora_choice_0`）；
  - 保留调用方原始的 `score.criteria` 表达。
- **验收标准 1.3**：当同方言主后端返回 429/5xx/连接超时并回退至异构备用后端（或异构主后端回退至同方言备用后端）时，网关按每次尝试的目标后端动态切换“直通模式”与“跨方言转换模式”。

### 需求 2：直通响应的数学不变量校验与零污染透传
- **验收标准 2.1**：同方言直通绝不等于“盲目转发”。上游返回的响应仍必须经过 Decora 的严格数学与协议校验：
  - 题目键集合与题型必须与请求严格一致；
  - `noul / predicate`、`choice`、`score` 的概率必须在有限 `[0, 1]` 内，分布和及 `score` 加权期望容差 `<= 0.0001`，`choice` 选中项必须为最大概率项，`score.legend` 必须与请求 `criteria` 完全一致；
  - `usage` 各计数字段必须为非负 `i32` 且满足输入输出加和守恒。
- **验收标准 2.2**：校验通过后，同方言直通响应保持上游原生结构（仅将顶层 `model` 写回客户端请求的公开 `model` alias）：
  - **不注入 `usage.decora` 扩展字段**；
  - 在 `Openrouter -> Openrouter` 直通时，原样保留上游 `usage.cost`、`input_tokens_details`、`output_tokens_details`；
  - 在 `Gemini -> Gemini` 直通时，原样保留上游 `usage.total_input_tokens / total_output_tokens / total_tokens` 及可选的 `input_tokens_by_modality`。

### 需求 3：`usage_policy: strict` 下的同方言直通支持
- **验收标准 3.1**：当服务端配置 `usage_policy: strict` 时，同方言候选后端（例如 `/v1beta/decisions` 路由至 `kind: gemini`，或 `/v0/decisions` 路由至 `kind: typesafe`）因为返回的就是调用方方言期望的完整原生 usage，**不被 `unsupported_usage` 过滤排除**。
- **验收标准 3.2**：在 `usage_policy: strict` 下，跨方言的非 OpenAI 转换候选仍然在请求前被排除（若某路由仅含异构非完整 usage 候选，仍返回 `400 unsupported_usage`）。
