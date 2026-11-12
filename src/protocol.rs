//! OpenAI Decisions 请求和响应合同；不修补概率、不猜测 token 明细。
use crate::{config::UsagePolicy, error::ApiError};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 验证后的请求；input/instructions 保持原始多模态内容，不扁平化角色。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequest {
    pub model: String,
    pub input: Value,
    pub questions: Vec<Question>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_options: Option<Value>,
}
/// 三种原生题型。name 可省略，响应中会明确返回 null。
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Question {
    Predicate {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        instructions: Value,
    },
    Choice {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        instructions: Value,
        choices: Vec<ChoiceOption>,
    },
    Score {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        instructions: Value,
        levels: Vec<ScoreLevel>,
    },
}
/// 字符串 "true" 和布尔 true 是不同选项。
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChoiceValue {
    String(String),
    Bool(bool),
}
impl PartialEq for ChoiceValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            _ => false,
        }
    }
}
/// 离散选项的值和可选说明。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: ChoiceValue,
    pub description: Option<String>,
}
/// 从零开始的有序评分等级；label 不是任意数值刻度。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreLevel {
    pub label: String,
    pub description: Option<String>,
}
impl Question {
    /// 题型名称，用于能力匹配。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Predicate { .. } => "predicate",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }
    /// 原始相关名称；不要求唯一，映射 adapter 使用内部索引键。
    pub fn name(&self) -> &Option<String> {
        match self {
            Self::Predicate { name, .. } | Self::Choice { name, .. } | Self::Score { name, .. } => {
                name
            }
        }
    }
    /// 原始文字或多模态题目说明。
    pub fn instructions(&self) -> &Value {
        match self {
            Self::Predicate { instructions, .. }
            | Self::Choice { instructions, .. }
            | Self::Score { instructions, .. } => instructions,
        }
    }
}
/// 请求的实际能力需求，供路由器先过滤再尝试。
#[derive(Default)]
pub struct Requirements {
    pub images: usize,
    pub json: bool,
    pub messages: bool,
    pub cache: bool,
    pub safety: bool,
}
fn keys(value: &Value, allowed: &[&str], required: &[&str]) -> Result<(), ApiError> {
    let obj = value.as_object().ok_or_else(ApiError::invalid)?;
    if obj.keys().any(|key| !allowed.contains(&key.as_str()))
        || required.iter().any(|key| !obj.contains_key(*key))
    {
        return Err(ApiError::invalid());
    }
    Ok(())
}
fn text(value: &Value, max: usize) -> Result<(), ApiError> {
    if value.as_str().is_none_or(|s| s.chars().count() > max) {
        return Err(ApiError::invalid());
    }
    Ok(())
}
fn breakpoint(v: &Value, r: &mut Requirements) -> Result<(), ApiError> {
    if v.is_null() {
        return Ok(());
    }
    keys(v, &["mode"], &["mode"])?;
    if v["mode"] != "explicit" {
        return Err(ApiError::invalid());
    }
    r.cache = true;
    Ok(())
}
fn image_url(v: &Value) -> Result<(), ApiError> {
    let s = v.as_str().ok_or_else(ApiError::invalid)?;
    if let Some(data) = s.strip_prefix("data:") {
        let (mime, data) = data.split_once(";base64,").ok_or_else(ApiError::invalid)?;
        if !["image/png", "image/jpeg", "image/webp", "image/gif"].contains(&mime)
            || data.is_empty()
            || base64::engine::general_purpose::STANDARD
                .decode(data)
                .is_err()
        {
            return Err(ApiError::invalid());
        }
    } else {
        let url = reqwest::Url::parse(s).map_err(|_| ApiError::invalid())?;
        if !["https", "http"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::invalid());
        }
    }
    Ok(())
}
fn validate_json_data(data: &Value) -> Result<(), ApiError> {
    if serde_json::to_vec(data)
        .map_err(|_| ApiError::invalid())?
        .len()
        > 256 * 1024
    {
        return Err(ApiError::invalid());
    }
    let mut stack = vec![(data, 0usize)];
    let mut values = 0;
    while let Some((value, parent_depth)) = stack.pop() {
        values += 1;
        if values > 10_000 {
            return Err(ApiError::invalid());
        }
        let depth = parent_depth + usize::from(value.is_array() || value.is_object());
        if depth > 32 {
            return Err(ApiError::invalid());
        }
        match value {
            Value::Array(a) => stack.extend(a.iter().map(|v| (v, depth))),
            Value::Object(o) => stack.extend(o.values().map(|v| (v, depth))),
            _ => {}
        }
    }
    Ok(())
}
fn parts(v: &Value, mode: &str, cache_allowed: bool, r: &mut Requirements) -> Result<(), ApiError> {
    let list = v.as_array().ok_or_else(ApiError::invalid)?;
    if list.len() > 16384 || (mode == "question" && list.is_empty()) {
        return Err(ApiError::invalid());
    }
    for part in list {
        match part["type"].as_str() {
            Some("input_text") if mode != "assistant" => {
                keys(
                    part,
                    if cache_allowed {
                        &["type", "text", "prompt_cache_breakpoint"]
                    } else {
                        &["type", "text"]
                    },
                    &["type", "text"],
                )?;
                text(&part["text"], 10_485_760)?;
            }
            Some("input_image") if mode == "user" || mode == "question" => {
                keys(
                    part,
                    if cache_allowed {
                        &["type", "image_url", "detail", "prompt_cache_breakpoint"]
                    } else {
                        &["type", "image_url", "detail"]
                    },
                    &["type", "image_url"],
                )?;
                image_url(&part["image_url"])?;
                if let Some(detail) = part.get("detail")
                    && !detail.is_null()
                    && !["auto", "low", "high", "original"].contains(&detail.as_str().unwrap_or(""))
                {
                    return Err(ApiError::invalid());
                }
                r.images += 1;
            }
            Some("input_json") if mode == "user" || mode == "question" => {
                keys(part, &["type", "data"], &["type", "data"])?;
                let d = &part["data"];
                if !d.is_object() && !d.as_array().is_some_and(|a| a.iter().all(Value::is_object)) {
                    return Err(ApiError::invalid());
                }
                validate_json_data(d)?;
                r.json = true;
            }
            Some("output_text") if mode == "assistant" => {
                keys(part, &["type", "text"], &["type", "text"])?;
                text(&part["text"], 10_485_760)?;
            }
            Some("refusal") if mode == "assistant" => {
                keys(part, &["type", "refusal"], &["type", "refusal"])?;
                text(&part["refusal"], 10_485_760)?;
            }
            _ => return Err(ApiError::unsupported("unsupported_content")),
        }
        if let Some(v) = part.get("prompt_cache_breakpoint") {
            breakpoint(v, r)?;
        }
    }
    Ok(())
}
impl DecisionRequest {
    /// 根据官方 schema 校验输入，网关另施加更小的图像/HTTP body 限制。
    pub fn parse(value: Value, max_images: usize) -> Result<(Self, Requirements), ApiError> {
        if let Some(qs) = value["questions"].as_array() {
            for q in qs {
                if q.get("name").is_some_and(Value::is_null) {
                    return Err(ApiError::invalid());
                }
                for field in ["choices", "levels"] {
                    if q[field].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item.get("description").is_some_and(Value::is_null))
                    }) {
                        return Err(ApiError::invalid());
                    }
                }
            }
        }
        let req: Self = serde_json::from_value(value).map_err(|_| ApiError::invalid())?;
        if req.model.is_empty()
            || req.model.chars().count() > 1_048_576
            || !(1..=400).contains(&req.questions.len())
        {
            return Err(ApiError::invalid());
        }
        let mut r = Requirements::default();
        if req.input.is_string() {
            text(&req.input, 10_485_760)?;
        } else if let Some(messages) = req.input.as_array() {
            r.messages = true;
            if messages.len() > 131072 {
                return Err(ApiError::invalid());
            }
            for msg in messages {
                if msg.get("type").is_some_and(|t| t != "message") {
                    return Err(ApiError::unsupported("unsupported_tool_history"));
                }
                let role = msg["role"].as_str().ok_or_else(ApiError::invalid)?;
                if !["user", "system", "developer", "assistant"].contains(&role) {
                    return Err(ApiError::invalid());
                }
                keys(
                    msg,
                    if role == "assistant" {
                        &["type", "role", "content", "phase"]
                    } else {
                        &["type", "role", "content"]
                    },
                    &["role", "content"],
                )?;
                if let Some(p) = msg.get("phase")
                    && !p.is_null()
                    && p != "commentary"
                    && p != "final_answer"
                {
                    return Err(ApiError::invalid());
                }
                if msg["content"].is_string() {
                    text(&msg["content"], 10_485_760)?;
                } else {
                    parts(&msg["content"], role, role != "assistant", &mut r)?;
                }
            }
        } else {
            return Err(ApiError::invalid());
        }
        for q in &req.questions {
            if q.name()
                .as_ref()
                .is_some_and(|s| s.chars().count() > 1_048_576)
            {
                return Err(ApiError::invalid());
            }
            if q.instructions().is_string() {
                text(q.instructions(), 1_048_576)?;
            } else {
                parts(q.instructions(), "question", false, &mut r)?;
            }
            match q {
                Question::Choice { choices, .. } => {
                    if !(2..=255).contains(&choices.len()) {
                        return Err(ApiError::invalid());
                    }
                    for (i, c) in choices.iter().enumerate() {
                        if choices[..i].iter().any(|prev| prev.value == c.value) {
                            return Err(ApiError::invalid());
                        }
                        if let ChoiceValue::String(v) = &c.value
                            && v.chars().count() > 1_048_576
                        {
                            return Err(ApiError::invalid());
                        }
                        if c.description
                            .as_ref()
                            .is_some_and(|s| s.chars().count() > 1_048_576)
                        {
                            return Err(ApiError::invalid());
                        }
                    }
                }
                Question::Score { levels, .. }
                    if !(2..=10).contains(&levels.len())
                        || levels.iter().any(|l| {
                            l.label.chars().count() > 1_048_576
                                || l.description
                                    .as_ref()
                                    .is_some_and(|s| s.chars().count() > 1_048_576)
                        }) =>
                {
                    return Err(ApiError::invalid());
                }
                _ => {}
            }
        }
        if r.images > max_images || r.images > 1500 {
            return Err(ApiError::unsupported("image_limit_exceeded"));
        }
        if let Some(s) = &req.safety_identifier {
            if s.chars().count() > 128 {
                return Err(ApiError::invalid());
            }
            r.safety = true;
        }
        if let Some(s) = &req.prompt_cache_key {
            if s.chars().count() > 64 {
                return Err(ApiError::invalid());
            }
            r.cache = true;
        }
        if let Some(options) = &req.prompt_cache_options {
            keys(options, &["mode", "ttl"], &[])?;
            if let Some(mode) = options.get("mode")
                && !mode.is_null()
                && mode != "implicit"
                && mode != "explicit"
            {
                return Err(ApiError::invalid());
            }
            if let Some(ttl) = options.get("ttl")
                && ttl != "30m"
            {
                return Err(ApiError::invalid());
            }
            r.cache = true;
        }
        Ok((req, r))
    }
    /// 生成 OpenAI 原生请求，只替换管理员指定的 backend model。
    pub fn openai_body(&self, model: &str) -> Value {
        let mut body = serde_json::to_value(self).expect("validated JSON");
        body["model"] = json!(model);
        // 可选 description 不能发送 null。
        for q in body["questions"].as_array_mut().unwrap() {
            for field in ["choices", "levels"] {
                if let Some(items) = q.get_mut(field).and_then(Value::as_array_mut) {
                    for item in items {
                        if item["description"].is_null() {
                            item.as_object_mut().unwrap().remove("description");
                        }
                    }
                }
            }
        }
        body
    }
}
/// 核心答案响应；usage 可按显式策略部分兼容，拒答仍然是成功响应的一项。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionResponse {
    pub model: String,
    pub answers: Vec<Answer>,
    pub usage: Usage,
}
/// 原生答案；保留独立 confidence，不把它强行等同于最大类别概率。
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Predicate {
        name: Option<String>,
        probability: f64,
    },
    Choice {
        name: Option<String>,
        choice: ChoiceValue,
        confidence: f64,
        probabilities: Vec<ChoiceProbability>,
    },
    Score {
        name: Option<String>,
        score: f64,
        confidence: f64,
        probabilities: Vec<ScoreProbability>,
    },
    Refusal {
        name: Option<String>,
    },
}
/// 某个离散选项的原生概率。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceProbability {
    pub value: ChoiceValue,
    pub probability: f64,
}
/// 评分等级的从零开始索引、原始 label 与原生概率。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreProbability {
    pub value: usize,
    pub label: String,
    pub probability: f64,
}
/// Token 统计；兼容模式允许省略未知明细，但从不填零猜测。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputTokensDetails>,
    pub output_tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
    pub total_tokens: i32,
    /// 原生跨协议转换的来源与口径扩展；OpenAI 原生响应不添加此字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decora: Option<UsageMetadata>,
}
/// 已知的缓存明细；缺失值仍为未知，不等于零。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i32>,
}
/// 已知的推理 token 明细。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i32>,
}
/// 经过原生协议适配的提供方，而非内部后端 ID 或 URL。
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageProvider {
    Openrouter,
    Typesafe,
    Gemini,
}
/// input_tokens 的真实计数口径，不将计费输入冒充总输入。
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputTokenBasis {
    ProviderReported,
    BillableInput,
    TotalInputTokens,
}
/// total_tokens 是哪一种输入口径与输出计数之和。
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TotalTokenBasis {
    ProviderInputPlusOutput,
    BillableInputPlusOutput,
    TotalInputPlusOutput,
}
/// 兼容响应扩展：清楚标记来源、输入口径、总量推导及明细完整性。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageMetadata {
    pub provider: UsageProvider,
    pub input_tokens_basis: InputTokenBasis,
    pub total_tokens_basis: TotalTokenBasis,
    pub details_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_by_modality: Option<Vec<Value>>,
}
impl Usage {
    /// 是否具备 OpenAI 所要求的全部明细；不代表不同上游的计数口径相同。
    pub fn details_complete(&self) -> bool {
        self.input_tokens_details
            .as_ref()
            .is_some_and(|d| d.cached_tokens.is_some() && d.cache_write_tokens.is_some())
            && self
                .output_tokens_details
                .as_ref()
                .is_some_and(|d| d.reasoning_tokens.is_some())
    }
    fn validate(&self, policy: UsagePolicy) -> Result<(), ApiError> {
        let cached = self
            .input_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens);
        let written = self
            .input_tokens_details
            .as_ref()
            .and_then(|d| d.cache_write_tokens);
        let reasoning = self
            .output_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens);
        if policy == UsagePolicy::Strict && (!self.details_complete() || self.decora.is_some()) {
            return Err(ApiError::protocol());
        }
        // billable input 未承诺包含全部缓存 token，不能拿它做 OpenAI 口径的缓存上界。
        let comparable_input = self
            .decora
            .as_ref()
            .is_none_or(|m| m.input_tokens_basis != InputTokenBasis::BillableInput);
        if [
            Some(self.input_tokens),
            Some(self.output_tokens),
            Some(self.total_tokens),
            cached,
            written,
            reasoning,
        ]
        .into_iter()
        .flatten()
        .any(|n| n < 0)
            || i64::from(self.input_tokens) + i64::from(self.output_tokens)
                != i64::from(self.total_tokens)
            || (comparable_input
                && (cached.is_some_and(|n| n > self.input_tokens)
                    || written.is_some_and(|n| n > self.input_tokens)
                    || matches!((cached,written),(Some(a),Some(b)) if i64::from(a)+i64::from(b) > i64::from(self.input_tokens))))
            || reasoning.is_some_and(|n| n > self.output_tokens)
        {
            return Err(ApiError::protocol());
        }
        if let Some(m) = &self.decora {
            let consistent = matches!(
                (&m.provider, &m.input_tokens_basis, &m.total_tokens_basis),
                (
                    UsageProvider::Typesafe,
                    InputTokenBasis::BillableInput,
                    TotalTokenBasis::BillableInputPlusOutput
                ) | (
                    UsageProvider::Openrouter,
                    InputTokenBasis::ProviderReported,
                    TotalTokenBasis::ProviderInputPlusOutput
                ) | (
                    UsageProvider::Gemini,
                    InputTokenBasis::TotalInputTokens,
                    TotalTokenBasis::TotalInputPlusOutput
                )
            );
            let modality_valid = m.input_tokens_by_modality.as_ref().is_none_or(|items| {
                items.iter().all(|item| {
                    item.as_object().is_some_and(|obj| {
                        obj.get("modality")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                            && obj
                                .get("tokens")
                                .and_then(Value::as_i64)
                                .and_then(|n| i32::try_from(n).ok())
                                .is_some_and(|n| n >= 0)
                    })
                })
            });
            if !consistent
                || !modality_valid
                || m.details_complete != self.details_complete()
                || m.provider_cost.is_some_and(|n| !n.is_finite() || n < 0.0)
            {
                return Err(ApiError::protocol());
            }
        }
        Ok(())
    }
}
fn probability(p: f64) -> bool {
    p.is_finite() && (0.0..=1.0).contains(&p)
}
const EPS: f64 = 0.0001;
impl DecisionResponse {
    /// 验证题序、名称、选项覆盖、概率和、评分期望及 usage 守恒。
    pub fn parse(value: Value, req: &DecisionRequest) -> Result<Self, ApiError> {
        Self::parse_with_usage(value, req, UsagePolicy::Strict)
    }
    /// 校验相同的核心答案合同，并按显式策略验证 usage 明细与来源口径。
    pub fn parse_with_usage(
        value: Value,
        req: &DecisionRequest,
        policy: UsagePolicy,
    ) -> Result<Self, ApiError> {
        if !value["answers"]
            .as_array()
            .is_some_and(|a| a.iter().all(|a| a.get("name").is_some()))
        {
            return Err(ApiError::protocol());
        }
        let mut response: Self = serde_json::from_value(value).map_err(|_| ApiError::protocol())?;
        if response.model.is_empty() || response.answers.len() != req.questions.len() {
            return Err(ApiError::protocol());
        }
        for (answer, q) in response.answers.iter().zip(&req.questions) {
            let name = match answer {
                Answer::Predicate { name, .. }
                | Answer::Choice { name, .. }
                | Answer::Score { name, .. }
                | Answer::Refusal { name } => name,
            };
            if name != q.name() {
                return Err(ApiError::protocol());
            }
            let valid = match (answer, q) {
                (Answer::Refusal { .. }, _) => true,
                (Answer::Predicate { probability: p, .. }, Question::Predicate { .. }) => {
                    probability(*p)
                }
                (
                    Answer::Choice {
                        choice,
                        confidence,
                        probabilities,
                        ..
                    },
                    Question::Choice { choices, .. },
                ) => {
                    probability(*confidence)
                        && probabilities.len() == choices.len()
                        && choices.iter().all(|c| {
                            probabilities.iter().filter(|p| p.value == c.value).count() == 1
                        })
                        && probabilities.iter().all(|p| probability(p.probability))
                        && (probabilities.iter().map(|p| p.probability).sum::<f64>() - 1.0).abs()
                            <= EPS
                        && probabilities
                            .iter()
                            .find(|p| &p.value == choice)
                            .is_some_and(|selected| {
                                probabilities
                                    .iter()
                                    .all(|p| p.probability <= selected.probability + EPS)
                            })
                }
                (
                    Answer::Score {
                        score,
                        confidence,
                        probabilities,
                        ..
                    },
                    Question::Score { levels, .. },
                ) => {
                    probability(*confidence)
                        && score.is_finite()
                        && *score >= 0.0
                        && *score <= (levels.len() - 1) as f64
                        && probabilities.len() == levels.len()
                        && levels.iter().enumerate().all(|(i, l)| {
                            probabilities
                                .iter()
                                .filter(|p| p.value == i && p.label == l.label)
                                .count()
                                == 1
                        })
                        && probabilities.iter().all(|p| probability(p.probability))
                        && (probabilities.iter().map(|p| p.probability).sum::<f64>() - 1.0).abs()
                            <= EPS
                        && (probabilities
                            .iter()
                            .map(|p| p.probability * p.value as f64)
                            .sum::<f64>()
                            - score)
                            .abs()
                            <= EPS
                }
                _ => false,
            };
            if !valid {
                return Err(ApiError::protocol());
            }
        }
        response.usage.validate(policy)?;
        response.model = req.model.clone();
        Ok(response)
    }
}
