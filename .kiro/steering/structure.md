# 项目结构

```
Cargo.toml / Cargo.lock
config.example.yaml     # 配置示例（不含密钥）；config.yaml 本地使用且不入库
README.md               # 面向使用者的外部合同、多方言端点、配置、安全默认值
HANDOFF.md              # 开发接续记录：决策、验证结果、待办
docs/PROTOCOL.md        # 官方依据、多前端/多后端能力矩阵、内部转换与编码说明
reference/              # 本机下载的官方 OpenAPI（被 .gitignore 忽略，不入库）
src/
  main.rs               # 可执行入口：读 config.yaml（或单个路径参数），固定 decora=info 日志
  lib.rs                # 模块声明，导出 Config / Gateway
  config.rs             # 配置与凭据解析、校验
  protocol.rs           # 内部标准中间表示（Canonical Decision IR）与 OpenAI 请求/响应校验
  dialect.rs            # 多前端 Decisions 方言编解码（OpenAI / Gemini / TypeSafe / OpenRouter 入站解析与出站响应格式化）
  adapter.rs            # 出站能力边界与 OpenAI / Gemini / OpenRouter / TypeSafe 原生上游协议映射
  gateway.rs            # 多端点 HTTP 路由、Bearer/x-goog-api-key 鉴权、候选排序、重试/回退、超时
  error.rs              # 统一对外错误体与错误码
tests/offline.rs        # 离线集成测试（loopback mock 上游）
```

## 职责划分

- 改内部标准中间表示（Canonical IR）、数学概率校验或 OpenAI 合同 → `protocol.rs`。
- 改多前端入站请求解析或出站响应格式化（`/v1/decisions`、`/v1beta/decisions`、`/v0/decisions`、`/api/v1/decisions`） → `dialect.rs`（并同步 README / PROTOCOL.md）。
- 改出站上游映射、能力筛选、usage 归一化、typed choice / score 映射 → `adapter.rs`。
- 改路由、重试、超时、入站鉴权、并发 → `gateway.rs`。
- 改配置字段 → `config.rs` + `config.example.yaml` + README；删除字段时同步删示例，否则示例配置测试会失败。
- 新增错误码 → `error.rs`，并更新 README 的常见错误码列表。

## 文档同步规则

行为或合同变化必须同时更新 `README.md`、`docs/PROTOCOL.md`，并在 `HANDOFF.md` 追加结论与实际验证结果（写真实命令输出，不写未执行的检查）。三处关于 usage 例外的表述必须保持一致。

## 提交约定

提交信息使用中文，简述变更意图（例：`重构多方言协议架构并支持 Gemini 与 TypeSafe 入站端点`）。只在用户明确要求时提交或推送；提交前确认暂存内容不含 `config.yaml`、`.env`、日志、`target/`、`reference/`，优先按文件名暂存。
