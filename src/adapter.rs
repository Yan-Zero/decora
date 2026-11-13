//! 官方原生协议映射：默认保留真实可用 usage；严格模式拒绝口径或必需明细差异。
use crate::{
    config::{Backend, BackendKind, UsagePolicy},
    dialect::Dialect,
    error::ApiError,
    protocol::{
        Answer, ChoiceOption, ChoiceValue, DecisionRequest, DecisionResponse, Question,
        Requirements, ScoreLevel,
    },
};
use serde_json::{Map, Value, json};

/// 检查管理员声明和 adapter 固有边界；默认按 OpenAI 入站方言评估。
pub fn compatible(
    backend: &Backend,
    req: &DecisionRequest,
    need: &Requirements,
    usage_policy: UsagePolicy,
) -> Result<(), ApiError> {
    compatible_for_dialect(Dialect::Openai, backend, req, need, usage_policy)
}

/// 按入站方言检查候选后端能力边界；同方言原生直通在 strict usage 下同样放行。
pub fn compatible_for_dialect(
    dialect: Dialect,
    backend: &Backend,
    req: &DecisionRequest,
    need: &Requirements,
    usage_policy: UsagePolicy,
) -> Result<(), ApiError> {
    let c = &backend.capabilities;
    if req.questions.len() > c.max_questions
        || req
            .questions
            .iter()
            .any(|q| !c.question_types.iter().any(|t| t == q.kind()))
        || (need.images > 0 && !c.images)
        || (need.cache && !c.prompt_cache)
        || (need.safety && !c.safety_identifier)
    {
        return Err(ApiError::unsupported("unsupported_capability"));
    }
    if backend.kind != BackendKind::Openai {
        validate_native_request(req)?;
    }
    if usage_policy == UsagePolicy::Strict
        && !dialect.matches_backend(backend.kind)
        && backend.kind != BackendKind::Openai
    {
        return Err(ApiError::unsupported("unsupported_usage"));
    }
    Ok(())
}
fn parts_have_images(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|parts| parts.iter().any(|p| p["type"] == "input_image"))
}
fn validate_native_request(req: &DecisionRequest) -> Result<(), ApiError> {
    // 只检查真正的内容 part，不把 input_json.data 中同名字段当图片。
    let images = req
        .input
        .as_array()
        .is_some_and(|messages| messages.iter().any(|m| parts_have_images(&m["content"])))
        || req
            .questions
            .iter()
            .any(|q| parts_have_images(q.instructions()));
    if images {
        return Err(ApiError::unsupported("unsupported_native_images"));
    }
    let cache_markers = req.input.as_array().is_some_and(|messages| {
        messages.iter().any(|m| {
            m["content"].as_array().is_some_and(|parts| {
                parts.iter().any(|p| {
                    p.get("prompt_cache_breakpoint")
                        .is_some_and(|b| !b.is_null())
                })
            })
        })
    });
    if cache_markers
        || req.safety_identifier.is_some()
        || req.prompt_cache_key.is_some()
        || req.prompt_cache_options.is_some()
    {
        return Err(ApiError::unsupported("unsupported_native_mapping"));
    }
    Ok(())
}
fn choice_key_for(choices: &[ChoiceOption], index: usize, kind: BackendKind) -> String {
    if kind == BackendKind::Gemini
        || choices
            .iter()
            .any(|c| matches!(c.value, ChoiceValue::Bool(_)))
    {
        // Gemini 或含布尔选项组统一重命名，避免非 ASCII/空串限制及同名字符串碰撞。
        format!("decora_choice_{index}")
    } else {
        let ChoiceValue::String(s) = &choices[index].value else {
            unreachable!("all strings")
        };
        s.clone()
    }
}
fn score_criterion(level: &ScoreLevel, kind: BackendKind) -> Value {
    if kind == BackendKind::Gemini {
        match &level.description {
            Some(d) => json!(json!({"label": level.label, "description": d}).to_string()),
            None if level.label.is_empty() => {
                json!(json!({"label": level.label, "description": Value::Null}).to_string())
            }
            None => json!(level.label),
        }
    } else {
        match &level.description {
            Some(d) => json!({"label": level.label, "description": d}),
            None => json!(level.label),
        }
    }
}
fn textual(value: &Value, kind: &str) -> Value {
    if value.is_string() {
        return value.clone();
    }
    // JSON 转义明确 data 边界，但不声称能阻止模型层 prompt injection。
    json!(
        json!({"decora_encoding":"openai_decisions_text_v1","kind":kind,"data":value}).to_string()
    )
}
/// 自动将标准中间请求转换为上游原生 body；转换是网关内部实现，不是客户端 API。
pub fn native_body(req: &DecisionRequest, model: &str) -> Result<Value, ApiError> {
    native_body_for(req, model, BackendKind::Openrouter)
}

