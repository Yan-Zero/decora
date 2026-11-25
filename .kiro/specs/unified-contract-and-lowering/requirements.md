# Requirements: Unified External Decisions Contract and Built-in Role/JSON/Typed-Choice Lowering (`unified-contract-and-lowering`)

## Background and Goals

Protocol translation is an internal responsibility of the gateway: the client should not be aware of whether the upstream is OpenAI, OpenRouter, or TypeSafe/Jev, and should not configure or switch any "input transformation policy". The goal of this spec is to fix a single external OpenAI Decisions request and core answer contract, automatically perform lossless encoding and restoration of role messages, `input_json`, and mixed boolean/string `choice` inside the gateway, and remove all transformation policy control surfaces.

---

## Requirements

### Requirement 1: Fix a single external request contract and remove transformation policy switches
- **Acceptance Criteria 1.1**: All public model aliases accept only standard OpenAI `POST /v1/decisions` requests (`model / input / questions`); if a client submits upstream-proprietary `state`, `criteria`, a named `questions` object, or any transformation policy field, return `400 invalid_request` without making an upstream call.
- **Acceptance Criteria 1.2**: Completely remove `native_input_policy` and the `capabilities.messages / json_input / typed_choices` switches from the server configuration; legacy configuration files that still contain these fields are explicitly rejected by `deny_unknown_fields`.

### Requirement 2: Lossless internal encoding of role messages and structured content
- **Acceptance Criteria 2.1**: The native OpenAI backend keeps the original `input` and `questions` structure unchanged.
- **Acceptance Criteria 2.2**: The OpenRouter/Jev backend passes string `input` and string `instructions` through unchanged; role message arrays and `input_text / input_json` content arrays are automatically encoded as a single-string JSON envelope `{"decora_encoding":"openai_decisions_text_v1","kind":"messages"|"question_content","data":...}`, preserving all four legal roles, order, `phase`, empty content, Unicode, and nested JSON.
- **Acceptance Criteria 2.3**: Explicitly state in the documentation that this text encoding preserves only role labels and data boundaries; it is not equivalent to the upstream natively executing the OpenAI role instruction hierarchy, nor is it prompt injection isolation.

### Requirement 3: Collision-free restoration of mixed boolean and string `choice`
- **Acceptance Criteria 3.1**: When a `choice` option group contains booleans (e.g., containing `true`, `"true"`, `false`, and `"false"` at the same time), switch the whole group to internal keys `decora_choice_0..N-1`, and keep the original JSON-typed `value` and optional `description` in `criteria`.
- **Acceptance Criteria 3.2**: During response parsing, accurately restore the original typed `value` of the selected `choice` and of items in `probabilities` using the internal index keys; boolean `false` is not treated as a default value, and no collision occurs even if the client provides a string option named `"decora_choice_0"` or an empty-string option.

### Requirement 4: Real multimodality and capability boundaries remain independently filtered
- **Acceptance Criteria 4.1**: Real `input_image` (whether in the shared `input` or in question `instructions`) is checked independently before any text encoding, excluding OpenRouter/Jev candidates that do not support native images; an image URL is never encoded into text to pretend the image has been analyzed.
- **Acceptance Criteria 4.2**: `"image_url"`, `"type"`, and `"role"` keys appearing inside `input_json.data` are treated as ordinary JSON data only, and do not trigger image interception or message role forgery.