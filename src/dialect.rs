//! 多前端 Decisions 方言编解码：同方言原生直通（仅替换 model 且保留数学/usage 校验），异构方言经标准中间表示无损互转。
use crate::{
    config::{BackendKind, UsagePolicy},
    error::ApiError,
    protocol::{Answer, ChoiceValue, DecisionRequest, DecisionResponse, Question, Requirements},
};
use axum::http::StatusCode;
use serde_json::{Map, Value, json};

/// 支持的 Decisions 入站协议方言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Openai,
    Gemini,
    Typesafe,
    Openrouter,
}

fn exact_keys(
    value: &Value,
    allowed: &[&str],
    required: &[&str],
) -> Result<Map<String, Value>, ApiError> {
    let obj = value.as_object().ok_or_else(ApiError::invalid)?;
    if obj.keys().any(|k| !allowed.contains(&k.as_str()))
        || required.iter().any(|k| !obj.contains_key(*k))
    {
        return Err(ApiError::invalid());
    }
    Ok(obj.clone())
}

fn gemini_question_key_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn gemini_choice_key_valid(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128 && key.is_ascii() && key.trim() == key
}

fn parse_gemini_input(input: &Value) -> Result<Value, ApiError> {
    if input.is_string() {
        return Ok(input.clone());
    }
    let parts = input.as_array().ok_or_else(ApiError::invalid)?;
    if parts.is_empty() || parts.len() > 16384 {
        return Err(ApiError::invalid());
    }
    let mut canonical_parts = Vec::with_capacity(parts.len());
    for part in parts {
        let obj = part.as_object().ok_or_else(ApiError::invalid)?;
        if obj.len() != 1 {
            return Err(ApiError::invalid());
        }
        if let Some(text) = obj.get("text").and_then(Value::as_str) {
            canonical_parts.push(json!({"type": "input_text", "text": text}));
        } else if let Some(inline) = obj.get("inline_data") {
            let inline_obj = exact_keys(inline, &["mime_type", "data"], &["mime_type", "data"])?;
            let mime = inline_obj["mime_type"]
                .as_str()
                .ok_or_else(ApiError::invalid)?;
            let data = inline_obj["data"].as_str().ok_or_else(ApiError::invalid)?;
            if mime.is_empty() || mime.contains(';') || data.is_empty() {
                return Err(ApiError::invalid());
            }
            canonical_parts.push(json!({
                "type": "input_image",
                "image_url": format!("data:{mime};base64,{data}"),
            }));
        } else {
            return Err(ApiError::invalid());
        }
    }
    Ok(json!([{"role": "user", "content": canonical_parts}]))
}

fn token_i32(source: &Map<String, Value>, key: &str) -> Result<i32, ApiError> {
    source
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| *n >= 0)
        .ok_or_else(ApiError::protocol)
}

impl Dialect {
    /// 判断入站端点方言是否与目标后端协议同构（可启用原生直通）。
    pub fn matches_backend(self, kind: BackendKind) -> bool {
        matches!(
            (self, kind),
            (Self::Openai, BackendKind::Openai)
                | (Self::Gemini, BackendKind::Gemini)
                | (Self::Typesafe, BackendKind::Typesafe)
                | (Self::Openrouter, BackendKind::Openrouter)
        )
    }

    /// 构造同方言原生直通请求体：保留客户端原始字段与键名，仅替换顶层 model。
    pub fn passthrough_body(
        self,
        raw_request: &Value,
        req: &DecisionRequest,
        backend_model: &str,
    ) -> Value {
        if self == Self::Openai {
            return req.openai_body(backend_model);
        }
        let mut body = raw_request.clone();
        body["model"] = json!(backend_model);
        body
    }

