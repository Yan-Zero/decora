# Decora

**Decora 是一个 Rust 实现的独立 Decisions 协议网关。所有公开 model alias 一律使用同一个 `POST /v1/decisions`：固定 OpenAI 形状的 `model / input / questions` 请求与 predicate、choice、score、refusal 核心答案。** 客户端不提交 Jev/OpenRouter 格式、不选择转换策略，也不使用上游的 state/criteria 参数。OpenAI、OpenRouter、TypeSafe/Jev 之间的格式转换由网关自动完成，不调用聊天接口，不伪造概率。

文字、角色消息、JSON parts 和布尔/字符串混合 choice 均自动适配；图片等真实能力仍先筛选。**尚存的合同例外是 usage**：默认服务端 `usage_policy: compatible` 保留真实已知统计、允许省略未知明细，并添加 `usage.decora` 标记，**并非严格完全符合 OpenAI usage schema**。TypeSafe billable input 不能冒充精确总输入，未知值也不填 0。管理员可在服务端选择 usage strict；该选项不属于客户端请求。详见[协议与能力矩阵](docs/PROTOCOL.md)。

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

### Usage 管理选项与当前合同例外

以下是**服务端 config.yaml 管理选项**，不是请求参数。本轮没有把兼容 usage 规范化为严格 OpenAI schema：省略必需明细和增加 `usage.decora` 都是已知差异，不能据此声称整个响应完全一致。

- `compatible`（默认）：三类上游都可参与路由。原生 OpenAI 仍严格校验它本来就提供的完整 usage；OpenRouter/Jev 保留已知计数和存在的明细，未知字段省略。
- `strict`：要求完整精确的 OpenAI usage，OpenRouter/Jev 在发请求前返回 `unsupported_usage` 或被其他兼容候选替代。

例如 TypeSafe 响应可包含如下 usage（合同示例，不是真实调用记录）。这里的 132 **只是计费输入与输出之和**，不是经验证的 OpenAI 总 token 量。

```json
{
  "input_tokens": 120,
  "output_tokens": 12,
  "total_tokens": 132,
  "decora": {
    "provider": "typesafe",
    "input_tokens_basis": "billable_input",
    "total_tokens_basis": "billable_input_plus_output",
    "details_complete": false
  }
}
```

OpenRouter 使用 `provider_reported / provider_input_plus_output` 标记；若上游提供 `cached_tokens`，则保留该明细而不补 `cache_write_tokens`。上游 cost 存在时保留为 `usage.decora.provider_cost`。需要精确 OpenAI usage schema 的部署可由管理员配置 strict；客户端不应将省略值解释为 0。

### 网关内部的角色、JSON 与有类型选项转换

**客户端始终发送标准 Decisions 请求；下面仅说明内部实现，不是额外 API、请求选项或调用前置要求。** 对合法且已可映射的角色、JSON 和 typed choice，不会再因为需要内部编码而拒绝。

- OpenAI adapter 保留原始 input/questions 结构。
- OpenRouter/Jev 的字符串 input 原样转换；消息数组自动编码为有边界的 JSON 文本，放在官方允许的内部 state 中，保留四种合法角色、顺序、phase、空内容、Unicode 和嵌套 JSON。
- instructions 字符串原样转换；text/JSON 内容数组自动编码，不要求客户端扁平化数据或选择协议。
- 不提供输入转换策略或角色/JSON/typed choice 能力开关；图片、题型、题数、缓存与安全标识等真实能力仍独立检查。

内部消息编码如下；**整个对象序列化成一个字符串**供上游使用，客户端无需生成它。题目内容内部使用 `kind: question_content`。

```json
{"decora_encoding":"openai_decisions_text_v1","kind":"messages","data":[{"role":"system","content":"系统标签"},{"role":"user","content":[{"type":"input_json","data":{"ok":false,"nested":[1,"中文"]}}]}]}
```

这保留了角色标签和数据，**不等于上游原生执行 OpenAI 的角色指令优先级**。JSON 数据边界也不是 prompt injection 隔离：模型仍可能受内容中的恶意指令影响。这是后端语义差异，不是客户端可切换的输入格式；要求原生层级语义的部署应由管理员配置合适的上游。

布尔 choice 与字符串可混用，包括 `true` / `"true"`、`false` / `"false"`。只要一组选项含布尔值，该组**所有选项**统一改用内部 `decora_choice_0...` 键，criteria 中保留原始 JSON 类型的 value 和 description。返回的 choice 与各概率 value 按映射还原；布尔 false 不作为缺省值。即使客户端字符串本身等于某个内部键，也不会碰撞；纯字符串组选项继续使用原字符串键。

