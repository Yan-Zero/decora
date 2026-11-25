# Design: Unified External Decisions Contract and Built-in Role/JSON/Typed-Choice Lowering (`unified-contract-and-lowering`)

## 1. Remove the transformation control surface (`src/config.rs` & `src/gateway.rs`)

- Delete the `NativeInputPolicy` enum and `Config::native_input_policy`.
- Delete the `messages`, `json_input`, and `typed_choices` boolean switches in `Capabilities`.
- `Gateway::execute` calls the unified internal transformation functions directly for non-OpenAI backends, with no policy parameter.

## 2. Internal text envelope and option key remapping (`src/adapter.rs`)

- **Real capability pre-validation (`validate_native_request`)**:
  - Iterate over the message parts of `req.input` and the parts of `req.questions[i].instructions`; only when the outer content block has `part["type"] == "input_image"` is it treated as a real image and `unsupported_native_images` returned (same-named keys inside `input_json.data` are ignored).
  - Check `prompt_cache_breakpoint`, `safety_identifier`, `prompt_cache_key`, and `prompt_cache_options`; return `unsupported_native_mapping` if any is present.
- **Structured input encoding (`textual`)**:
  - If `value` is a JSON string, clone it as-is;
  - If `value` is an array, serialize it as `json!({"decora_encoding":"openai_decisions_text_v1","kind":kind,"data":value}).to_string()`.
- **Mixed-type option keys (`choice_key`)**:
  - If the option group contains any `ChoiceValue::Bool(_)`, the key for item `i` is fixed to `format!("decora_choice_{i}")`, and `criteria` writes `{"value": choice.value, "description"?: ...}`;
  - If all are strings, use the original string directly as the key.
  - In `native_answers`, locate the selected item via `(0..choices.len()).find(|i| choice_key(choices, *i) == selected)` and restore the original `ChoiceValue`.