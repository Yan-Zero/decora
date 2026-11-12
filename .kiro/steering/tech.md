# 技术栈与常用命令

- 语言：Rust，edition 2024；本地验证工具链 `rustc/cargo 1.95.0`。
- 主要依赖：axum 0.8、reqwest 0.12（rustls，**无默认特性**）、serde / serde_json、serde-saphyr（YAML）、tokio、tracing、subtle（常量时间比较入站 key）、rand、httpdate、base64。dev：tower。
- 依赖版本由 `Cargo.lock` 锁定；所有命令使用 `--locked`。新增依赖须精确说明理由（项目刻意保持依赖很少），并提醒用户跑 `cargo audit`。
- Release profile：`lto = "thin"`、`strip = true`。
- Windows + PowerShell 环境：命令分隔用 `;`，不要用 `&&`（Windows PowerShell 5.1 不支持）。

## 命令

```powershell
cargo run --locked --release -- config.yaml   # 启动，默认监听 127.0.0.1:8787
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo audit --file Cargo.lock                 # 需另装 cargo-audit
```

## 完成一项代码改动前的验证清单

按顺序运行并全部通过：`fmt --check` → `check --locked` → `test --locked` → `clippy --locked --all-targets -- -D warnings`。基线：34 项测试（1 单测 + 33 集成测试）。测试若失败要修复根因，不得跳过或删除测试来“通过”。

## 配置与凭据

- 配置为 YAML（见 `config.example.yaml`），`deny_unknown_fields`：未知/已移除字段会显式报错。
- 凭据只通过环境变量名间接引用（`api_key_env`、`bearer_key_env`），**不支持 YAML 明文密钥**，也不自动加载 `.env`。
- `.gitignore` 已忽略 `config.yaml`、`config.*.local.yaml`、`.env*`、`*.log`、`/logs/`、`/target/`、`/reference/`。
