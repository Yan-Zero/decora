# Decora 开发接续记录

## 当前状态（2026-11-04）
**已补齐 OpenRouter/TypeSafe 的角色消息、JSON parts 和布尔 choice 适配。默认 native_input_policy:textual 与 usage_policy:compatible；34 项测试通过。** OpenAI 原生结构保持不变。角色/JSON 采用明确的结构化文本编码，不宣称保留上游原生 OpenAI 指令层级；图片仍独立过滤。

## 本轮要求与 Git 起点
- 主会话转达用户要求，不再将角色/JSON/布尔 choice 一概 unsupported，允许有清晰文档的 textual lowering。
- 开工先检查 git：工作区干净，feat/decisions-gateway 跟踪 origin/main，已有 e14b4b1 与 c3f137c；主会话已经推送这两个提交并配置 SSH remote。
- 本轮只追加本地提交，不推送、不创建远端、不部署、不调用付费 API、不继续派发 agent。
- 本地目录 E:\source\toy\decora；工具链 rustc/cargo 1.95.0，无系统安装。最初目录不存在，上级无额外 CLAUDE.md。

## 三项适配的实际合同

### 角色消息与 JSON parts
- 新增 native_input_policy，默认 textual；strict 可禁用结构化输入的文本降级，与 usage_policy 独立。
- 字符串 input/instructions 原样保留，包括空字符串；消息数组及 instructions 内容数组编码为 JSON 文本，分别放在官方 native state / instructions 字符串字段。
- 封装严格为 {decora_encoding:"openai_decisions_text_v1",kind:"messages"|"question_content",data:原始结构}。JSON 转义保留明确 data 边界，不采用易混淆的裸分隔符拼接。
- 保留 system/developer/user/assistant 标签、顺序、phase、字符串/数组形态、空内容、Unicode、嵌套 JSON。JSON 语义保留，原始字节空白及对象键顺序不作保留承诺。
- 不创造新角色，不擅加 Decisions 不支持的消息 name 字段。自定义角色和 name 仍在原始请求校验时拒绝，已补回归案例。
- capabilities.messages/json_input/typed_choices 默认 true，可逐项 false 禁止该候选承接；strict input 下消息/内容数组提前 unsupported_textual_lowering，可选择其他兼容候选。
- 角色标签不是原生指令层级。JSON 数据边界也不是 prompt injection 隔离；模型仍可能遵从数据中的恶意指令。需要原生层级语义时用 OpenAI 或 input strict。

### 布尔与字符串混合 choice
- 一组只要包含 bool，全部选项统一重命名为 decora_choice_0...N-1。criteria 中保存原始有类型 value 与可选 description；纯字符串组沿用旧字符串键。
- 响应 choice 与 probabilities 按同一索引映射还原准确类型。false 是明确 bool，不作缺省；true/"true"、false/"false"、等于内部键的原始字符串以及空字符串均不碰撞。
- 未知选项键、缺概率键、原生响应擅自返回 bool choice 等都报协议错误，不重试或切换；还原后复用原概率和/选项覆盖/最高概率检查。
- input strict 不禁止这种可逆选项映射；typed_choices:false 可单独过滤布尔选项能力。

### 图片与 OpenAI 原生结构
- 真实 input_image 在任何文本编码前独立排除，覆盖共享输入和题目 instructions；即使配置 native images:true 也不能绕过。
- input_json.data 内普通 role/type/image_url 键仍是数据，不触发取图或创建角色。图片 URL 不会藏入文本后冒充视觉分析。
- 缓存断点/safety 参数不做 native 降级；工具调用历史仍 unsupported_tool_history。
- OpenAI 始终走原始 body 分支；已端到端验证即使 native_input_policy:strict，角色、题内 JSON 与 mixed typed choice 也不被编码或更名。

## 官方协议依据及保留的 usage 策略
- 官方资料、完整 schema 下载来源及 SHA256 见 docs/PROTOCOL.md；原始资料在 gitignore 的 reference/ 内。
- OpenRouter 真正入口 https://openrouter.ai/api/alpha/decisions（operation-level server 覆盖全局 /api/v1）；TypeSafe 入口 https://api.typesafe.ai/v1/systemone。
- 两者官方 state/instructions/criteria 允许字符串/结构化指导，本轮编码只使用这些现有字段，不改成 chat completions、不增加虚构顶层字段。
- usage_policy:compatible 保留真实已知 input/output/合计及已有细分，未知省略而非 0；usage.decora 标记来源及口径。
- TypeSafe input 为 billable_input，total 为 billable_input_plus_output，不是 OpenAI 精确总量；不能套用总输入口径的缓存包含关系。OpenRouter 标记 provider_reported/provider_input_plus_output。
- usage_policy:strict 仍在调用前排除 native；OpenAI 本来保证完整 usage，因此始终严格校验原生合同。概率、confidence、评分期望及拒答真实性没有放宽。

## 实际验证

| 检查 | 结果 |
|---|---|
| cargo fmt --check | 通过，无格式差异 |
| cargo check --locked | 通过 |
| cargo test --locked | 1 项路由单测 + 33 项离线集成测试，共 34 项通过，0 失败 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告 |
| cargo audit --file Cargo.lock | 182 个锁定依赖，1403 条公告库，无漏洞或维护警告 |

全部检查实际运行，命令以 && 串行执行。原生端到端 mock 新增覆盖两家上游的：
- 四种角色、字符串/数组/空内容、Unicode、嵌套 JSON、题内 JSON 与转义数据边界；
- 角色+题内 JSON+布尔 choice 与 predicate/score 同批组合；
- 6 个混合有类型选项逐一作为赢家，含 bool false 与内部键同名字符串；
- 错误内部键不重试、strict 独立开关、逐项禁用能力、真实图片独立筛选；
- 原生 OpenAI 请求结构保持不变；自定义角色/name 字段仍拒绝。

此前 e14b4b1 完成初版，c3f137c 补齐 native HTTP 与 usage 策略；曾通过 19/26 项测试。本轮在其基础上扩展为 34 项。早先 unsafe-libyaml 维护警告已通过改用 serde-saphyr 解决，本轮未新增依赖。

## 文件与交付
- src/config.rs：NativeInputPolicy 与默认/可禁用能力。
- src/adapter.rs：结构化文本编码、typed choice 无碰撞请求/响应映射、固有图片边界。
- src/gateway.rs：按策略筛选/生成 native body；OpenAI 原生分支不变。
- tests/offline.rs：新增往返、组合与回归案例。
- README.md、docs/PROTOCOL.md、config.example.yaml：默认策略、能力矩阵、编码合同、curl 示例与角色层级边界。
- 独立仓库分支 feat/decisions-gateway，本机身份 Yan；新提交见 git log -1。提交前扫描完整暂存快照，真实 config.yaml/.env/日志/target/reference 均不纳入提交。

## 上线前仍需
1. 设置真实账户环境变量和权限受限的配置，用户授权后分别做最小真实调用；本次均是 loopback 离线 mock，不证明实际账户权限或模型概率校准。
2. 根据业务是否要求原生角色层级选择 textual/strict；需要完整 OpenAI usage 时另设 usage strict，不把省略值当 0。
3. 公网使用需 TLS 反代、防火墙、限流和进程守护；重试可能重复计费，需绝对避免时 max_attempts:1。主会话负责本轮推送，本代理不执行。