    /// 校验同方言原生直通响应的数学概率、legend 与 usage 守恒，并原样返回（不注入 usage.decora）。
    pub fn passthrough_response(
        self,
        raw_request: &Value,
        req: &DecisionRequest,
        upstream_body: Value,
    ) -> Result<Value, ApiError> {
        if self == Self::Openai {
            // OpenAI 官方本来就保证完整 usage；不把其缺项伪装成兼容模式。
            let parsed = DecisionResponse::parse(upstream_body, req)?;
            return serde_json::to_value(parsed).map_err(|_| ApiError::protocol());
        }
        if self == Self::Gemini {
            if let Some(m) = upstream_body.get("model")
                && m.as_str().is_none_or(str::is_empty)
            {
                return Err(ApiError::protocol());
            }
        } else if upstream_body["model"].as_str().is_none_or(str::is_empty) {
            return Err(ApiError::protocol());
        }
        let answers_obj = upstream_body
            .get("answers")
            .and_then(Value::as_object)
            .ok_or_else(ApiError::protocol)?;
        if answers_obj.len() != req.questions.len() {
            return Err(ApiError::protocol());
        }
        let mut canonical_answers = Vec::with_capacity(req.questions.len());
        for q in &req.questions {
            let key = q.name().as_ref().ok_or_else(ApiError::protocol)?;
            let a = answers_obj.get(key).ok_or_else(ApiError::protocol)?;
            let mut canonical = json!({"type": q.kind(), "name": q.name()});
            match q {
                Question::Predicate { .. } => {
                    if a["type"] != "noul" {
                        return Err(ApiError::protocol());
                    }
                    canonical["probability"] = a["noul"].clone();
                }
                Question::Choice { choices, .. } => {
                    if a["type"] != "choice" {
                        return Err(ApiError::protocol());
                    }
                    let probs = a["probabilities"]
                        .as_object()
                        .ok_or_else(ApiError::protocol)?;
                    if probs.len() != choices.len() {
                        return Err(ApiError::protocol());
                    }
                    let selected = a["choice"].as_str().ok_or_else(ApiError::protocol)?;
                    canonical["choice"] = json!(selected);
                    canonical["confidence"] = a["confidence"].clone();
                    let mut mapped_probs = Vec::with_capacity(choices.len());
                    for c in choices {
                        let ChoiceValue::String(opt_key) = &c.value else {
                            return Err(ApiError::protocol());
                        };
                        let p = probs.get(opt_key).ok_or_else(ApiError::protocol)?;
                        mapped_probs.push(json!({"value": opt_key, "probability": p}));
                    }
                    canonical["probabilities"] = Value::Array(mapped_probs);
                }
                Question::Score { levels, .. } => {
                    if a["type"] != "score" {
                        return Err(ApiError::protocol());
                    }
                    let probs = a["probabilities"]
                        .as_object()
                        .ok_or_else(ApiError::protocol)?;
                    let legend = a["legend"].as_object().ok_or_else(ApiError::protocol)?;
                    if probs.len() != levels.len() || legend.len() != levels.len() {
                        return Err(ApiError::protocol());
                    }
                    let expected_criteria = raw_request["questions"][key]["criteria"]
                        .as_array()
                        .ok_or_else(ApiError::protocol)?;
                    for i in 0..levels.len() {
                        if legend.get(&i.to_string()) != expected_criteria.get(i) {
                            return Err(ApiError::protocol());
                        }
                    }
                    canonical["score"] = a["score"].clone();
                    canonical["confidence"] = a["confidence"].clone();
                    canonical["probabilities"] = json!(
                        levels
                            .iter()
                            .enumerate()
                            .map(|(i, l)| {
                                json!({
                                    "value": i,
                                    "label": l.label,
                                    "probability": probs.get(&i.to_string()),
                                })
                            })
                            .collect::<Vec<_>>()
                    );
                }
            }
            canonical_answers.push(canonical);
        }

        let source = upstream_body["usage"]
            .as_object()
            .ok_or_else(ApiError::protocol)?;
        let canonical_usage = if self == Self::Gemini {
            let total_input = token_i32(source, "total_input_tokens")?;
            let total_output = token_i32(source, "total_output_tokens")?;
            let total = token_i32(source, "total_tokens")?;
            if total_input.checked_add(total_output) != Some(total) {
                return Err(ApiError::protocol());
            }
            if let Some(modality) = source.get("input_tokens_by_modality") {
                let items = modality.as_array().ok_or_else(ApiError::protocol)?;
                for item in items {
                    let obj = item.as_object().ok_or_else(ApiError::protocol)?;
                    if obj
                        .get("modality")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                        || obj
                            .get("tokens")
                            .and_then(Value::as_i64)
                            .and_then(|n| i32::try_from(n).ok())
                            .is_none_or(|n| n < 0)
                    {
                        return Err(ApiError::protocol());
                    }
                }
            }
            json!({
                "input_tokens": total_input,
                "output_tokens": total_output,
                "total_tokens": total,
            })
        } else {
            let input = token_i32(source, "input_tokens")?;
            let output = token_i32(source, "output_tokens")?;
            let total = input.checked_add(output).ok_or_else(ApiError::protocol)?;
            if source
                .get("total_tokens")
                .is_some_and(|n| n.as_i64() != Some(i64::from(total)))
            {
                return Err(ApiError::protocol());
            }
            if let Some(cost) = source.get("cost")
                && cost.as_f64().is_none_or(|n| !n.is_finite() || n < 0.0)
            {
                return Err(ApiError::protocol());
            }
            let mut u =
                json!({"input_tokens": input, "output_tokens": output, "total_tokens": total});
            for (field, known) in [
                (
                    "input_tokens_details",
                    &["cached_tokens", "cache_write_tokens"][..],
                ),
                ("output_tokens_details", &["reasoning_tokens"][..]),
            ] {
                if let Some(details) = source.get(field) {
                    let details = details.as_object().ok_or_else(ApiError::protocol)?;
                    if self == Self::Openrouter
                        && field == "input_tokens_details"
                        && !details.contains_key("cached_tokens")
                    {
                        return Err(ApiError::protocol());
                    }
                    let mut available = Map::new();
                    for key in known {
                        if let Some(value) = details.get(*key) {
                            let count = value
                                .as_i64()
                                .and_then(|n| i32::try_from(n).ok())
                                .filter(|n| *n >= 0)
                                .ok_or_else(ApiError::protocol)?;
                            available.insert((*key).into(), json!(count));
                        }
                    }
                    if !available.is_empty() {
                        u[field] = Value::Object(available);
                    }
                }
            }
            if self == Self::Typesafe {
                u["decora"] = json!({
                    "provider": "typesafe",
                    "input_tokens_basis": "billable_input",
                    "total_tokens_basis": "billable_input_plus_output",
                    "details_complete": u["input_tokens_details"].get("cached_tokens").is_some()
                        && u["input_tokens_details"].get("cache_write_tokens").is_some()
                        && u["output_tokens_details"].get("reasoning_tokens").is_some(),
                });
            }
            u
        };

        // 复用 DecisionResponse::parse_with_usage 严格验证所有题目的概率和、最大值及期望守恒。
        DecisionResponse::parse_with_usage(
            json!({
                "model": req.model,
                "answers": canonical_answers,
                "usage": canonical_usage,
            }),
            req,
            UsagePolicy::Compatible,
        )?;

        let mut out = upstream_body;
        out["model"] = json!(req.model);
        Ok(out)
    }

