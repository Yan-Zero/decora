# Decora 开发接续记录

## 任务边界
独立 Rust OpenAI Decisions 协议网关；目录 E:\source\toy\decora。只在本地实现、离线测试，不部署、不付费调用、不创建远端、不推送、不继续派发 agent。主会话已创建私有 GitHub 仓库，remote/推送由主会话负责。

## 初查（2026-11-04）
- 目标目录原不存在，已新建；E:\、E:\source、E:\source\toy 没有发现 CLAUDE.md。上级不是 git 仓库。
- 已有 rustc/cargo 1.95.0，无需安装工具链。

## 官方协议研究（进行中）
- https://developers.openai.com/api/docs/guides/decisions
- https://developers.openai.com/api/reference/resources/decisions/methods/create
  - POST /v1/decisions；model/input/questions，1–400 个题目，predicate/choice/score；概率是原生协议输出，不能用聊天 JSON 冒充。
  - input 支持字符串/消息数组，instructions 支持文本/内容数组；input_image 不支持 file_id。需要进一步核对图像数量限制。
  - score levels 为有序 label 列表，不是任意数值刻度；结果包含概率分布、confidence、score；refusal 是独立答案类型。
- https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-questions-and-answers-request
  - 原生 POST /api/alpha/decisions，state + 命名 questions；noul/choice/score/box，不能当成 OpenAI 原样透传。
- https://docs.typesafe.ai/api
  - 官方提供 https://api.typesafe.ai/openapi.json；POST /v1/systemone，state + 命名 questions，noul/choice/score/box。
  - 图像依赖 danger-zone header/assets；需核对 schema 后决定安全兼容边界。
- 搜索出现大量名称相近的第三方域名，均不作为协议依据。

## 实施计划
1. 核对官方完整 schema，明确保真映射/拒绝能力边界并保存文档。
2. axum/tokio/reqwest 实现：配置校验、凭据环境变量、loopback 默认监听、可配 Bearer 入站鉴权、严格请求与返回校验。
3. alias→候选池，能力过滤后按 priority/weight 选择，受限重试/回退、Retry-After、连接复用与 redirect 禁用。
4. 离线 mock 覆盖协议/路由/鉴权/错误/多模态/拒答/概率/usage。
5. 实跑 fmt/check/test/clippy，安全检查与中文说明，更新本记录。

## 已核实的关键边界
- 已下载官方 OpenAPI 到 gitignore 的 reference/ 下。OpenAI 源 https://raw.githubusercontent.com/openai/openai-openapi/master/openapi.yaml，OpenRouter https://openrouter.ai/openapi.json，TypeSafe https://api.typesafe.ai/openapi.json。
- OpenAI 完整 Usage 强制要求 input_tokens_details.cached_tokens/cache_write_tokens 和 output_tokens_details.reasoning_tokens；OpenRouter/TypeSafe 官方 schema 不提供完整明细。严格 API 下这两个 adapter 提前返回 unsupported_usage，不产生明知无法返回完整合同的上游付费请求。原生 request/answers 映射单独实现供离线验证，不能当作生产可用。
- OpenAI score 以等级位置 0..N-1 计分；ChoiceValue 字符串和布尔类型不混淆；name 可省略、允许重复，响应为 null 或原名称。
- 图像官方通用上限 1500 张/请求、512 MB，总计 input 与各题 instructions；本服务更保守默认 32 张/8 MiB body，配置上限 64 MiB。网关不下载图像、不解码检查动画/patch，模型的图片内容/分辨率约束由上游检查，4xx 不回退。
- 官方新增工具调用历史存在，但本版明确 unsupported_tool_history，不冒充完整工具历史支持。

## 最终实现与验证
- 已完成 src/{config,error,protocol,adapter,gateway,lib,main}.rs、config.example.yaml、tests/offline.rs、中文 README.md 和 docs/PROTOCOL.md。
- 实现 OpenAI 原生三题型和拒答、角色消息/JSON/共享与题目图片、缓存参数、完整 usage、固定 alias 路由、priority/weight、兼容候选过滤、有界重试/回退、Retry-After 秒数/日期、鉴权、双向 body 限制、并发限制、TLS 校验/redirect 禁用/连接复用及固定脱敏日志。
- 严格全程 deadline 从鉴权后的入站 body 接收开始计算；已增加慢速上传后收到 429 的回归测试，确认不会因读取 body 耗时而重置上游重试预算。
- 原生 OpenRouter/TypeSafe 文本 request/answers 转换已测试，但运行时提前返回 unsupported_usage；这是已核实的合同缺口，不是已上线的 adapter。

### 最终实际检查（2026-11-04）
串行命令使用 &&，任一失败都会中止；最终退出码 0。

| 检查 | 实际结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过，dev profile 1.14 秒 |
| cargo test --locked | 1 项路由权重单测 + 18 项离线集成测试，共 19 项全部通过；集成测试 0.83 秒 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告，1.82 秒 |
| cargo audit --file Cargo.lock | 182 个锁定依赖，1403 条公告库；无漏洞/维护警告，退出码 0 |

首轮测试 13 项通过，Clippy 两项风格警告已修复；按官方 OpenAPI 修正 assistant 混合内容/phase null，补齐 input_json 限制。额外 audit 曾提示 unsafe-libyaml 停止维护（RUSTSEC-2024-0449），已替换为仅启用 deserialize 的 serde-saphyr 2.1 并完成以上全量复验，最终依赖已无该库。

### Git 与凭据检查
- 已初始化独立本地 git；为遵守默认分支提交前建分支的约定，交付分支为 feat/decisions-gateway。本机 git 身份 Yan <1964649083@qq.com>；最终提交以 git log -1 --oneline 为准。
- .gitignore 的 config.yaml、.env、logs/、target/、reference/ 忽略行为已用 git check-ignore 核验。
- 已扫描常见 OpenAI/GitHub/Google 密钥及私钥模式，无匹配。测试里的 sk-offline-upstream / sk-UPSTREAM-MUST-NOT-LEAK 均为明示的虚构字符串。
- 没有创建真实 config.yaml，没有读取真实 API key，没有付费调用或部署。未配置 remote、未推送；GitHub 仓库与推送由主会话处理。

## 上线前剩余操作
1. 用户设置真实进程环境变量，复制示例配置并限制文件权限；核对实际账户模型权限。
2. 在用户授权下发起一个最小真实请求，核对响应合同和账单；本项目的验证全为本机离线 mock。
3. 公网使用需外部 TLS 反代、防火墙、流量限制与进程守护。OpenRouter/Jev 要等完整 usage 合同补齐并重新审查后才能启用。
