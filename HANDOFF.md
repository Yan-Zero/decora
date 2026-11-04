# Decora 开发接续记录

## 当前结论：固定客户端合同，自动内部转换
**所有公开 alias 使用同一个 /v1/decisions、同一种 OpenAI 形状的 model/input/questions 请求及核心 answers。角色消息、JSON parts、typed choice 自动内部适配，不再有输入转换策略或相关能力开关。34 项测试通过，fmt/check/clippy/audit 全通过。**

**尚未完全符合 OpenAI schema 的明确例外：usage compatible 仍允许省略必需细分并增加 usage.decora。** 本轮按主会话要求保留现有服务端 usage 管理选项，不伪造未知零值，不将整个响应描述为严格完全一致。

## 本轮任务与 Git 起点
- 用户纠正架构：转换是网关内部职责，客户端不需要知道 Jev/OpenRouter 输入格式，不应选择 textual/strict 输入转换策略。
- 开工先核对 git：工作区干净，feat/decisions-gateway 跟踪 origin/main；主会话已推送上一提交 16263247e5dda739f0525f30dd82002c65796997。
- 本轮只本地修改、测试、追加提交，不推送、不创建远端、不付费调用、不部署、不再派发 agent。
- 路径 E:\source\toy\decora；已有 rustc/cargo 1.95.0，无需安装。主会话负责后续推送。

## 本轮实际删除及保留

### 删除的转换控制面
- 移除 NativeInputPolicy 枚举、config.native_input_policy、native_body_with_policy 及结构化输入的 strict 拒绝分支。
- 移除 capabilities.messages/json_input/typed_choices 字段及相应筛选；不会仅因为角色/JSON/typed choice 需要内部编码而 unsupported。
- 移除 unsupported_textual_lowering 的执行路径。native_body 自动接受当前已映射的合法结构。
- 配置示例和公共 README/PROTOCOL 不再要求选择输入转换策略。
- 升级旧服务端配置时应删除上述已移除字段；配置仍 deny_unknown_fields，会显式拒绝旧字段，而非静默忽略。HTTP 客户端无需迁移请求格式。

### 保留的真实能力筛选
- 图片、题型、题数、缓存/safety 参数以及现存 usage 管理要求继续检查。
- 真实 input_image 在任何编码前独立排除不支持它的候选，覆盖共享输入和题目图片；声明 images:true 不能扩大 adapter 固有能力。
- JSON 数据中的 role/type/image_url 是普通键，不创建消息或图片；不把图片 URL 藏在文本里冒充视觉分析。
- 原始 schema 仍只允许四种合法角色，不添加自定义角色或 Decisions 不支持的消息 name 字段；工具历史仍是已记录的 unsupported 边界。

## 固定外部请求与核心答案
- 客户端始终提交 OpenAI model/input/questions，与公开 alias 实际路由到哪家上游无关。
- state、criteria、命名 questions、noul 以及编码包封装都只是服务端对上游使用的内部表示，不是额外客户端 API。
- typed choice 的内部索引键不会取代公开值：true/"true"、false/"false"、内部键同名字符串及空字符串均准确还原。false 不作 missing/default。
- 已有内部 JSON 文本编码继续保留角色标签/顺序/phase、字符串/数组/空内容、Unicode、嵌套 JSON 和题内 JSON。JSON 语义保留，不承诺原始字节空白及对象键顺序。
- 角色标签不等于目标上游原生执行 OpenAI 指令层级；结构边界也不是 prompt injection 安全隔离。这是后端语义差异，管理员应选择满足需求的后端，而非要求客户端更换格式。
- OpenAI 原生分支保持原始 input/questions 结构；概率、confidence、评分期望、拒答及重试规则未放宽。

## Usage 例外（本轮不扩大改动）
- usage_policy:compatible/strict 仍只在服务端 config.yaml 中管理，不是客户端请求参数。
- compatible：上游真实已知 input/output、按标记口径合计及已有细分保留；未知省略而非 0。
- usage.decora 是现存扩展，标记 provider/input_tokens_basis/total_tokens_basis/details_complete，以及可选 provider_cost。
- TypeSafe 使用 billable_input/billable_input_plus_output，不是 OpenAI 精确总量；缓存计数不套用不同输入口径的包含关系。OpenRouter 标记 provider_reported/provider_input_plus_output。
- strict 在请求前排除无法满足完整 OpenAI usage 的上游；OpenAI 原生响应本来保证完整 usage，仍按其官方合同严格校验。
- README 与 PROTOCOL 已在开头明确：请求和核心 answers 固定，整个响应仍有上述 usage schema 偏差。未来外部合同规范化应单独决策，不能通过编造零值完成。

## 验证结果（2026-11-04）
串行命令使用 &&，最终退出码 0。

| 检查 | 实际结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过 |
| cargo test --locked | 1 项单测 + 33 项离线集成测试，共 34 项通过、0 失败；集成测试 0.85 秒 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告 |
| cargo audit --file Cargo.lock | 182 个锁定依赖，1403 条公告库，无漏洞或维护警告 |

新增/替换回归重点：
- 相同 alias、相同含角色/JSON/mixed typed choice 的 OpenAI 请求分别路由三家上游，公开 model/answers 完全相同；各上游内部传输不同。
- 同一回归显式检查 usage 差异仍保留，没有把“核心答案一致”升级成“完整响应严格一致”。
- 客户端上游 state/命名 questions 格式以及转换控制字段均 400，且不发出上游调用。
- 已移除的服务端输入策略/能力字段不能继续配置；合法结构不再有格式拒绝开关。
- 图片独立筛选、OpenAI 原结构、typed false/碰撞、全部既有鉴权/错误/429/超时/概率/usage 测试继续通过。

第一次中途复验有一项配置测试失败：示例配置尚留已移除字段。清理示例后全量复验通过；不是跳过测试。未新增依赖。

## 文件与安全交付
- src/config.rs：删除输入转换策略及三项格式能力开关。
- src/adapter.rs：统一自动转换，删除结构化输入拒绝路径，保留真实能力边界。
- src/gateway.rs：调用统一内部 native_body，不再传入转换策略。
- tests/offline.rs：固定外部合同、非法上游形状与清理后的配置回归。
- README.md、docs/PROTOCOL.md、config.example.yaml：外部合同、内部实现边界及 usage 例外。
- 分支 feat/decisions-gateway，本机身份 Yan；新提交见 git log -1。提交前扫描完整暂存快照，config.yaml/.env/日志/target/reference 仍被忽略。

## 后续仍需
1. 主会话推送本轮本地提交；旧服务端配置删除被移除字段。客户端仍使用固定 OpenAI 请求。
2. 用户授权后做最小真实账户调用，核对模型权限、响应与账单；本次只有 loopback mock，不证明实际账户可用性或概率校准。
3. 公网仍需 TLS 反代、防火墙、限流与进程守护。usage 规范化的下一步需主会话/用户明确合同，当前已公开标注偏差。
