# Decora 开发接续记录

## 当前结论：对称中心辐射式（Hub-and-Spoke）多方言架构与四端点全通
**Decora 现已支持多前端方言（OpenAI、Gemini、TypeSafe、OpenRouter）通过内部标准中间表示（Canonical Decision IR）跨任意兼容上游路由与故障回退。42 项测试通过（1 项单测 + 41 项离线集成测试），fmt/check/clippy/audit 全通过。**

**尚未完全符合原生 schema 的明确例外：usage compatible 仍允许省略目标方言或 OpenAI 未知的必需明细并增加 `usage.decora`。** 本轮完整保留该服务端管理行为，不伪造未知零值，不将整个响应描述为严格完全一致。

## 本轮任务与 Git 起点
- 任务：完成 `.kiro/specs/multi-dialect-endpoints/` 规范全部 6 项任务，构建 **“多前端方言（Dialect） ↔ 内部标准中间表示（Canonical Decision IR） ↔ 多后端适配器（Adapter）”** 架构，原生挂载四类官方入站端点与双轨入站鉴权。
- 路径 `E:\source\toy\decora`；分支 `feat/decisions-gateway`。只本地修改、测试与提交，不推送、不访问付费 API。

## 本轮多方言架构与核心实现

### 1. 对称中心辐射式重构（`src/dialect.rs` + `src/adapter.rs` + `src/protocol.rs`）
- **`src/dialect.rs`（新增）**：定义 `Dialect::{Openai, Gemini, Typesafe, Openrouter}`，提供 `parse_request`（入站严格校验并归一化至 Canonical `DecisionRequest`）与 `format_response`（出站按入站方言重排并格式化）。
  - Gemini `input` 支持纯文本与 `[{"text": ...}, {"inline_data": {"mime_type": ..., "data": ...}}]` 多模态数组，归一化为标准 `input_text` / `input_image` 交由统一校验器执行 base64、MIME 与图片数量上限拦截。
  - 若非 OpenAI 入站端点（Gemini、TypeSafe、OpenRouter）收到 OpenAI 上游返回的逐题 `Answer::Refusal`，明确返回 `502 upstream_refusal_unrepresentable`，绝不伪造 `0.0` 或触发跨模型回退。
- **`src/protocol.rs`**：`UsageProvider` 新增 `Openai`、`InputTokenBasis` 新增 `OpenaiInputTokens`、`TotalTokenBasis` 新增 `OpenaiInputPlusOutput`，用于非 OpenAI 入站端点路由至 OpenAI 后端时的透明来源标记。
- **`src/adapter.rs`**：提取 `choice_key_for`、`score_criterion` 与 `non_negative_i32`，消除 `native_answers_for` 重新构建整份 `native_body_for` 的冗余开销及重复解析闭包。

### 2. 四方言入站端点与双轨入站鉴权（`src/gateway.rs`）
- 挂载四个独立端点：
  - `POST /v1/decisions` -> `Dialect::Openai`
  - `POST /v1beta/decisions` -> `Dialect::Gemini`
  - `POST /v0/decisions` -> `Dialect::Typesafe`
  - `POST /api/v1/decisions` -> `Dialect::Openrouter`
- 严格方言边界：禁止在同一路径上混用其他方言字段（如向 `/v1/decisions` 发 `state`、向 `/v1beta/decisions` 发 `state` 或数组 `questions`、向 `/v0/decisions` 发 `input` 或数组 `questions`），一律返回 `400 invalid_request`。
- 双轨入站鉴权：在 `guard` 中支持单一定长比较（`ConstantTimeEq`）的 `Authorization: Bearer <key>` 或 `x-goog-api-key: <key>`；若同时携带两种头、重复同名头或凭据无效，在读取请求体前立即返回 `401 invalid_api_key`（附 `WWW-Authenticate: Bearer`），且绝不向上游转发客户端入站凭据。

## 验证结果（2026-11-13）
按顺序串行执行：

| 检查 | 实际结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过 |
| cargo test --locked | 1 项单测 + 41 项离线集成测试，共 42 项通过、0 失败；集成测试 0.83 秒 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告 |
| cargo audit --file Cargo.lock | 182 个锁定依赖，1490 条公告库，0 漏洞，1 项 yanked 警告允许 |

新增 4 项多方言离线集成测试重点：
- `multi_dialect_gemini_inbound_routes_to_gemini_and_falls_back_to_openai`：Gemini `/v1beta/decisions` 使用 `x-goog-api-key` 鉴权往返三题型、Gemini 503 回退 OpenAI（校验 `usage.decora.provider == "openai"` 与 Gemini usage 口径）、Gemini `inline_data` 图片数组路由至 OpenAI 视觉后端并拦截纯文本非视觉后端（`400 unsupported_native_images`）。
- `multi_dialect_typesafe_and_openrouter_inbound_with_chinese_and_score_descriptions`：TypeSafe `/v0/decisions` 与 OpenRouter `/api/v1/decisions` 发送含中文选项（`"售后"`）及带 `description` 的 `score.criteria`，路由至 Gemini/OpenAI 后端并精准还原 `answers` 与 `score.legend`。
- `strict_dialect_boundaries_and_dual_auth_conflict_rejections`：跨方言字段混用均返回 `400 invalid_request` 且不触达上游；同时发送 `Authorization` 与 `x-goog-api-key` 或重复头均返回 `401 invalid_api_key`。
- `non_openai_dialect_returns_502_unrepresentable_on_openai_refusal_without_fallback`：`/v1beta/decisions`、`/v0/decisions`、`/api/v1/decisions` 遇到 OpenAI 逐题 `refusal` 时返回 `502 upstream_refusal_unrepresentable` 且不触发回退。

## 文件与安全交付
- `src/dialect.rs`、`src/lib.rs`：新增并导出多方言编解码层。
- `src/protocol.rs`、`src/adapter.rs`、`src/gateway.rs`：支持 Hub-and-Spoke、四端点路由与双轨定长鉴权。
- `tests/offline.rs`：新增 4 项多方言与跨协议边界集成测试（总数增至 42 项）。
- `README.md`、`docs/PROTOCOL.md`、`.kiro/steering/`、`.kiro/specs/multi-dialect-endpoints/`：文档与 Spec 任务清单全部更新并勾选 `[x]`。

## 后续仍需
1. 主会话推送本轮本地提交。
2. 用户授权后做最小真实账户调用，核对各官方端点模型权限、响应与账单；本项目只做了离线 loopback mock 验证，没有花费真实 API 额度。
3. 公网仍需 TLS 反代、防火墙、限流与进程守护。
