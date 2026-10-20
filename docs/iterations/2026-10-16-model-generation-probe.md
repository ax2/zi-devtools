# Model profile generation probe

## Goal

Close the model-profile validation gap: model discovery proves only that a catalog endpoint responds; it does not prove that the selected model can generate a chat response.

## Change

- Added an explicit “测试模型生成” action for OpenAI-compatible and Ollama chat tools that use the supported text-chat request shape.
- The probe always sends the fixed prompt `只回复 OK`. It does not compose the current draft, attachments, or prior conversation, and uses the existing temporary or Windows-vault credential path.
- The existing streaming parser, cancellation controls, bounded response handling, and credential redaction remain in use.
- On completion, restore the draft exactly as it was. Probe output is not committed to conversation history and is excluded from the cross-tool handoff API.
- Bumped `model-profiles` to 0.2.0 and synchronized catalog-derived documentation.

## Validation

- `cargo test --lib`: 902 passed, 27 ignored.
- `cargo test --lib --features ui-preview plugin_ui::tests`: 17 passed, 1 ignored.
- `cargo clippy --all-targets --features ui-preview -- -D warnings`: passed.
- `cargo fmt --check` and `python scripts/sync_tools.py --check`: passed.
- Rendered and inspected the 1600×1125 light-theme model-discovery screen. The probe is visible beside the model controls, shows the fixed-prompt and billing notice, and the capture made no model request.
- Screenshot: `C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-model-generation-probe\model-discovery-light.png`.
- The capture runner continued into later scenes. Its two output directories remain under `target/` (29,789,223 bytes total) because the shell cleanup command was rejected by the automatic safety policy; the representative screenshot is archived separately.

## Limits

No real provider was contacted in validation. The feature supports only the existing OpenAI-compatible and Ollama chat templates; broader provider-specific configuration remains unfinished. A user-triggered remote test may incur provider charges.
