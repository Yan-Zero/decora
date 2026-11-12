# 任务清单：统一外部 Decisions 合同与内置角色/JSON/有类型选项转换（`unified-contract-and-lowering`）

- [x] 1. 从 `src/config.rs` 与 `config.example.yaml` 中删除 `NativeInputPolicy` 及 `messages / json_input / typed_choices` 能力开关。
- [x] 2. 在 `src/adapter.rs` 中实现统一的 `textual` JSON 封包编码与 `choice_key` 混合类型选项无碰撞映射，移除 `unsupported_textual_lowering` 分支。
- [x] 3. 在 `src/adapter.rs` 中确保真实 `input_image` 在编码前独立筛选，且不误判 `input_json.data` 内的同名字段。
- [x] 4. 在 `tests/offline.rs` 中新增三家上游使用完全相同结构化 OpenAI 请求返回相同核心 `answers` 的回归测试、碰撞边界测试及非法上游请求形状 400 测试。
- [x] 5. 更新 `README.md`、`docs/PROTOCOL.md` 与 `HANDOFF.md`，运行 `fmt / check / test / clippy / audit` 全量通过。
