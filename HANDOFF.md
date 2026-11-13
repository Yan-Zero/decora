# Decora 开发接续记录

## 当前结论：同方言原生直通（Same-Dialect Passthrough）与异构自动转换协同全通
**Decora 现已在对称中心辐射式（Hub-and-Spoke）多方言架构上支持“按单次尝试动态选择同方言原生直通或异构跨方言自动转换”。43 项测试通过（1 项单测 + 42 项离线集成测试），fmt/check/clippy/audit 全通过。**

**尚未完全符合原生 schema 的明确例外：跨方言路由在 `usage_policy: compatible` 下仍允许省略目标方言或 OpenAI 未知的必需明细并增加 `usage.decora`。** 同方言原生直通则原样返回上游原生 `usage`（不注入 `usage.decora`），且在 `usage_policy: strict` 下正常放行。

## 本轮任务与 Git 起点
- 任务：完成 `.kiro/specs/dialect-passthrough/` 规范全部 5 项任务，实现 `OpenAI↔OpenAI`、`Gemini↔Gemini`、`TypeSafe↔TypeSafe`、`OpenRouter↔OpenRouter` 同方言原生直通及跨方言自动回退。
- 路径 `E:\source\toy\decora`；分支 `feat/decisions-gateway`。只本地修改、测试与提交，不推送、不访问付费 API。

## 本轮同方言直通核心实现

### 1. 同构匹配与按尝试（Per-Attempt）动态切换（`src/dialect.rs` + `src/gateway.rs`）
- `Dialect::matches_backend(kind)` 定义四类同构配对：`Openai↔Openai`、`Gemini↔Gemini`、`Typesafe↔Typesafe`、`Openrouter↔Openrouter`。
- `Dialect::passthrough_body`：在同方言直通模式下直接克隆已通过入站校验的原始请求 JSON，仅将顶层 `model` 替换为 `backend.model`，完整保留客户端原始 `questions` 键名、选项键名与 `criteria`（不再重写为 `q0/q1` 或 `decora_choice_0`）。
- `Gateway::execute`：在每次 `attempt` 循环中动态判断 `dialect.matches_backend(backend.config.kind)`；若同方言主后端因 429/5xx/超时回退至异构备用后端，下一次尝试自动切换为跨方言 Canonical IR 转换（`native_body_for` / `format_response`）。

### 2. 直通响应的严格校验与零污染透传（`src/dialect.rs`）
- `Dialect::passthrough_response`：复用了 `DecisionResponse::parse_with_usage` 与原生 `usage` 校验逻辑，严格检查概率有限 `[0, 1]`、分布和与 `score` 加权期望容差 `<= 0.0001`、`choice` 最大概率选中项、`score.legend == raw_request["questions"][key]["criteria"][i]` 以及 token 加和守恒。
- 校验通过后仅将顶层 `model` 还原为公开 `model` alias，原样返回原生 `answers` 与 `usage`（**不注入 `usage.decora`**，完整保留 OpenRouter `usage.cost` 与 Gemini `input_tokens_by_modality`）。

### 3. `usage_policy: strict` 放行同方言候选（`src/adapter.rs`）
- `compatible_for_dialect` 在 `usage_policy: strict` 下放行同方言原生候选（例如 `/v1beta/decisions -> gemini`、`/v0/decisions -> typesafe`、`/api/v1/decisions -> openrouter`），同时继续在网络调用前拦截跨方言非 OpenAI 候选（返回 `400 unsupported_usage`）与真实 `input_image`（非 OpenAI 原生后端返回 `400 unsupported_native_images`）。

## 验证结果（2026-11-13）
按顺序串行执行：

| 检查 | 实际结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过（1.00 秒） |
| cargo test --locked | 1 项单测 + 42 项离线集成测试，共 43 项通过、0 失败；集成测试 0.84 秒 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告（2.60 秒） |
| cargo audit --file Cargo.lock | 182 个锁定依赖，1490 条公告库，0 漏洞，1 项 yanked 警告允许 |

新增与更新测试重点：
- `multi_dialect_gemini_inbound_routes_to_gemini_and_falls_back_to_openai`：第 1 步验证 `Gemini -> Gemini` 同方言直通（保留原始键名 `"a_damaged"`、`"b_route"`、`"c_level"` 与 `"a_support"`、`"b_sales"`，无 `usage.decora`）；第 2 步验证 Gemini 503 自动回退至 OpenAI 后端并切换为跨方言转换（含 `usage.decora.provider == "openai"`）。
- `same_dialect_passthrough_preserves_native_keys_and_works_under_strict_usage`：验证 `/v0/decisions -> typesafe`、`/api/v1/decisions -> openrouter`（保留原生 `usage.cost` 与中文键名，无 `usage.decora`）以及 `/v1beta/decisions -> gemini` 在 `usage_policy: strict` 下全部直通成功；同时验证跨方言非 OpenAI 路由在 `strict` 下仍返回 `400 unsupported_usage`，以及直通模式下非法概率、篡改 `legend` 与非法 `usage` 均被拦截为 `502 upstream_protocol_error` 且不回退。

## 文件与安全交付
- `src/adapter.rs`、`src/dialect.rs`、`src/gateway.rs`：同方言直通与异构回退切换逻辑。
- `tests/offline.rs`：更新 Gemini 直通与跨方言回退测试，新增同方言直通与 `strict` 模式测试（总数增至 43 项）。
- `README.md`、`docs/PROTOCOL.md`、`HANDOFF.md`、`.kiro/specs/dialect-passthrough/tasks.md`：文档与 Spec 清单全部同步更新并勾选 `[x]`。

## 后续仍需
1. 主会话推送本轮本地提交。
2. 用户授权后做最小真实账户调用，核对各官方端点模型权限、响应与账单；本项目只做了离线 loopback mock 验证，没有花费真实 API 额度。
3. 公网仍需 TLS 反代、防火墙、限流与进程守护。