### 图片与其他多模态

OpenAI adapter 保留原结构并支持图片。OpenRouter/Jev 本版仍对真实 `input_image` 独立筛选，**不会将图片 URL 编入文本后声称分析过图片**；可选择兼容 OpenAI 候选。JSON 数据中的同名 `image_url/type/role` 键只是普通数据。缓存/safety 参数仍不做 native 降级，不支持时提前过滤。

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

下面是**只使用 TypeSafe/Jev 的完整最小配置**，不需要 OpenAI key。设置 `TYPESAFE_API_KEY` 和 `DECORA_BEARER_KEY`，将前面 curl 的 model 改为 `decora-jev` 即可。模型名称仍需实际账户有权使用。

```yaml
listen: 127.0.0.1:8787
bearer_key_env: DECORA_BEARER_KEY
usage_policy: compatible
backends:
  jev-official:
    kind: typesafe
    model: jev-latest
    api_key_env: TYPESAFE_API_KEY
    priority: 0
    weight: 1
    capabilities:
      question_types: [predicate, choice, score]
      images: false
      max_questions: 400
models:
  decora-jev: [jev-official]
```

下面仍是同一种 OpenAI Decisions 请求，发送角色消息、题内 JSON 和混合 typed choice；`decora-jev` 只是示例公开 alias，不是 Jev 格式入口。管理员换上游时可以沿用原 alias，客户端请求无需改写：

```bash
curl http://127.0.0.1:8787/v1/decisions \
  -H "Authorization: Bearer $DECORA_BEARER_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model":"decora-jev",
    "input":[{"role":"user","content":[{"type":"input_json","data":{"flag":false,"text":"true"}}]}],
    "questions":[{"type":"choice","instructions":[
      {"type":"input_text","text":"按 flag 的实际类型和值选择。"},
      {"type":"input_json","data":{"field":"flag"}}
    ],"choices":[{"value":true},{"value":"true"},{"value":false},{"value":"false"}]}]
  }'
```

OpenRouter 的等价配置使用 `kind: openrouter`、账户可用的模型 ID（官方示例为 `typesafe/jev-1.13`）和 `api_key_env: OPENROUTER_API_KEY`。默认入口分别为 `https://api.typesafe.ai/v1/systemone`、`https://openrouter.ai/api/alpha/decisions`；无需配置 endpoint。也可把多个后端 ID 放在同一 alias 中实现跨协议回退。

1. 根据题型、图片、缓存、安全标识、题数及服务端 usage 要求筛选候选；合法角色消息、JSON parts 和 typed choice 自动适配，不因内部转换格式过滤。
2. `priority` 越小越优先；同一优先级按 `weight` **加权无放回**排列，低优先候选排在后面。
3. 每个请求最多 `max_attempts` 次**总调用**，包括首次；候选全部尝试完后可从该次排列首位开始下一轮。没有运行时模型发现或后台探测。
4. 仅连接失败、发送阶段超时、HTTP `429/500/502/503/504` 允许按候选顺序回退或重试；等待带有界指数退避及抖动。
5. `429 Retry-After` 支持整数秒和 HTTP 日期，在尝试任何下一个候选之前等待。若等待将超过剩余总预算，则立即返回 429 及剩余建议等待秒数，不缩短上游要求的等待。
6. 客户端错误、上游鉴权错误、拒答、非法概率、必需 usage 计数缺失/非法、重定向、成功头之后的响应体传输失败都**不回退**。OpenAI 拒答以 HTTP 200 的 `type: refusal` 保留；同批其他答案照常返回。OpenRouter/Jev 官方未定义逐题拒答答案；遇到未定义类型返回协议错误，不伪造否定答案、不盲目切换模型。

`attempt_timeout_ms` 覆盖一次上游发送及响应体读取；`request_timeout_ms` 覆盖入站 body 读取、所有尝试和等待。默认分别为 30 秒和 60 秒。响应体也受单独大小限制，并发满时立即返回 503。超时重放可能导致上游重复计费；Decisions 没有在本项目中验证过的幂等凭据合同，需要绝对避免重复调用时将 `max_attempts: 1`。

## 安全默认值

