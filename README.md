# Decora

**Decora 是一个 Rust 实现的独立 Decisions 协议网关。** 对外提供真正的 `POST /v1/decisions`：接收 `model / input / questions`，返回原生 `predicate / choice / score / refusal`，校验概率分布及完整 token usage。它不调用聊天接口，不生成或伪造“校准概率”。

当前可运行的是 **OpenAI 原生 Decisions adapter**。OpenRouter 与 TypeSafe/Jev 的官方协议和文本题型转换已经实现并用离线样例验证，但其官方 usage 缺少 OpenAI 强制字段，因此在严格接口下**提前返回 `unsupported_usage`，不发送上游请求**。这两者不是本版可生产使用的后端。详见[协议与能力矩阵](docs/PROTOCOL.md)。

## 快速开始

需要 Rust 2024 工具链；本地验证版本为 `rustc/cargo 1.95.0`。项目不需要数据库、UI 或其他服务。

```bash
cp config.example.yaml config.yaml
# 用安全方式在当前进程/服务管理器中设置 OPENAI_API_KEY 和 DECORA_BEARER_KEY。
# 示例文件只保存变量名称，不支持把 api_key 明文写入 YAML。
cargo run --locked --release -- config.yaml
```

默认监听 `127.0.0.1:8787`。`config.yaml`、`.env*`、日志和构建产物已经加入 `.gitignore`；程序**不自动加载 `.env`**。例如在 Git Bash 中交互设置凭据，避免把密钥写进 shell 历史。

```bash
read -r -s -p 'OpenAI API key: ' OPENAI_API_KEY; printf '\n'
export OPENAI_API_KEY
read -r -s -p 'Decora 入站 key: ' DECORA_BEARER_KEY; printf '\n'
export DECORA_BEARER_KEY
cargo run --locked --release -- config.yaml
```

`DECORA_BEARER_KEY` 应使用单独生成的高熵随机值，不要复用上游 API key。配置中的所有后端凭据都会在启动时解析，缺失/空白/换行等异常会阻止启动，程序不会输出其内容。

```bash
curl http://127.0.0.1:8787/healthz
# {"status":"ok"}；这只代表进程存活，不会查询上游或透露配置。

curl http://127.0.0.1:8787/v1/decisions \
  -H "Authorization: Bearer $DECORA_BEARER_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "decora-luna",
    "input": "包裹外壳破裂，希望尽快换货。",
    "questions": [
      {"type":"predicate","name":"damaged","instructions":"包裹是否损坏？"},
      {"type":"choice","name":"team","instructions":"应分给哪个团队？",
       "choices":[{"value":"售后"},{"value":"销售"}]},
      {"type":"score","name":"urgency","instructions":"按等级判断处理紧迫性。",
       "levels":[{"label":"低"},{"label":"高","description":"需要今天处理"}]}
    ]
  }'
```

评分以等级**位置 `0..N-1`**计分；不是根据 label 的数值计分。响应的 `confidence` 与最大类别概率是两个独立字段。概率是上游模型的估计，Decora 不额外证明其在业务数据上已经校准。

### 多模态

文字、图片和 JSON 内容都保留原始结构；不会把角色消息转换成普通文本。

```bash
curl http://127.0.0.1:8787/v1/decisions \
  -H "Authorization: Bearer $DECORA_BEARER_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model":"decora-luna",
    "input":[{"role":"user","content":[
      {"type":"input_text","text":"这是共享参考图。"},
      {"type":"input_image","image_url":"https://example.com/reference.png"},
      {"type":"input_json","data":{"批次":"A"}}
    ]}],
    "questions":[{"type":"predicate","name":"same","instructions":[
      {"type":"input_text","text":"候选图是否为参考图的变体？"},
      {"type":"input_image","image_url":"https://example.com/candidate.png"}
    ]}]
  }'
```

示例图片地址是占位符，调用前替换为实际可公开访问的 HTTP(S) URL 或 `data:image/png;base64,...`。程序不下载图片，不接受 `file_id`。`input` 与**所有题目的 instructions** 中的图片合计受限：默认 **32 张、整个 JSON body 8 MiB**。可配置上限 1500 张、body 64 MiB；这比官方通用图片上限 512 MB 更保守。格式/分辨率/动画/patch/context 等模型内容限制由上游检查，收到 4xx 后不会换模型重试。

## 路由配置

完整字段见 [`config.example.yaml`](config.example.yaml)。`models` 是公开 alias 到后端 ID 列表的映射；每个后端固定绑定 `kind/model/endpoint/api_key_env/capabilities/priority/weight`。

```yaml
# 在示例的 backends 下增加另一个有相同能力的可信 OpenAI 候选
# secondary:
#   kind: openai
#   model: gpt-6-luna
#   api_key_env: SECONDARY_OPENAI_API_KEY
#   priority: 10
#   weight: 1
#   capabilities: ...  # 显式声明实际支持的能力
models:
  decora-luna: [openai-primary] # 增加候选后可写 [openai-primary, secondary]
```

