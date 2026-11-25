//! YAML configuration stores only environment variable names; actual credentials are resolved at startup and are not included in Debug output.
#[cfg(feature = "server")]
use reqwest::Url;
use serde::Deserialize;
#[cfg(feature = "server")]
use std::{
    collections::{BTreeMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::Path,
};

#[cfg(feature = "server")]
fn listen() -> SocketAddr {
    "127.0.0.1:8787".parse().unwrap()
}
#[cfg(feature = "server")]
fn body() -> usize {
    8 * 1024 * 1024
}
#[cfg(feature = "server")]
fn images() -> usize {
    32
}
#[cfg(feature = "server")]
fn timeout() -> u64 {
    30_000
}
#[cfg(feature = "server")]
fn deadline() -> u64 {
    60_000
}
#[cfg(feature = "server")]
fn attempts() -> usize {
    3
}
#[cfg(feature = "server")]
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

/// Usage compatibility policy; compatible mode does not fill in unknown details, while strict mode requires complete OpenAI semantics.
#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsagePolicy {
    /// Default: preserve known counts and record their source and counting basis in native usage.decora.
    #[default]
    Compatible,
    /// Only candidates with a complete, exact OpenAI usage contract are eligible.
    Strict,
}

/// Native upstream protocols; chat completions compatibility mode is not included.
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Openai,
    Openrouter,
    Typesafe,
    Gemini,
}
impl BackendKind {
    /// Fixed official endpoint; an administrator may replace it with a trusted endpoint.
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1/decisions",
            Self::Openrouter => "https://openrouter.ai/api/alpha/decisions",
            Self::Typesafe => "https://api.typesafe.ai/v1/systemone",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/decisions",
        }
    }
}
/// Upstream model capabilities declared by the administrator; these cannot exceed the adapter's built-in capabilities.
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
/// Each candidate is bound to one model, endpoint, and credential environment variable.
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
/// Decora single-tenant configuration. Non-loopback listening requires explicit authorization and inbound authentication.
#[cfg(feature = "server")]
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
#[cfg(feature = "server")]
impl Config {
    /// Read YAML; errors do not include file contents or possible credentials.
    pub fn load(path: &Path) -> Result<Self, String> {
        let content =
            std::fs::read_to_string(path).map_err(|_| "cannot read the config file".to_owned())?;
        Self::from_yaml(&content)
    }
    /// Parse and validate YAML (supports api_key_env/bearer_key_env; inline keys are forbidden).
    pub fn from_yaml(content: &str) -> Result<Self, String> {
        let cfg: Self = serde_saphyr::from_str(content)
            .map_err(|_| "config YAML has invalid fields or types".to_owned())?;
        cfg.validate()?;
        Ok(cfg)
    }
    /// Reject unbounded retries, plaintext remote credentials, unauthenticated public listeners, and invalid candidates.
    pub fn validate(&self) -> Result<(), String> {
        let bad = || "config has an invalid range, capability or route".to_owned();
        if !self.listen.ip().is_loopback()
            && (!self.allow_public_bind || self.bearer_key_env.is_none())
        {
            return Err("non-loopback listen requires allow_public_bind and bearer_key_env".into());
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
                return Err("endpoints must be HTTPS without userinfo, query or fragment; HTTP is only allowed for explicitly enabled loopback IPs".into());
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
#[cfg(feature = "server")]
fn check_env_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    {
        return Err("invalid credential environment variable name".into());
    }
    Ok(())
}
/// Resolve credentials; the caller supplies a lookup function so offline tests do not modify the process environment.
#[cfg(feature = "server")]
pub fn resolve_secret(
    name: &str,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let value = lookup(name).ok_or_else(|| "missing credential environment variable".to_owned())?;
    if value.is_empty() || value.len() > 8192 || value.bytes().any(|b| b <= 32 || b >= 127) {
        return Err("credentials must be non-empty printable ASCII without whitespace".into());
    }
    Ok(value)
}