- 默认 loopback；若要监听公网，必须同时设置 `allow_public_bind: true` 和 `bearer_key_env`，并在外部反向代理终止 TLS、做防火墙和客户端流量限制。没有入站 TLS 服务端功能。
- 可以删除 `bearer_key_env` 使用仅本地无鉴权模式；不建议在不受信任的多用户机器上这样做。
- 客户端只能使用配置中的 alias，不能指定任意上游 URL、任意 backend model、provider 参数或转发 header。
- 端点默认是官方 HTTPS；管理员可以配置其他可信 HTTPS 入口。禁止 URL 内用户名、密码、查询和片段。测试用 HTTP 只允许显式启用的 loopback **IP** 地址。
- 共用 reqwest 连接池，启用正常证书校验，**禁用重定向和环境 HTTP 代理自动发现**；不会把密钥跨域转发。请求只发送后端凭据，不转发调用者的 Bearer key。
- 日志只记录固定消息、尝试次数和固定错误码；不记录输入、问题、响应正文、密钥、上游 URL、alias 或 YAML 内容。启动器固定启用 `decora=info`，不会接受 `RUST_LOG` 开启 HTTP 客户端的敏感 debug 日志。
- 对外错误统一采用 `{"error":{"message":...,"type":...,"param":null,"code":...}}`，不回显上游错误正文。

常见错误码：`invalid_request`、`unsupported_content`、`unsupported_tool_history`、`image_limit_exceeded`、`model_not_found`、`unsupported_capability`、`unsupported_native_mapping`、`unsupported_native_images`、`unsupported_usage`、`invalid_api_key`、`rate_limit_exceeded`、`upstream_protocol_error`、`upstream_client_error`、`upstream_timeout`、`gateway_busy`。

## 离线验证

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

2026-11-04 实际结果：**34 项测试通过**（1 项单测 + 33 项集成测试），以上四项检查均通过；额外 `cargo audit` 对 182 个锁定依赖的检查也无漏洞或维护警告。详细过程见 `HANDOFF.md`。

测试只访问随机端口的 loopback mock，不读取真实 key、不访问付费 API。包含真实入站 TCP 请求、三类上游的三题型 HTTP 往返、所有后端 alias 使用相同 OpenAI 请求和核心 answers 的回归、上游专有请求形状拒绝、OpenRouter→TypeSafe 实际协议回退、服务端 usage compatible/strict 选项、TypeSafe billable input 标记、未知 usage 明细省略、概率/usage 校验、四种角色与题内 JSON 的文本往返、Unicode/空内容/转义边界、true/false 与同名字符串及内部键碰撞、实际图片独立筛选、重复名称、鉴权、路由与加权排序、兼容候选回退、重定向拦截、429、慢速上传后的共享重试预算、超时、并发及双向 body 限制。

## 上线前需要完成

1. 配置实际账户有权使用的原生 Decisions 模型及高熵入站 key，将真实配置/环境文件权限限制给服务账户。
2. 用户自行授权一个最小真实请求，核对账户权限、实际响应和账单；本项目只做了离线验证，没有花费 API 额度。
3. 如需公网服务，配置 TLS 反代、访问控制、进程守护和外层流量限制；本版不做多租户、计费、持久化或数据库。依赖锁定在 `Cargo.lock`，上线前检查最新安全公告。

### 重要边界

- **Usage 是显式部分兼容**：默认模式保留已知数据并标记来源；TypeSafe 的合计是 billable input 加 output，不是 OpenAI 精确总 token。需要完整同构元数据时使用 strict。
- **能力与实际质量不同**：离线测试证明结构转换和防错行为，不代表真实账户可访问所有模型，也不证明模型概率在你的数据上校准。
- **严格、保守的协议子集**：工具调用历史明确报 unsupported；未知请求字段、超限内容或上游非法响应被拒绝而不是静默丢弃。公开返回 `model` 使用请求 alias，避免暴露路由内部名称。

## 文件

- `src/config.rs`：配置和凭据解析；`src/protocol.rs`：请求/响应合同。
- `src/adapter.rs`：能力边界和原生协议映射；`src/gateway.rs`：HTTP/路由/重试。
- `tests/offline.rs`：离线集成测试；`docs/PROTOCOL.md`：官方依据及能力矩阵。
- `HANDOFF.md`：开发过程、实际检查结果与接续信息。

## 官方资料来源

- [OpenAI Decisions 指南](https://developers.openai.com/api/docs/guides/decisions)
- [OpenAI Decisions 创建接口](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- [OpenRouter Alpha Decisions](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-questions-and-answers-request)
- [TypeSafe 官方 API](https://docs.typesafe.ai/api)
