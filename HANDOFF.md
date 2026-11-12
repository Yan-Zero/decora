# Decora 开发接续记录

## 当前结论：支持 Google Gemini 原生 Decisions 后端与四家上游一致性
**所有公开 alias 使用同一个 /v1/decisions、同一种 OpenAI 形状的 model/input/questions 请求及核心 answers。原生后端支持 OpenAI、Google Gemini、OpenRouter、TypeSafe/Jev。四家后端在相同 OpenAI 结构化请求下返回完全相同的核心答案。38 项测试通过，fmt/check/clippy/audit 全通过。**

**尚未完全符合 OpenAI schema 的明确例外：usage compatible 仍允许省略必需细分并增加 usage.decora。** 本轮完整实现 Gemini 原生 usage 映射，不伪造未知零值，Gemini `total_input_tokens` 与 TypeSafe `billable_input` 均明确标明口径，不将整个响应描述为严格完全一致。

## 本轮任务与 Git 起点
- 任务：按照 `.kiro/specs/gemini-backend/` 规范执行 6 项任务，为 Decora 新增 Google Gemini 原生 Decisions API（`kind: gemini`，官方端点 `https://generativelanguage.googleapis.com/v1beta/decisions`）后端支持。
- 开工核对 git：工作区干净，`feat/decisions-gateway` 跟踪 `origin/main`，起点提交 `695ab2a 恢复对 .kiro 的追踪`。
- 本轮只本地修改、测试、追加提交，不推送、不创建远端、不付费调用、不部署、不再派发 agent。
- 路径 `E:\source\toy\decora`；已有 rustc/cargo 1.95.0。主会话负责后续推送。

## 本轮 Gemini 后端核心实现

### 1. 配置与鉴权扩展
- `src/config.rs`：`BackendKind` 新增 `Gemini` 变体，默认官方入口为 `https://generativelanguage.googleapis.com/v1beta/decisions`。
- `src/gateway.rs`：出站请求时对 `BackendKind::Gemini` 分派 `x-goog-api-key: $KEY` 请求头（不发 `Authorization: Bearer`，且绝不转发客户端 Bearer 凭据）；其余后端保持 `Authorization: Bearer`。

### 2. 外部合同固定与内部无损适配
- 外部请求保持固定 OpenAI Decisions 格式，网关内部调用 `adapter::native_body_for(req, model, kind)` 与 `adapter::native_answers_for(value, req, kind)`：
  - 顶层输入：Gemini 使用 `"input": textual(&req.input, "messages")`（非 OpenRouter 的 `"state"`），字符串原样透传，角色消息数组自动使用 `openai_decisions_text_v1` 文本编码序列化。
  - `predicate`：映射为 `{"type": "noul", "instructions": ...}`。
  - `choice`：Gemini 限制选项键为 1..=128 位纯 ASCII 且描述必须为字符串。对 Gemini 统一采用 `decora_choice_0..N-1` 内部键，并在 criteria 中序列化紧凑 JSON 字符串（含原始 typed `value` 及可选 `description`）。响应解析按索引精确还原 `ChoiceValue`，无损支持中文（如 `"售后"`）、空字符串、首尾空格、布尔 `true`/`false`、`"true"`/`"false"` 及内部键同名选项。
  - `score`：Gemini 限制 criteria 必须为非空字符串数组 `string[]`。无描述且非空 label 直接使用 label 字符串；空 label 或带 description 的等级转换为紧凑 JSON 字符串。响应逐项比对 `legend` 后还原原始等级索引与 label。
  - 顶层 `model`：Gemini 官方响应无顶层 `model` 字段（若存在则要求为非空字符串）；对外响应统一定位到客户端请求的公开 alias。

### 3. Usage 映射与真实能力筛选
- `usage_policy: compatible` 下：严格校验 Gemini 的 `total_input_tokens`、`total_output_tokens`、`total_tokens` 为非负 `i32` 整数且满足两数之和。省略未知明细（不填 0）。
- 在 `usage.decora` 中标记：
  - `provider: "gemini"`
  - `input_tokens_basis: "total_input_tokens"`
  - `total_tokens_basis: "total_input_plus_output"`
  - `details_complete: false`
  - 若包含 `input_tokens_by_modality`：严格校验元素为含非空 `modality` 与非负 `tokens` 的对象，原样保留在 `usage.decora.input_tokens_by_modality` 中。
- 能力前置拦截：真实 `input_image` 统一拦截为 `400 unsupported_native_images`（即使配置 `images: true`）；缓存与安全参数报 `unsupported_native_mapping`；`strict` usage 模式报 `unsupported_usage`。

## 验证结果（2026-11-13）
按顺序串行执行：

| 检查 | 实际结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过 |
| cargo test --locked | 1 项单测 + 37 项离线集成测试，共 38 项通过、0 失败；集成测试 0.82 秒 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告 |
| cargo audit --file Cargo.lock | 182 个锁定依赖，1490 条公告库，0 漏洞，1 项 yanked 警告允许 |

新增/替换回归重点：
- **四家上游一致性**：在 `identical_openai_request_and_answers_contract_for_every_backend_alias` 中加入 Gemini，验证 OpenAI、Gemini、OpenRouter、TypeSafe 四家后端在相同 OpenAI 结构化请求（含系统指令、题内 JSON、布尔与字符串混合 choice）下返回完全一致的核心 `answers` 与公开 `model`。
- **Gemini HTTP 与鉴权**：验证出站 `x-goog-api-key` 请求头（无 `Authorization` 头）、顶层 `input` 字段、紧凑 JSON criteria 字符串往返。
- **中文与特殊等级评分**：验证中文 choice（`"售后"`）、空 label score 等级、布尔 choice 与同名键无冲突。
- **Gemini Usage 与 Modality**：验证 `total_input_tokens` 映射、省略未知明细、`input_tokens_by_modality` 合法保留与非法格式拦截。
- **故障与跨协议回退**：Gemini 503 回退到 TypeSafe、非法概率/Usage 不回退、`strict` 模式提前过滤、图片前置拦截。

## 文件与安全交付
- `src/config.rs`：新增 `BackendKind::Gemini` 及默认端点 `https://generativelanguage.googleapis.com/v1beta/decisions`。
- `src/protocol.rs`：`UsageProvider`、`InputTokenBasis`、`TotalTokenBasis` 新增 Gemini 对应枚举变体与 `input_tokens_by_modality` 校验。
- `src/adapter.rs`：新增 `native_body_for`、`native_answers_for` 及 Gemini 专有的 criteria/usage 解析。
- `src/gateway.rs`：调用 `native_body_for` 并分派 `x-goog-api-key` 请求头。
- `tests/offline.rs`：四后端统一回归与 4 项新增 Gemini 离线测试（总数增至 38 项）。
- `config.example.yaml`、`README.md`、`docs/PROTOCOL.md`、`.kiro/steering/`、`.kiro/specs/gemini-backend/tasks.md`：文档与 Spec 清单同步勾选。

## 后续仍需
1. 主会话推送本轮本地提交。
2. 用户授权后做最小真实账户调用，核对 Gemini 模型权限、响应与账单；本项目只做了离线验证，没有花费真实 API 额度。
3. 公网仍需 TLS 反代、防火墙、限流与进程守护。