/// 按提供方原生协议构造请求体。
pub fn native_body_for(
    req: &DecisionRequest,
    model: &str,
    kind: BackendKind,
) -> Result<Value, ApiError> {
    validate_native_request(req)?;
    let mut questions = Map::new();
    for (index, q) in req.questions.iter().enumerate() {
        let mut question = json!({
            "type": q.kind(),
            "instructions": textual(q.instructions(), "question_content"),
        });
        match q {
            Question::Predicate { .. } => question["type"] = json!("noul"),
            Question::Choice { choices, .. } => {
                let typed = choices
                    .iter()
                    .any(|c| matches!(c.value, ChoiceValue::Bool(_)));
                let mut criteria = Map::new();
                for (i, choice) in choices.iter().enumerate() {
                    let criterion = if kind == BackendKind::Gemini {
                        let s = match &choice.description {
                            Some(d) => json!({"value": choice.value, "description": d}).to_string(),
                            None => json!({"value": choice.value}).to_string(),
                        };
                        json!(s)
                    } else if typed {
                        let mut c = json!({"value": choice.value});
                        if let Some(description) = &choice.description {
                            c["description"] = json!(description);
                        }
                        c
                    } else {
                        json!(choice.description)
                    };
                    criteria.insert(choice_key_for(choices, i, kind), criterion);
                }
                question["criteria"] = Value::Object(criteria);
            }
            Question::Score { levels, .. } => {
                question["criteria"] = json!(
                    levels
                        .iter()
                        .map(|l| score_criterion(l, kind))
                        .collect::<Vec<_>>()
                );
            }
        }
        // 公开名称可重复、可省略；内部键稳定且无碰撞，原名称只在最终答案还原。
        questions.insert(format!("q{index}"), question);
    }
    let mut root = json!({
        "model": model,
        "questions": questions,
    });
    let input_field = if kind == BackendKind::Gemini {
        "input"
    } else {
        "state"
    };
    root[input_field] = textual(&req.input, "messages");
    Ok(root)
}

/// 将 native 答案重排并转换为标准中间答案形状；概率及 confidence 原样保留。
pub fn native_answers(value: &Value, req: &DecisionRequest) -> Result<Vec<Answer>, ApiError> {
    native_answers_for(value, req, BackendKind::Openrouter)
}