1. 先根据请求实际需要的题型、图片、消息、JSON、缓存、安全标识及题数过滤候选。
2. `priority` 越小越优先；同一优先级按 `weight` **加权无放回**排列，低优先候选排在后面。
3. 每个请求最多 `max_attempts` 次**总调用**，包括首次；候选全部尝试完后可从该次排列首位开始下一轮。没有运行时模型发现或后台探测。
4. 仅连接失败、发送阶段超时、HTTP `429/500/502/503/504` 允许按候选顺序回退或重试；等待带有界指数退避及抖动。
5. `429 Retry-After` 支持整数秒和 HTTP 日期，在尝试任何下一个候选之前等待。若等待将超过剩余总预算，则立即返回 429 及剩余建议等待秒数，不缩短上游要求的等待。
6. 客户端错误、上游鉴权错误、拒答、非法概率、usage 缺项、重定向、成功头之后的响应体传输失败都**不回退**。拒答以 HTTP 200 的 `type: refusal` 保留；同批其他答案照常返回。

`attempt_timeout_ms` 覆盖一次上游发送及响应体读取；`request_timeout_ms` 覆盖入站 body 读取、所有尝试和等待。默认分别为 30 秒和 60 秒。响应体也受单独大小限制，并发满时立即返回 503。超时重放可能导致上游重复计费；Decisions 没有在本项目中验证过的幂等凭据合同，需要绝对避免重复调用时将 `max_attempts: 1`。

## 安全默认值

- 默认 loopback；若要监听公网，必须同时设置 `allow_public_bind: true` 和 `bearer_key_env`，并在外部反向代理终止 TLS、做防火墙和客户端流量限制。没有入站 TLS 服务端功能。
- 可以删除 `bearer_key_env` 使用仅本地无鉴权模式；不建议在不受信任的多用户机器上这样做。
- 客户端只能使用配置中的 alias，不能指定任意上游 URL、任意 backend model、provider 参数或转发 header。
- 端点默认是官方 HTTPS；管理员可以配置其他可信 HTTPS 入口。禁止 URL 内用户名、密码、查询和片段。测试用 HTTP 只允许显式启用的 loopback **IP** 地址。
- 共用 reqwest 连接池，启用正常证书校验，**禁用重定向和环境 HTTP 代理自动发现**；不会把密钥跨域转发。请求只发送后端凭据，不转发调用者的 Bearer key。
- 日志只记录固定消息、尝试次数和固定错误码；不记录输入、问题、响应正文、密钥、上游 URL、alias 或 YAML 内容。启动器固定启用 `decora=info`，不会接受 `RUST_LOG` 开启 HTTP 客户端的敏感 debug 日志。
- 对外错误统一采用 `{"error":{"message":...,"type":...,"param":null,"code":...}}`，不回显上游错误正文。

常见错误码：`invalid_request`、`unsupported_content`、`unsupported_tool_history`、`image_limit_exceeded`、`model_not_found`、`unsupported_capability`、`unsupported_usage`、`invalid_api_key`、`rate_limit_exceeded`、`upstream_protocol_error`、`upstream_client_error`、`upstream_timeout`、`gateway_busy`。

## 离线验证

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

2026-11-04 实际结果：**19 项测试通过**（1 项单测 + 18 项集成测试），以上四项检查均通过；额外 `cargo audit` 对 182 个锁定依赖的检查也无漏洞或维护警告。详细过程见 `HANDOFF.md`。

测试只访问随机端口的 loopback mock，不读取真实 key、不访问付费 API。包含真实入站 TCP 请求、OpenAI 三题型往返、OpenRouter/TypeSafe 原生转换、概率/usage 校验、角色与图片、重复名称、鉴权、路由与加权排序、兼容候选回退、重定向拦截、429、慢速上传后的共享重试预算、超时、并发及双向 body 限制。

## 上线前需要完成

1. 配置实际账户有权使用的原生 Decisions 模型及高熵入站 key，将真实配置/环境文件权限限制给服务账户。
2. 用户自行授权一个最小真实请求，核对账户权限、实际响应和账单；本项目只做了离线验证，没有花费 API 额度。
3. 如需公网服务，配置 TLS 反代、访问控制、进程守护和外层流量限制；本版不做多租户、计费、持久化或数据库。依赖锁定在 `Cargo.lock`，上线前检查最新安全公告。

### 重要边界

- **OpenRouter/Jev 暂不可用**：完整 usage 缺项会导致严格协议失真，因而前置拒绝；官方将来补齐后，需要重新核实 schema 并补测试才能启用。
- **能力与实际质量不同**：离线测试证明结构转换和防错行为，不代表真实账户可访问所有模型，也不证明模型概率在你的数据上校准。
- **严格、保守的协议子集**：工具调用历史明确报 unsupported；未知字段、超限内容或上游非法响应被拒绝而不是静默丢弃。公开返回 `model` 使用请求 alias，避免暴露路由内部名称。

## 文件

- `src/config.rs`：配置和凭据解析；`src/protocol.rs`：请求/响应合同。
- `src/adapter.rs`：能力边界和原生协议映射；`src/gateway.rs`：HTTP/路由/重试。
- `tests/offline.rs`：离线集成测试；`docs/PROTOCOL.md`：官方依据及能力矩阵。
- `HANDOFF.md`：开发过程、实际检查结果与接续信息。

## Sources

- [OpenAI Decisions 指南](https://developers.openai.com/api/docs/guides/decisions)
- [OpenAI Decisions 创建接口](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- [OpenRouter Alpha Decisions](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-questions-and-answers-request)
- [TypeSafe 官方 API](https://docs.typesafe.ai/api)
