//! 官方原生协议映射。OpenRouter/Jev 的答案映射可测，完整响应因 usage 缺项而禁用。
use crate::{
    config::{Backend, BackendKind},
    error::ApiError,
    protocol::{Answer, ChoiceValue, DecisionRequest, Question, Requirements},
};
use serde_json::{Map, Value, json};

/// 检查管理员声明和 adapter 固有边界；绝不靠切换模型规避不兼容。
pub fn compatible(
    backend: &Backend,
    req: &DecisionRequest,
    need: &Requirements,
) -> Result<(), ApiError> {
    let c = &backend.capabilities;
    if req.questions.len() > c.max_questions
        || req
            .questions
            .iter()
            .any(|q| !c.question_types.iter().any(|t| t == q.kind()))
        || (need.images > 0 && !c.images)
        || (need.json && !c.json_input)
        || (need.messages && !c.messages)
        || (need.cache && !c.prompt_cache)
        || (need.safety && !c.safety_identifier)
    {
        return Err(ApiError::unsupported("unsupported_capability"));
    }
    if backend.kind != BackendKind::Openai {
        // 官方两份 schema 都没有 cache_write_tokens / reasoning_tokens；拒绝在付费调用后才发现。
        return Err(ApiError::unsupported("unsupported_usage"));
    }
    Ok(())
}
/// 将保守文本子集映射到 OpenRouter /api/alpha/decisions 或 TypeSafe /v1/systemone。
/// 此函数供协议验证使用，不代表 adapter 可用于严格 OpenAI API。
/// 角色消息、多模态说明、布尔选项、缓存和安全标识无法保真时明确报错。
pub fn native_body(req: &DecisionRequest, model: &str) -> Result<Value, ApiError> {
    if !req.input.is_string()
        || req.questions.iter().any(|q| !q.instructions().is_string())
        || req.safety_identifier.is_some()
        || req.prompt_cache_key.is_some()
        || req.prompt_cache_options.is_some()
    {
        return Err(ApiError::unsupported("unsupported_native_mapping"));
    }
    let mut questions = Map::new();
    for (index, q) in req.questions.iter().enumerate() {
        let mut question = json!({"type": q.kind(), "instructions": q.instructions()});
        match q {
            Question::Predicate { .. } => question["type"] = json!("noul"),
            Question::Choice { choices, .. } => {
                let mut criteria = Map::new();
                for choice in choices {
                    let ChoiceValue::String(value) = &choice.value else {
                        return Err(ApiError::unsupported("unsupported_boolean_choice"));
                    };
                    criteria.insert(value.clone(), json!(choice.description));
                }
                question["criteria"] = Value::Object(criteria);
            }
            Question::Score { levels, .. } => {
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
        // 公开名称可重复、可省略；内部键稳定且无碰撞，原名称只在最终答案还原。
        questions.insert(format!("q{index}"), question);
    }
    Ok(json!({"model": model, "state": req.input, "questions": questions}))
}
/// 将 native 答案重排并转换为 OpenAI 题型形状；概率及 confidence 原样保留。
/// 调用者仍须用 DecisionResponse::parse 校验概率和与期望。返回值没有 usage，不能独立用作 API 响应。
pub fn native_answers(value: &Value, req: &DecisionRequest) -> Result<Vec<Answer>, ApiError> {
    let body = native_body(req, "schema-validation")?;
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
                answer["choice"] = a["choice"].clone();
                answer["confidence"] = a["confidence"].clone();
                answer["probabilities"] = json!(
                    choices
                        .iter()
                        .map(|c| {
                            let ChoiceValue::String(key) = &c.value else {
                                unreachable!("native_body rejects booleans")
                            };
                            json!({"value": c.value, "probability": probs.get(key)})
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
