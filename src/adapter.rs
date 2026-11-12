//! 官方原生协议映射：默认保留真实可用 usage；严格模式拒绝口径或必需明细差异。
use crate::{
    config::{Backend, BackendKind, UsagePolicy},
    error::ApiError,
    protocol::{Answer, ChoiceValue, DecisionRequest, DecisionResponse, Question, Requirements},
};
use serde_json::{Map, Value, json};

/// 检查管理员声明和 adapter 固有边界；绝不靠切换模型规避不兼容。
pub fn compatible(
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
        if usage_policy == UsagePolicy::Strict {
            return Err(ApiError::unsupported("unsupported_usage"));
        }
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
fn choice_key(choices: &[crate::protocol::ChoiceOption], index: usize) -> String {
    if choices
        .iter()
        .any(|c| matches!(c.value, ChoiceValue::Bool(_)))
    {
        // 全组选项统一重命名，原始字符串即使等于内部键也不会碰撞。
        format!("decora_choice_{index}")
    } else {
        let ChoiceValue::String(s) = &choices[index].value else {
            unreachable!("all strings")
        };
        s.clone()
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
/// 自动将固定 OpenAI 请求转换为上游原生 body；转换是网关内部实现，不是客户端 API。
/// 角色/JSON 编码保留数据，不冒充原生角色指令层级；真实图片、缓存和 safety 仍独立筛选。
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
        let mut question = json!({"type": q.kind(), "instructions": textual(q.instructions(), "question_content")});
        match q {
            Question::Predicate { .. } => question["type"] = json!("noul"),
            Question::Choice { choices, .. } => {
                let mut criteria = Map::new();
                if kind == BackendKind::Gemini {
                    for (i, choice) in choices.iter().enumerate() {
                        let criterion_str = match &choice.description {
                            Some(d) => json!({"value": choice.value, "description": d}).to_string(),
                            None => json!({"value": choice.value}).to_string(),
                        };
                        criteria.insert(format!("decora_choice_{i}"), json!(criterion_str));
                    }
                } else {
                    let typed = choices
                        .iter()
                        .any(|c| matches!(c.value, ChoiceValue::Bool(_)));
                    for (i, choice) in choices.iter().enumerate() {
                        let criterion = if typed {
                            let mut criterion = json!({"value":choice.value});
                            if let Some(description) = &choice.description {
                                criterion["description"] = json!(description);
                            }
                            criterion
                        } else {
                            json!(choice.description)
                        };
                        criteria.insert(choice_key(choices, i), criterion);
                    }
                }
                question["criteria"] = Value::Object(criteria);
            }
            Question::Score { levels, .. } => {
                if kind == BackendKind::Gemini {
                    question["criteria"] = json!(
                        levels
                            .iter()
                            .map(|l| match &l.description {
                                Some(d) => json!({"label": l.label, "description": d}).to_string(),
                                None if l.label.is_empty() => {
                                    json!({"label": l.label, "description": Value::Null})
                                        .to_string()
                                }
                                None => l.label.clone(),
                            })
                            .collect::<Vec<_>>()
                    );
                } else {
                    question["criteria"] = json!(
                        levels
                            .iter()
                            .map(|l| match &l.description {
                                Some(d) => json!({"label": l.label, "description": d}),
                                None => json!(l.label),
                            })
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        // 公开名称可重复、可省略；内部键稳定且无碰撞，原名称只在最终答案还原。
        questions.insert(format!("q{index}"), question);
    }
    let mut root = json!({
        "model": model,
        "questions": questions,
    });
    if kind == BackendKind::Gemini {
        root["input"] = textual(&req.input, "messages");
    } else {
        root["state"] = textual(&req.input, "messages");
    }
    Ok(root)
}

/// 将 native 答案重排并转换为 OpenAI 题型形状；概率及 confidence 原样保留。
/// 调用者仍须用 DecisionResponse::parse 校验概率和与期望。返回值没有 usage，不能独立用作 API 响应。
pub fn native_answers(value: &Value, req: &DecisionRequest) -> Result<Vec<Answer>, ApiError> {
    native_answers_for(value, req, BackendKind::Openrouter)
}

/// 按后端种类重排并转换答案。
pub fn native_answers_for(
    value: &Value,
    req: &DecisionRequest,
    kind: BackendKind,
) -> Result<Vec<Answer>, ApiError> {
    let body = native_body_for(req, "schema-validation", kind)?;
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
                let get_key = |i: usize| {
                    if kind == BackendKind::Gemini {
                        format!("decora_choice_{i}")
                    } else {
                        choice_key(choices, i)
                    }
                };
                let selected_index = (0..choices.len())
                    .find(|i| get_key(*i) == selected)
                    .ok_or_else(ApiError::protocol)?;
                answer["choice"] = json!(choices[selected_index].value);
                answer["confidence"] = a["confidence"].clone();
                answer["probabilities"] = json!(
                    choices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            json!({"value": c.value, "probability": probs.get(&get_key(i))})
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
                for i in 0..levels.len() {
                    if legend.get(&i.to_string()) != Some(&body["questions"][&key]["criteria"][i]) {
                        return Err(ApiError::protocol());
                    }
                }
                answer["score"] = a["score"].clone();
                answer["confidence"] = a["confidence"].clone();
                answer["probabilities"] = json!(levels.iter().enumerate().map(|(i,l)| json!({"value": i, "label": l.label, "probability": probs.get(&i.to_string())})).collect::<Vec<_>>());
            }
        }
        output.push(serde_json::from_value(answer).map_err(|_| ApiError::protocol())?);
    }
    Ok(output)
}

/// 将官方原生响应转换为核心 OpenAI 答案及透明的 usage 兼容扩展。
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
        let token = |key: &str| {
            source
                .get(key)
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .filter(|n| *n >= 0)
                .ok_or_else(ApiError::protocol)
        };
        let total_input_tokens = token("total_input_tokens")?;
        let total_output_tokens = token("total_output_tokens")?;
        let total_tokens = token("total_tokens")?;
        if total_input_tokens.checked_add(total_output_tokens) != Some(total_tokens) {
            return Err(ApiError::protocol());
        }
        let mut usage = json!({
            "input_tokens": total_input_tokens,
            "output_tokens": total_output_tokens,
            "total_tokens": total_tokens,
        });
        let mut decora = json!({
            "provider": "gemini",
            "input_tokens_basis": "total_input_tokens",
            "total_tokens_basis": "total_input_plus_output",
            "details_complete": false,
        });
        if let Some(modality) = source.get("input_tokens_by_modality") {
            let items = modality.as_array().ok_or_else(ApiError::protocol)?;
            for item in items {
                let obj = item.as_object().ok_or_else(ApiError::protocol)?;
                let modality_name = obj
                    .get("modality")
                    .and_then(Value::as_str)
                    .ok_or_else(ApiError::protocol)?;
                if modality_name.is_empty() {
                    return Err(ApiError::protocol());
                }
                let tokens = obj
                    .get("tokens")
                    .and_then(Value::as_i64)
                    .and_then(|n| i32::try_from(n).ok())
                    .ok_or_else(ApiError::protocol)?;
                if tokens < 0 {
                    return Err(ApiError::protocol());
                }
            }
            decora["input_tokens_by_modality"] = json!(items);
        }
        usage["decora"] = decora;
        return DecisionResponse::parse_with_usage(
            json!({"model": req.model, "answers": answers, "usage": usage}),
            req,
            policy,
        );
    }

    let token = |key: &str| {
        source
            .get(key)
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(ApiError::protocol)
    };
    let input = token("input_tokens")?;
    let output = token("output_tokens")?;
    let total = input.checked_add(output).ok_or_else(ApiError::protocol)?;
    if source
        .get("total_tokens")
        .is_some_and(|n| n.as_i64() != Some(i64::from(total)))
    {
        return Err(ApiError::protocol());
    }
    let mut usage = json!({"input_tokens":input,"output_tokens":output,"total_tokens":total});
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
        json!({"model":value["model"],"answers":answers,"usage":usage}),
        req,
        policy,
    )
}
