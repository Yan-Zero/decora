# 任务清单：Decora 核心 Decisions 网关与安全基线（`decisions-gateway`）

- [x] 1. 初始化 Rust 2024 项目结构，实现 `src/error.rs` 固定对外错误封装与错误码映射。
- [x] 2. 实现 `src/config.rs` 配置解析、环境变量名校验、凭据解析器 `resolve_secret` 与安全边界验证。
- [x] 3. 实现 `src/protocol.rs` 对外 OpenAI Decisions 请求校验（含角色消息、`input_json` 深度/大小、`input_image` 计数与 data URL 校验）及响应概率/期望/usage 校验。
- [x] 4. 实现 `src/gateway.rs` Axum 路由、常量时间 Bearer 鉴权、并发限制、加权优先级无放回排序、有界重试与 `429 Retry-After` 处理。
- [x] 5. 实现 `src/main.rs` 启动入口（固定 `decora=info` 日志级别与优雅停机）并编写 `tests/offline.rs` 离线 loopback mock 集成测试与文档。
