# 项目结构

```
Cargo.toml / Cargo.lock
config.example.yaml     # 配置示例（不含密钥）；config.yaml 本地使用且不入库
README.md               # 面向使用者的外部合同、配置、安全默认值
HANDOFF.md              # 开发接续记录：决策、验证结果、待办
docs/PROTOCOL.md        # 官方依据、能力矩阵、内部转换与编码说明
reference/              # 本机下载的官方 OpenAPI（被 .gitignore 忽略，不入库）
src/
  main.rs               # 可执行入口：读 config.yaml（或单个路径参数），固定 decora=info 日志
  lib.rs                # 模块声明，导出 Config / Gateway
  config.rs             # 配置与凭据解析、校验
  protocol.rs           # 对外 OpenAI Decisions 请求/响应合同与校验
  adapter.rs            # 能力边界与 OpenAI/Gemini/OpenRouter/TypeSafe 原生协议映射
  gateway.rs            # HTTP 路由、鉴权、候选排序、重试/回退、超时
  error.rs              # 统一对外错误体与错误码
tests/offline.rs        # 离线集成测试（loopback mock 上游，约 37 项）
```

## 职责划分

- 改对外请求/响应形状或校验 → `protocol.rs`（并同步 README / PROTOCOL.md）。
- 改上游映射、能力筛选、usage 处理、typed choice 映射 → `adapter.rs`。
- 改路由、重试、超时、鉴权、并发 → `gateway.rs`。
- 改配置字段 → `config.rs` + `config.example.yaml` + README；删除字段时同步删示例，否则示例配置测试会失败。
- 新增错误码 → `error.rs`，并更新 README 的常见错误码列表。

## 文档同步规则

行为或合同变化必须同时更新 `README.md`、`docs/PROTOCOL.md`，并在 `HANDOFF.md` 追加结论与实际验证结果（写真实命令输出，不写未执行的检查）。三处关于 usage 例外的表述必须保持一致。

## 提交约定

提交信息使用中文，简述变更意图（例：`统一 Decisions 请求与核心答案并内置协议转换`）。只在用户明确要求时提交或推送；提交前确认暂存内容不含 `config.yaml`、`.env`、日志、`target/`、`reference/`，优先按文件名暂存。