/// 按后端种类重排并转换答案。
pub fn native_answers_for(
    value: &Value,
    req: &DecisionRequest,
    kind: BackendKind,
) -> Result<Vec<Answer>, ApiError> {
    validate_native_request(req)?;
    let answers = value
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(ApiError::protocol)?;
    if answers.len() != req.questions.len() {
        return Err(ApiError::protocol());
    }
    let mut output = Vec::new();
    for (index, q) in req.questions.iter().enumerate() {
        let key = format!("q{index}");
        let a = answers.get(&key).ok_or_else(ApiError::protocol)?;
        let mut answer = json!({"type": q.kind(), "name": q.name()});
        match q {
            Question::Predicate { .. } => {
                if a["type"] != "noul" {
                    return Err(ApiError::protocol());
                }
                answer["probability"] = a["noul"].clone();
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
                let selected_index = (0..choices.len())
                    .find(|i| choice_key_for(choices, *i, kind) == selected)
                    .ok_or_else(ApiError::protocol)?;
                answer["choice"] = json!(choices[selected_index].value);
                answer["confidence"] = a["confidence"].clone();
                answer["probabilities"] = json!(
                    choices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            json!({
                                "value": c.value,
                                "probability": probs.get(&choice_key_for(choices, i, kind)),
                            })
                        })
                        .collect::<Vec<_>>()
                );
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
                for (i, level) in levels.iter().enumerate() {
                    if legend.get(&i.to_string()) != Some(&score_criterion(level, kind)) {
                        return Err(ApiError::protocol());
                    }
                }
                answer["score"] = a["score"].clone();
                answer["confidence"] = a["confidence"].clone();
                answer["probabilities"] = json!(
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
        output.push(serde_json::from_value(answer).map_err(|_| ApiError::protocol())?);
    }
    Ok(output)
}

fn token_i32(source: &Map<String, Value>, key: &str) -> Result<i32, ApiError> {
    source
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(ApiError::protocol)
}

/// 将官方原生响应转换为核心答案及透明的 usage 兼容扩展。
/// input/output 直接来自上游；total 是显式标记口径的两数之和，未知明细不补零。
pub fn native_response(
    value: Value,
    req: &DecisionRequest,
    kind: BackendKind,
    policy: UsagePolicy,
) -> Result<DecisionResponse, ApiError> {
    if policy == UsagePolicy::Strict {
        return Err(ApiError::unsupported("unsupported_usage"));
    }
    if kind == BackendKind::Openai {
        return Err(ApiError::protocol());
    }
    if kind == BackendKind::Gemini {
        if let Some(m) = value.get("model")
            && m.as_str().is_none_or(str::is_empty)
        {
            return Err(ApiError::protocol());
        }
    } else if value["model"].as_str().is_none_or(str::is_empty) {
        return Err(ApiError::protocol());
    }
    let answers = native_answers_for(&value, req, kind)?;
    let source = value["usage"].as_object().ok_or_else(ApiError::protocol)?;

    if kind == BackendKind::Gemini {
        let total_input = token_i32(source, "total_input_tokens")?;
        let total_output = token_i32(source, "total_output_tokens")?;
        let total = token_i32(source, "total_tokens")?;
        if total_input.checked_add(total_output) != Some(total) {
            return Err(ApiError::protocol());
        }
        let mut decora = json!({
            "provider": "gemini",
            "input_tokens_basis": "total_input_tokens",
            "total_tokens_basis": "total_input_plus_output",
            "details_complete": false,
        });
        if let Some(modality) = source.get("input_tokens_by_modality") {
            if !modality.is_array() {
                return Err(ApiError::protocol());
            }
            decora["input_tokens_by_modality"] = modality.clone();
        }
        let usage = json!({
            "input_tokens": total_input,
            "output_tokens": total_output,
            "total_tokens": total,
            "decora": decora,
        });
        return DecisionResponse::parse_with_usage(
            json!({"model": req.model, "answers": answers, "usage": usage}),
            req,
            policy,
        );
    }

    let input = token_i32(source, "input_tokens")?;
    let output = token_i32(source, "output_tokens")?;
    let total = input.checked_add(output).ok_or_else(ApiError::protocol)?;
    if source
        .get("total_tokens")
        .is_some_and(|n| n.as_i64() != Some(i64::from(total)))
    {
        return Err(ApiError::protocol());
    }
    let mut usage = json!({"input_tokens": input, "output_tokens": output, "total_tokens": total});
    for (field, known) in [
        (
            "input_tokens_details",
            &["cached_tokens", "cache_write_tokens"][..],
        ),
        ("output_tokens_details", &["reasoning_tokens"][..]),
    ] {
        if let Some(details) = source.get(field) {
            let details = details.as_object().ok_or_else(ApiError::protocol)?;
            if kind == BackendKind::Openrouter
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
                        .ok_or_else(ApiError::protocol)?;
                    available.insert((*key).into(), json!(count));
                }
            }
            if !available.is_empty() {
                usage[field] = Value::Object(available);
            }
        }
    }
    let complete = usage["input_tokens_details"].get("cached_tokens").is_some()
        && usage["input_tokens_details"]
            .get("cache_write_tokens")
            .is_some()
        && usage["output_tokens_details"]
            .get("reasoning_tokens")
            .is_some();
    usage["decora"] = match kind {
        BackendKind::Typesafe => {
            json!({"provider":"typesafe","input_tokens_basis":"billable_input","total_tokens_basis":"billable_input_plus_output","details_complete":complete})
        }
        BackendKind::Openrouter => {
            json!({"provider":"openrouter","input_tokens_basis":"provider_reported","total_tokens_basis":"provider_input_plus_output","details_complete":complete})
        }
        BackendKind::Gemini | BackendKind::Openai => unreachable!("handled above"),
    };
    if let Some(cost) = source.get("cost") {
        let cost = cost
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or_else(ApiError::protocol)?;
        usage["decora"]["provider_cost"] = json!(cost);
    }
    DecisionResponse::parse_with_usage(
        json!({"model": value["model"], "answers": answers, "usage": usage}),
        req,
        policy,
    )
}
