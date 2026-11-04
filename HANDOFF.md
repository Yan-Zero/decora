# Decora 开发接续记录

## 当前结论（2026-11-04）
**三个原生 HTTP adapter 均已实现；默认 usage_policy: compatible，OpenRouter/Jev 的纯文本 predicate/choice/score 可调用。26 项测试通过，fmt/check/clippy/audit 全通过。** 不把 usage 细分字段缺失作为默认全局阻塞；未知明细省略，TypeSafe billable input 明确标记。strict 可选，仍在调用前排除无法提供完整 OpenAI usage 的候选。

## 任务边界与环境
- 独立项目 E:\source\toy\decora，原目录不存在，已新建；E:\、E:\source、E:\source\toy 未发现 CLAUDE.md，上级不是 git 仓库。
- rustc/cargo 1.95.0 已安装，无需系统安装；没有继续派发 agent。
- 只本地实现和离线测试，不部署、不付费调用、不创建远端、不推送。主会话已创建私有 GitHub 仓库，remote/推送由主会话负责。

## 官方研究与协议决策
官方资料链接和下载 SHA256 详见 docs/PROTOCOL.md；原始全量资料在 gitignore 的 reference/ 内。

- OpenAI 指南和创建接口均已核对，官方 OpenAPI 源为 https://raw.githubusercontent.com/openai/openai-openapi/master/openapi.yaml。
- OpenRouter 官方 https://openrouter.ai/openapi.json；operation-level server 为 https://openrouter.ai，覆盖全局 /api/v1，因此真实 POST 入口是 https://openrouter.ai/api/alpha/decisions。
- TypeSafe 官方 https://docs.typesafe.ai/api 和 https://api.typesafe.ai/openapi.json；真实 POST 入口 https://api.typesafe.ai/v1/systemone。
- native 使用 state + 命名 questions、noul/choice/score，不是 chat completions，也不是直接透传 OpenAI 字段。
- predicate→noul；choice→字符串键 criteria；score→从零开始的有序 criteria。内部 q0/q1 键保留重复或省略的公开 name；按原题序还原 answers。
- choice 字符串 "true" 与布尔 true 不混淆：OpenAI 支持二者；native 布尔选项在网络调用前 unsupported_boolean_choice。
- 图片总限包括 input 与所有 questions.instructions；默认 32 张、8 MiB JSON body，上限 1500 张和 64 MiB。官方通用图片上限为 1500 张、512 MB。本服务不下载图像，模型的格式/动画/patch/context 内容限制由上游检查。
- input_json 每块限制 32 层容器、10,000 个值、256 KiB 紧凑 JSON。assistant 支持 output_text/refusal 混合内容及 phase:null。
- 工具调用历史本版 unsupported_tool_history；native 多模态、角色消息、JSON parts、缓存/safety 参数及非字符串 instructions 在网络调用前过滤，可选其他兼容候选。

## Usage 策略（本轮主会话纠正后已完成）
初版 e14b4b1 将 OpenRouter/Jev 因 usage 缺项全局禁用。主会话明确要求实用的协议转换层，允许显式部分兼容 metadata；本轮已解除默认阻塞，而非只修改文档。

- Config 新增 usage_policy，缺省 compatible；strict 可选。
- compatible：native input/output 使用真实提供的整数，total 为两数之和，未知缓存/推理明细省略，绝不填 0。存在的缓存/推理值照实保留并校验。
- usage.decora 标记 provider、input_tokens_basis、total_tokens_basis、details_complete；提供方 cost 存在时保留为 provider_cost。只返回固定受控元数据，不透传任意上游字符串。
- TypeSafe 的 input_tokens_basis=billable_input，total_tokens_basis=billable_input_plus_output。合计不是 OpenAI 精确总输入/输出口径，不能将其用于同口径上下文长度或跨厂商计费推断。若提供额外缓存明细，不用 billable input 当作全部缓存 token 的上界；已测试缓存数大于 billable input 时按真实来源保留而不是错误套用 OpenAI 包含关系。
- OpenRouter 的口径标记为 provider_reported / provider_input_plus_output。官方可选 cached_tokens 明细存在时保留，缺失写入/推理 token 继续省略。
- details_complete 只是三个已知细分字段是否存在；即使 native 将来返回所有明细，也不将 billable 口径误当 strict。
- OpenAI adapter 本来保证完整 usage，因此始终严格校验原生合同；compatible 不会放过其缺项/非法响应。
- strict：提前排除 native，只剩这类候选则 400 unsupported_usage；存在兼容 OpenAI 候选则继续选择它。

