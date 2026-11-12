# 需求文档：Decora 核心 Decisions 网关与安全基线（`decisions-gateway`）

## 背景与目标

构建一个独立的 Rust 单租户 **OpenAI Decisions 协议网关**（`POST /v1/decisions` 与 `GET /healthz`），严格校验请求与响应中的原生概率、置信度、加权评分期望及 token 统计，提供带能力筛选的加权优先级路由与有界故障回退，同时默认满足高安全与零敏感日志泄露要求。

---

## 需求列表

### 需求 1：OpenAI Decisions 请求与响应合同校验
- **验收标准 1.1**：仅公开 `POST /v1/decisions` 与 `GET /healthz`，未知路由返回 `404 not_found`，未知 JSON 字段一律拒绝（`400 invalid_request`）。
- **验收标准 1.2**：校验 `model`、`input`（字符串或 `user / system / developer / assistant` 角色消息数组）和 `questions`（`1..=400` 题，支持 `predicate`、`choice`、`score`）。
- **验收标准 1.3**：`choice` 支持 `2..=255` 个唯一选项（区分布尔 `true` 与字符串 `"true"`）；`score` 支持 `2..=10` 个有序等级并按索引位置 `0..N-1` 计算加权期望。
- **验收标准 1.4**：校验 `input_json.data`（对象或对象数组，深度 `<= 32`、值数 `<= 10,000`、紧凑字节 `<= 256 KiB`）与 `input_image.image_url`（HTTP(S) 或 base64 data URL，禁止 `file_id`，统计共享输入与全部题目中的图片总数 `<= max_images`）。
- **验收标准 1.5**：严格校验上游返回的概率与 `confidence` 在有限 `[0, 1]` 区间内；`choice` 概率和与 `score` 概率和/加权期望的绝对误差容忍度为 `0.0001`，不重归一化或裁剪上游概率。
- **验收标准 1.6**：保留逐题 `type: refusal`（HTTP 200），绝不因单题拒答而切换模型重放。

### 需求 2：能力筛选、加权优先级路由与有界重试
- **验收标准 2.1**：在发起网络请求前，按题型、题数、图片、缓存（`prompt_cache`）与 `safety_identifier` 筛选兼容后端候选。
- **验收标准 2.2**：按 `priority` 升序排列候选，同优先级内按 `weight` 进行加权无放回抽样排序。
- **验收标准 2.3**：整个请求最多执行 `max_attempts` 次总上游调用；仅连接失败、发送阶段超时、HTTP `429/500/502/503/504` 允许回退或重试。
- **验收标准 2.4**：解析 `429 Retry-After`（整数秒或 HTTP 日期），若等待时间超过剩余请求总预算（`request_timeout_ms`），立即向客户端返回 `429 rate_limit_exceeded` 及建议等待秒数，不缩短上游要求的等待。

### 需求 3：安全默认值与离线验证
- **验收标准 3.1**：默认监听 `127.0.0.1:8787`；非 loopback 监听必须同时配置 `allow_public_bind: true` 与 `bearer_key_env`。
- **验收标准 3.2**：所有密钥仅通过环境变量名在启动时解析（禁止 YAML 明文密钥），入站 Bearer 采用 `subtle` 常量时间比较，出站禁用重定向跟随与环境代理自动发现。
- **验收标准 3.3**：日志固定为 `decora=info`，只记录固定消息与错误码，绝不记录输入、问题、响应体、URL、alias 或密钥；对外错误统一返回固定结构 `{"error":{"message","type","param":null,"code"}}`。
- **验收标准 3.4**：提供完全离线的 loopback mock 集成测试，不读取真实 API key、不发生付费网络调用。
