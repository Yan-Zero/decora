//! YAML 配置只保存环境变量名称；真实凭据在启动时解析，不参与 Debug 输出。
use reqwest::Url;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::Path,
};

fn listen() -> SocketAddr {
    "127.0.0.1:8787".parse().unwrap()
}
fn body() -> usize {
    8 * 1024 * 1024
}
fn images() -> usize {
    32
}
fn timeout() -> u64 {
    30_000
}
fn deadline() -> u64 {
    60_000
}
fn attempts() -> usize {
    3
}
fn concurrency() -> usize {
    64
}
fn questions() -> usize {
    400
}
fn one() -> u32 {
    1
}
fn types() -> Vec<String> {
    vec!["predicate".into(), "choice".into(), "score".into()]
}

/// Usage 兼容策略；兼容模式不填充未知明细，严格模式要求完整 OpenAI 口径。
#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsagePolicy {
    /// 默认：保留已知计数并在 native usage.decora 中标明来源和计数口径。
    #[default]
    Compatible,
    /// 只有完整、精确的 OpenAI usage 合同才能进入候选池。
    Strict,
}

/// 上游原生协议；不包含聊天 completions 兼容模式。
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Openai,
    Openrouter,
    Typesafe,
}
impl BackendKind {
    /// 官方固定入口；配置可由管理员替换为可信入口。
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1/decisions",
            Self::Openrouter => "https://openrouter.ai/api/alpha/decisions",
            Self::Typesafe => "https://api.typesafe.ai/v1/systemone",
        }
    }
}
/// 管理员声明的上游模型能力；不能扩大 adapter 固有能力。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default = "types")]
    pub question_types: Vec<String>,
    #[serde(default)]
    pub images: bool,
    #[serde(default)]
    pub prompt_cache: bool,
    #[serde(default)]
    pub safety_identifier: bool,
    #[serde(default = "questions")]
    pub max_questions: usize,
}
/// 一个候选固定对应一个模型、端点和凭据环境变量。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub kind: BackendKind,
    pub model: String,
    pub api_key_env: String,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub priority: u32,
    #[serde(default = "one")]
    pub weight: u32,
    pub capabilities: Capabilities,
}
/// Decora 单租户配置。非 loopback 监听必须显式授权并配置入站鉴权。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "listen")]
    pub listen: SocketAddr,
    #[serde(default)]
    pub allow_public_bind: bool,
    pub bearer_key_env: Option<String>,
    #[serde(default)]
    pub usage_policy: UsagePolicy,
    #[serde(default)]
    pub allow_http_loopback: bool,
    #[serde(default = "body")]
    pub max_body_bytes: usize,
    #[serde(default = "body")]
    pub max_response_bytes: usize,
    #[serde(default = "images")]
    pub max_images: usize,
    #[serde(default = "timeout")]
    pub attempt_timeout_ms: u64,
    #[serde(default = "deadline")]
    pub request_timeout_ms: u64,
    #[serde(default = "attempts")]
    pub max_attempts: usize,
    #[serde(default = "concurrency")]
    pub max_in_flight: usize,
    pub backends: BTreeMap<String, Backend>,
    pub models: BTreeMap<String, Vec<String>>,
}
impl Config {
    /// 读取 YAML；错误不包含文件内容或可能的凭据。
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|_| "无法读取配置文件".to_owned())?;
        Self::from_yaml(&content)
    }
    /// 解析和验证 YAML（支持 api_key_env/bearer_key_env，禁止内联密钥）。
    pub fn from_yaml(content: &str) -> Result<Self, String> {
        let cfg: Self =
            serde_saphyr::from_str(content).map_err(|_| "配置 YAML 字段或类型错误".to_owned())?;
        cfg.validate()?;
        Ok(cfg)
    }
    /// 拒绝无界重试、明文远端凭据、无鉴权公网监听和无效候选。
    pub fn validate(&self) -> Result<(), String> {
        let bad = || "配置范围、能力或路由无效".to_owned();
        if !self.listen.ip().is_loopback()
            && (!self.allow_public_bind || self.bearer_key_env.is_none())
        {
            return Err("非 loopback 监听需要 allow_public_bind 和 bearer_key_env".into());
        }
        if !(1..=16).contains(&self.max_attempts)
            || !(1..=360_000).contains(&self.attempt_timeout_ms)
            || !(1..=600_000).contains(&self.request_timeout_ms)
            || !(1..=1500).contains(&self.max_images)
            || !(1024..=64 * 1024 * 1024).contains(&self.max_body_bytes)
            || !(1024..=64 * 1024 * 1024).contains(&self.max_response_bytes)
            || !(1..=4096).contains(&self.max_in_flight)
            || self.models.is_empty()
            || self.backends.is_empty()
        {
            return Err(bad());
        }
        if let Some(name) = &self.bearer_key_env {
            check_env_name(name)?;
        }
        for b in self.backends.values() {
            check_env_name(&b.api_key_env)?;
            if b.model.is_empty()
                || b.model.len() > 1_048_576
                || b.weight == 0
                || b.weight > 1_000_000
                || !(1..=400).contains(&b.capabilities.max_questions)
                || b.capabilities.question_types.is_empty()
                || b.capabilities
                    .question_types
                    .iter()
                    .any(|t| !["predicate", "choice", "score"].contains(&t.as_str()))
            {
                return Err(bad());
            }
            let url = Url::parse(b.endpoint.as_deref().unwrap_or(b.kind.endpoint()))
                .map_err(|_| bad())?;
            let local = url
                .host_str()
                .and_then(|h| h.trim_matches(['[', ']']).parse::<IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback());
            if !(url.scheme() == "https"
                || (url.scheme() == "http" && local && self.allow_http_loopback))
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("端点必须为 HTTPS 且不含用户信息、查询参数或片段；HTTP 只允许显式启用的 loopback IP".into());
            }
        }
        for (alias, ids) in &self.models {
            let mut seen = HashSet::new();
            if alias.is_empty()
                || alias.len() > 1_048_576
                || ids.is_empty()
                || ids
                    .iter()
                    .any(|id| !self.backends.contains_key(id) || !seen.insert(id))
            {
                return Err(bad());
            }
        }
        Ok(())
    }
}
fn check_env_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    {
        return Err("凭据环境变量名称无效".into());
    }
    Ok(())
}
/// 解析凭据；调用者传入解析器以便离线测试不修改进程环境。
pub fn resolve_secret(
    name: &str,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let value = lookup(name).ok_or_else(|| "缺少凭据环境变量".to_owned())?;
    if value.is_empty() || value.len() > 8192 || value.bytes().any(|b| b <= 32 || b >= 127) {
        return Err("凭据必须是非空可打印 ASCII 且不含空白".into());
    }
    Ok(value)
}
