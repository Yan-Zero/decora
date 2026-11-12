# 设计文档：Decora 核心 Decisions 网关与安全基线（`decisions-gateway`）

## 1. 模块划分

- **`src/config.rs`**：定义 `Config`、`Backend`、`BackendKind`、`Capabilities`。使用 `serde_saphyr` 解析 YAML 并开启 `#[serde(deny_unknown_fields)]`。在 `Config::validate` 中强制检查非 loopback 绑定条件、端点 URL 安全约束（HTTPS 或显式开启 `allow_http_loopback` 下的 loopback IP，禁止用户名/密码/query/fragment）及参数范围。
- **`src/protocol.rs`**：定义 `DecisionRequest`、`Question`、`ChoiceOption`、`ChoiceValue`、`ScoreLevel`、`Requirements` 与 `DecisionResponse`。在 `DecisionRequest::parse` 中收集请求所需的 `Requirements`（图片数量、缓存、safety、JSON、消息），在 `DecisionResponse::parse` 中校验概率分布、加权期望与 OpenAI `usage` 算术关系。
- **`src/gateway.rs`**：维护共享 `reqwest::Client`（禁用 redirect 与系统代理）和并发限制 `Semaphore`（`max_in_flight`）。提供 `/v1/decisions` 与 `/healthz` 路由、常量时间 Bearer 鉴权中间件、加权无放回排序 `weighted_order`、带抖动指数退避 `backoff` 与 `Retry-After` 预算控制。
- **`src/error.rs`**：定义 `ApiError`，将所有内部与上游错误映射为固定中文说明与固定错误码，杜绝回显上游响应正文。

## 2. 关键不变量

1. **鉴权先于读取大请求体**：入站 Bearer 校验与并发槽位获取在业务执行前完成；超过 `max_body_bytes` 或 `max_response_bytes` 立即终止。
2. **成功响应头之后的传输中断不重试**：一旦上游返回 2xx 响应头，读取响应体流失败返回 `502 upstream_body_error`，避免在无幂等凭据时静默重复计费。
3. **对外隐藏内部拓扑**：对外响应中的 `model` 始终重写为客户端请求的公开 alias，不暴露后端模型 ID。