    /// 按入站端点对应的官方方言严格校验请求，并转换为网关内部标准中间表示。
    pub fn parse_request(
        self,
        value: Value,
        max_images: usize,
    ) -> Result<(DecisionRequest, Requirements), ApiError> {
        match self {
            Self::Openai => DecisionRequest::parse(value, max_images),
            Self::Gemini => {
                let root = exact_keys(
                    &value,
                    &["model", "input", "questions"],
                    &["model", "input", "questions"],
                )?;
                let input = parse_gemini_input(&root["input"])?;
                let qs = root["questions"]
                    .as_object()
                    .ok_or_else(ApiError::invalid)?;
                if qs.is_empty() || qs.len() > 400 {
                    return Err(ApiError::invalid());
                }
                let mut questions = Vec::with_capacity(qs.len());
                for (name, q) in qs {
                    if !gemini_question_key_valid(name) {
                        return Err(ApiError::invalid());
                    }
                    let q_type = q
                        .get("type")
                        .and_then(Value::as_str)
                        .ok_or_else(ApiError::invalid)?;
                    let instructions = q
                        .get("instructions")
                        .and_then(Value::as_str)
                        .ok_or_else(ApiError::invalid)?;
                    match q_type {
                        "noul" => {
                            exact_keys(q, &["type", "instructions"], &["type", "instructions"])?;
                            questions.push(json!({
                                "type": "predicate",
                                "name": name,
                                "instructions": instructions,
                            }));
                        }
                        "choice" => {
                            exact_keys(
                                q,
                                &["type", "instructions", "criteria"],
                                &["type", "instructions", "criteria"],
                            )?;
                            let criteria =
                                q["criteria"].as_object().ok_or_else(ApiError::invalid)?;
                            if !(2..=255).contains(&criteria.len()) {
                                return Err(ApiError::invalid());
                            }
                            let mut choices = Vec::with_capacity(criteria.len());
                            for (opt, desc) in criteria {
                                if !gemini_choice_key_valid(opt) {
                                    return Err(ApiError::invalid());
                                }
                                let desc_str = desc.as_str().ok_or_else(ApiError::invalid)?;
                                if desc_str.is_empty() {
                                    choices.push(json!({"value": opt}));
                                } else {
                                    choices.push(json!({"value": opt, "description": desc_str}));
                                }
                            }
                            questions.push(json!({
                                "type": "choice",
                                "name": name,
                                "instructions": instructions,
                                "choices": choices,
                            }));
                        }
                        "score" => {
                            exact_keys(
                                q,
                                &["type", "instructions", "criteria"],
                                &["type", "instructions", "criteria"],
                            )?;
                            let criteria =
                                q["criteria"].as_array().ok_or_else(ApiError::invalid)?;
                            if !(2..=10).contains(&criteria.len()) {
                                return Err(ApiError::invalid());
                            }
                            let mut levels = Vec::with_capacity(criteria.len());
                            for item in criteria {
                                let label = item.as_str().ok_or_else(ApiError::invalid)?;
                                if label.is_empty() {
                                    return Err(ApiError::invalid());
                                }
                                levels.push(json!({"label": label}));
                            }
                            questions.push(json!({
                                "type": "score",
                                "name": name,
                                "instructions": instructions,
                                "levels": levels,
                            }));
                        }
                        _ => return Err(ApiError::invalid()),
                    }
                }
                DecisionRequest::parse(
                    json!({
                        "model": root["model"],
                        "input": input,
                        "questions": questions,
                    }),
                    max_images,
                )
            }
            Self::Typesafe | Self::Openrouter => {
                let root = exact_keys(
                    &value,
                    &["model", "state", "questions"],
                    &["model", "state", "questions"],
                )?;
                if !root["state"].is_string() {
                    return Err(ApiError::invalid());
                }
                let qs = root["questions"]
                    .as_object()
                    .ok_or_else(ApiError::invalid)?;
                if qs.is_empty() || qs.len() > 400 {
                    return Err(ApiError::invalid());
                }
                let mut questions = Vec::with_capacity(qs.len());
                for (name, q) in qs {
                    if name.is_empty() {
                        return Err(ApiError::invalid());
                    }
                    let q_type = q
                        .get("type")
                        .and_then(Value::as_str)
                        .ok_or_else(ApiError::invalid)?;
                    let instructions = q
                        .get("instructions")
                        .and_then(Value::as_str)
                        .ok_or_else(ApiError::invalid)?;
                    match q_type {
                        "noul" => {
                            exact_keys(q, &["type", "instructions"], &["type", "instructions"])?;
                            questions.push(json!({
                                "type": "predicate",
                                "name": name,
                                "instructions": instructions,
                            }));
                        }
                        "choice" => {
                            exact_keys(
                                q,
                                &["type", "instructions", "criteria"],
                                &["type", "instructions", "criteria"],
                            )?;
                            let criteria =
                                q["criteria"].as_object().ok_or_else(ApiError::invalid)?;
                            if !(2..=255).contains(&criteria.len()) {
                                return Err(ApiError::invalid());
                            }
                            let mut choices = Vec::with_capacity(criteria.len());
                            for (opt, desc) in criteria {
                                if opt.is_empty() {
                                    return Err(ApiError::invalid());
                                }
                                match desc {
                                    Value::Null => choices.push(json!({"value": opt})),
                                    Value::String(d) => {
                                        choices.push(json!({"value": opt, "description": d}))
                                    }
                                    _ => return Err(ApiError::invalid()),
                                }
                            }
                            questions.push(json!({
                                "type": "choice",
                                "name": name,
                                "instructions": instructions,
                                "choices": choices,
                            }));
                        }
                        "score" => {
                            exact_keys(
                                q,
                                &["type", "instructions", "criteria"],
                                &["type", "instructions", "criteria"],
                            )?;
                            let criteria =
                                q["criteria"].as_array().ok_or_else(ApiError::invalid)?;
                            if !(2..=10).contains(&criteria.len()) {
                                return Err(ApiError::invalid());
                            }
                            let mut levels = Vec::with_capacity(criteria.len());
                            for item in criteria {
                                match item {
                                    Value::String(label) => levels.push(json!({"label": label})),
                                    Value::Object(_) => {
                                        let obj = exact_keys(
                                            item,
                                            &["label", "description"],
                                            &["label"],
                                        )?;
                                        if !obj["label"].is_string()
                                            || obj
                                                .get("description")
                                                .is_some_and(|d| !d.is_string())
                                        {
                                            return Err(ApiError::invalid());
                                        }
                                        levels.push(Value::Object(obj));
                                    }
                                    _ => return Err(ApiError::invalid()),
                                }
                            }
                            questions.push(json!({
                                "type": "score",
                                "name": name,
                                "instructions": instructions,
                                "levels": levels,
                            }));
                        }
                        _ => return Err(ApiError::invalid()),
                    }
                }
                DecisionRequest::parse(
                    json!({
                        "model": root["model"],
                        "input": root["state"],
                        "questions": questions,
                    }),
                    max_images,
                )
            }
        }
    }

