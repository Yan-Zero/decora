# Decora 产品概述

Decora 是 Rust 实现的独立 **OpenAI Decisions 协议网关**。所有公开 model alias 统一提供 `POST /v1/decisions`（另有 `GET /healthz`），请求固定为 OpenAI 形状的 `model / input / questions`，答案为 predicate、choice、score、refusal 四类核心答案。

## 核心原则（修改前必须理解）

1. **外部合同固定**：客户端永远只发 OpenAI Decisions 请求，不选择上游协议、不提交 Jev/OpenRouter 的 `state/criteria/noul` 格式、不提供任何“转换策略”开关。发送上游专有字段或转换控制字段一律 400。
2. **转换是网关内部职责**：OpenAI、Gemini、OpenRouter、TypeSafe/Jev 之间的格式转换由 `adapter` 自动完成；不调用聊天接口、不伪造概率。
3. **不伪造数据**：未知 usage 明细省略而不是填 0；不重归一化、裁剪或重算上游概率；不合成缺失的 confidence；Gemini/OpenRouter/Jev 的未知答案类型报协议错误，不映射成否定答案。
4. **已知合同例外（必须诚实标注）**：默认 `usage_policy: compatible` 可省略 OpenAI 必需的 usage 明细并增加 `usage.decora` 扩展，**不是严格完整的 OpenAI schema**。不得在文档或代码注释中宣称“响应完全一致”。`strict` 是服务端选项，不是客户端参数。
5. **公开 `model` 返回请求 alias**，不暴露内部后端 ID、URL 或凭据。
6. **能力筛选仍然真实**：图片、题型、题数、缓存、safety 参数、usage 要求会在发起网络调用前筛选候选；图片不会被编码成文本冒充已分析。

## 范围边界

- 无数据库、UI、多租户、计费、持久化；无入站 TLS（需外部反向代理）。
- 工具调用历史明确 `unsupported_tool_history`；不实现 `box` 题型；不支持 `file_id` 图片。
- 离线验证只用 loopback mock，没有花费过真实 API 额度。不要声称真实账户可用性或概率已校准。

## 语言约定

README、docs、注释、提交信息、错误说明均使用**中文**；代码标识符使用英文。回复用户时也使用中文。