## 实现文件
- src/config.rs：YAML、默认策略、端点/路由校验、环境变量凭据。
- src/protocol.rs：请求/多模态校验、核心答案、概率/评分期望、部分 usage 与来源元数据。
- src/adapter.rs：固有能力边界、实际 native request/answers/usage 转换。
- src/gateway.rs：三类真实 HTTP transport、鉴权、连接池、priority/weight、兼容过滤、重试/回退、Retry-After、全程 deadline、并发/大小限制。
- src/error.rs、src/main.rs、src/lib.rs：固定脱敏错误、启动入口、公共库。
- tests/offline.rs：离线端到端与合同边界；config.example.yaml、README.md、docs/PROTOCOL.md 为中文说明及配置/能力矩阵。

## 最终实际检查
所有命令以 && 串行运行，任一失败即中止；最后退出码 0。

| 检查 | 实际结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过，0.94 秒 |
| cargo test --locked | 1 项路由权重单测 + 25 项离线集成测试，共 26 项全部通过；集成测试 0.84 秒 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告，1.61 秒 |
| cargo audit --file Cargo.lock | 182 个锁定依赖；1403 条公告库；无漏洞/维护警告，退出码 0 |

native 端到端 mock 覆盖两种真实路径、两种三题型请求/答案转换、默认 compatible、strict 预过滤、TypeSafe billable 标记、未知明细省略、已有 0/非零明细原样保留、来源口径不混淆、OpenRouter→TypeSafe 故障回退、429、客户端错误不回退、非法概率/缺 confidence/未知 refusal/非法 usage 不重试。

此前共通过初版 19 项测试；首轮 Clippy 两项风格警告已修复。首次 audit 提示 unsafe-libyaml 停止维护（RUSTSEC-2024-0449），已替换为仅启用 deserialize 的 serde-saphyr 2.1，最终依赖无该库或维护警告。

## Git 与凭据
- 独立本地仓库，交付分支 feat/decisions-gateway；本机身份 Yan <1964649083@qq.com>。初版提交 e14b4b1bb37b6198148ad8af6b6bf25e4a6bc337，本轮兼容策略新提交以 git log -1 --oneline 为准。
- .gitignore 的 config.yaml、.env、logs/、target/、reference/ 忽略行为已用 git check-ignore 核验。
- 常见 OpenAI/GitHub/Google 密钥和私钥模式扫描无发现；测试中的 sk-offline-upstream / sk-UPSTREAM-MUST-NOT-LEAK 为明确的虚构字符串。提交前再次扫描暂存区。
- 没有真实 config.yaml，没有读取真实 API key，没有付费调用或部署。未配置 remote、未推送。

## 上线前操作与限制
1. 选择实际有权限的模型，安全设置进程环境变量；真实配置文件只允许服务账户读取。README 提供只用 TypeSafe 的完整最小配置，不需要 OpenAI key。
2. 用户授权后各做最小真实调用，核对账户权限、响应精度/可选字段与账单。本次只有离线 mock，未花费 API 额度，也不证明模型概率在业务数据上已校准。
3. 公网使用需外部 TLS 反代、防火墙、流量限制与进程守护。本版 native 仅文本子集；需要精确 OpenAI usage 的客户端使用 strict，不把省略值当 0。重试可能重复计费，需要绝对避免时 max_attempts:1。
