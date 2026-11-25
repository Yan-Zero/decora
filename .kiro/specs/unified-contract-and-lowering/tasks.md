# Tasks: Unified External Decisions Contract and Built-in Role/JSON/Typed-Choice Lowering (`unified-contract-and-lowering`)

- [x] 1. Remove `NativeInputPolicy` and the `messages / json_input / typed_choices` capability switches from `src/config.rs` and `config.example.yaml`.
- [x] 2. Implement the unified `textual` JSON envelope encoding and the collision-free mixed-type `choice_key` mapping in `src/adapter.rs`, removing the `unsupported_textual_lowering` branch.
- [x] 3. Ensure in `src/adapter.rs` that real `input_image` is filtered independently before encoding, without mistaking same-named fields inside `input_json.data`.
- [x] 4. Add regression tests in `tests/offline.rs` for all three upstreams returning identical core `answers` for the exact same structured OpenAI request, collision edge cases, and 400 responses for illegal upstream request shapes.
- [x] 5. Update `README.md`, `docs/PROTOCOL.md`, and `HANDOFF.md`, and run `fmt / check / test / clippy / audit` with everything passing.