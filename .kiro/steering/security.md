# 安全与隐私约束

这些是项目的硬约束，修改代码时不得削弱；确需调整须先向用户说明风险。

- **日志**：只记录固定消息、尝试次数、固定错误码。不得记录输入、问题、响应正文、密钥、上游 URL、alias、YAML 内容。`main.rs` 固定 `decora=info`，不接受 `RUST_LOG`，避免 reqwest/hyper debug 泄露。
- **错误体**：统一 `{"error":{"message","type","param":null,"code"}}`，不回显上游错误正文。
- **凭据**：后端与入站 key 仅经环境变量；启动时全部解析，缺失/空白/含换行即拒绝启动，且不输出其内容。入站 Bearer 用 `subtle` 常量时间比较。请求上游只带后端凭据，绝不转发调用者的 Bearer 或其他 header。
- **出站 HTTP**：共用 reqwest 连接池；开启证书校验；**禁用重定向跟随与环境代理自动发现**；endpoint 默认官方 HTTPS，禁止 URL 中的用户名/密码/查询/片段；HTTP 仅允许显式启用的 loopback **IP**（测试用）。客户端不能指定上游 URL、backend model、provider 参数。
- **监听**：默认 loopback；公网监听须同时 `allow_public_bind: true` 和 `bearer_key_env`；无入站 TLS，依赖外部反代。
- **资源限制**：入站/响应 body 大小限制、图片数量限制、`max_in_flight` 满时立即 503、attempt/request 超时。
- **不重放风险**：超时重放可能导致上游重复计费；Decisions 无已验证的幂等凭据，需要杜绝重复时 `max_attempts: 1`。
- **测试**：只访问随机端口 loopback mock，绝不读取真实 key、不访问付费 API。任何真实调用必须由用户明确授权。
- **读写文件**：不要读取或回显 `config.yaml`、`.env*` 内容；引用时只说变量名。不要提交这些文件。