    /// 将经过数学与 usage 守恒校验的标准中间响应格式化为入站方言对应的跨方言响应体。
    pub fn format_response(
        self,
        req: &DecisionRequest,
        resp: DecisionResponse,
    ) -> Result<Value, ApiError> {
        if self == Self::Openai {
            return serde_json::to_value(resp).map_err(|_| ApiError::protocol());
        }
        let mut answers = Map::new();
        for (answer, q) in resp.answers.iter().zip(&req.questions) {
            let key = q.name().clone().ok_or_else(ApiError::protocol)?;
            match (answer, q) {
                (Answer::Refusal { .. }, _) => {
                    return Err(ApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "upstream_refusal_unrepresentable",
                        "This dialect cannot express a per-question refusal; no probabilities were fabricated.",
                    ));
                }
                (Answer::Predicate { probability, .. }, Question::Predicate { .. }) => {
                    answers.insert(key, json!({"type": "noul", "noul": probability}));
                }
                (
                    Answer::Choice {
                        choice,
                        confidence,
                        probabilities,
                        ..
                    },
                    Question::Choice { .. },
                ) => {
                    let ChoiceValue::String(selected) = choice else {
                        return Err(ApiError::protocol());
                    };
                    let mut probs_map = Map::new();
                    for p in probabilities {
                        let ChoiceValue::String(opt) = &p.value else {
                            return Err(ApiError::protocol());
                        };
                        probs_map.insert(opt.clone(), json!(p.probability));
                    }
                    answers.insert(
                        key,
                        json!({
                            "type": "choice",
                            "choice": selected,
                            "confidence": confidence,
                            "probabilities": probs_map,
                        }),
                    );
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
                    let mut probs_map = Map::new();
                    let mut legend_map = Map::new();
                    for (i, level) in levels.iter().enumerate() {
                        let p = probabilities
                            .iter()
                            .find(|p| p.value == i)
                            .ok_or_else(ApiError::protocol)?;
                        probs_map.insert(i.to_string(), json!(p.probability));
                        let criterion = if self == Self::Gemini {
                            json!(level.label)
                        } else {
                            match &level.description {
                                Some(d) => json!({"label": level.label, "description": d}),
                                None => json!(level.label),
                            }
                        };
                        legend_map.insert(i.to_string(), criterion);
                    }
                    answers.insert(
                        key,
                        json!({
                            "type": "score",
                            "score": score,
                            "confidence": confidence,
                            "probabilities": probs_map,
                            "legend": legend_map,
                        }),
                    );
                }
                _ => return Err(ApiError::protocol()),
            }
        }
        let decora = match &resp.usage.decora {
            Some(meta) => serde_json::to_value(meta).map_err(|_| ApiError::protocol())?,
            None => json!({
                "provider": "openai",
                "input_tokens_basis": "openai_input_tokens",
                "total_tokens_basis": "openai_input_plus_output",
                "details_complete": resp.usage.details_complete(),
            }),
        };
        let usage = if self == Self::Gemini {
            let mut u = json!({
                "total_input_tokens": resp.usage.input_tokens,
                "total_output_tokens": resp.usage.output_tokens,
                "total_tokens": resp.usage.total_tokens,
            });
            if let Some(modality) = decora.get("input_tokens_by_modality") {
                u["input_tokens_by_modality"] = modality.clone();
            }
            u["decora"] = decora;
            u
        } else {
            let mut u = serde_json::to_value(&resp.usage).map_err(|_| ApiError::protocol())?;
            u["decora"] = decora;
            u
        };
        Ok(json!({
            "model": resp.model,
            "answers": answers,
            "usage": usage,
        }))
    }
}
