//! 多前端 Decisions 方言编解码：入站严格校验并收敛至标准中间表示，出站按调用方方言格式化。
use crate::{
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

impl Dialect {
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

    /// 将经过数学与 usage 守恒校验的标准中间响应格式化为入站方言对应的响应体。
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
                        "当前入口协议不支持逐题 refusal 表达；未伪造概率。",
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
