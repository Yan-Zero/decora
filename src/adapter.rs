//! Official native protocol mapping: preserve genuinely available usage by default; strict mode rejects differences in counting basis or required details.
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

/// Check administrator declarations and adapter-specific limits; evaluate using the OpenAI inbound dialect by default.
pub fn compatible(
    backend: &Backend,
    req: &DecisionRequest,
    need: &Requirements,
    usage_policy: UsagePolicy,
) -> Result<(), ApiError> {
    compatible_for_dialect(Dialect::Openai, backend, req, need, usage_policy)
}

/// Check candidate backend capability boundaries for the inbound dialect; same-dialect native passthrough is also allowed under strict usage.
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
    // Check only actual content parts; do not treat fields with matching names in input_json.data as images.
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
        // Rename all choices for Gemini or groups containing boolean choices to avoid non-ASCII/empty-string restrictions and string-key collisions.
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
    // JSON escaping makes the data boundary explicit but does not claim to prevent model-level prompt injection.
    json!(
        json!({"decora_encoding":"openai_decisions_text_v1","kind":kind,"data":value}).to_string()
    )
}
/// Automatically convert a canonical intermediate request to the upstream's native body; this conversion is an internal gateway implementation, not a client API.
pub fn native_body(req: &DecisionRequest, model: &str) -> Result<Value, ApiError> {
    native_body_for(req, model, BackendKind::Openrouter)
}

/// Build a request body using the provider's native protocol.
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
        // Public names may be repeated or omitted; internal keys are stable and collision-free, and original names are restored only in the final answers.
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

/// Reorder native answers and convert them to the canonical intermediate answer shape; preserve probabilities and confidence unchanged.
pub fn native_answers(value: &Value, req: &DecisionRequest) -> Result<Vec<Answer>, ApiError> {
    native_answers_for(value, req, BackendKind::Openrouter)
}

/// Reorder and convert answers according to the backend kind.
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

/// Convert an official native response to core answers and a transparent usage compatibility extension.
/// input/output come directly from the upstream; total is their sum under an explicitly marked basis, and unknown details are not filled with zeros.
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
