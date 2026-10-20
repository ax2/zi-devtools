# Super clipboard JSON record export — 2026-10-17

## Goal

Make selected clipboard text useful as structured input to the existing data workbench and other handoff targets without losing the source context that already exists on each history entry.

## Behavior

- Added a separate `JSON记录` composition format. It exports only explicitly selected text entries in selection order as records with `order`, `text`, `source`, `time`, and `labels`.
- The export omits internal history IDs and file paths. Images and file references remain excluded from text composition.
- The normal preview is shown before copy or handoff. Existing 1 MiB input/output limits still apply. Composition itself does not persist data; destination tools retain their own preview and save steps.
- The tool version is `super-clipboard 0.20.3`; application version remains `0.82.0-dev.105`.

## Verification

- `cargo test --offline --locked clipboard::`: 80 passed, 1 ignored; includes the new record shape/order/metadata, output budget, and UI picker preview tests.
- `cargo test --offline --locked`: 1,031 passed, 0 failed, 31 ignored.
- `cargo clippy --offline --locked --all-targets -- -D warnings`: passed.
- `cargo fmt --all --check`, `python scripts/sync_tools.py --check`, and `python scripts/verify_tool_directory.py`: passed.
- Live synthetic UI handoff smoke: `cargo run --offline --locked --features ui-preview --example capture_ui -- target/clipboard-json-records-handoff-smoke 288 clipboard-json-records-handoff-smoke` passed. It clicked the recommended JSON table route and explicit apply action, verified all three ordered records and their source/time/labels in the new data instance, and confirmed the previous draft/export and clipboard source stayed intact.
- Rendered screenshot captured at 1600×1125 in dark theme and visually inspected: the source summary, scrollable JSON preview and handoff controls remain readable. Preserved as `screenshots/clipboard-json-records-handoff-20261017/handoff-discovery-dark.png` (SHA-256 `B069A65D2050A9AC6B94C3E0358E3342106E54725E53BA7667489B0A8BD0D106`).
- After adding the handoff smoke, `cargo test --offline --locked` passed (1,031 passed, 31 ignored), `cargo clippy --offline --locked --all-targets -- -D warnings`, `cargo fmt --all --check`, `git diff --check`, `python scripts/sync_tools.py --check`, and `python scripts/verify_tool_directory.py` passed.
- `cargo build --release --offline --locked`: passed in 16m24s; produced `target/release/ZiDevTools.exe` (40,796,160 bytes, timestamp 2026-10-17 19:50:58).

## Limits and follow-up

This change adds a structured export format, not an automatic extraction agent. It includes the source application filename, displayed copy time, and labels only when the user selects `JSON记录`; inspect the preview before handoff. The synthetic smoke verifies the clipboard-to-JSON-table route; it does not prove the full super clipboard is complete or validate capture from a real external application and Windows clipboard session.
