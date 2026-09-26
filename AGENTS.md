# Zi DevTools contributor instructions

- Windows-first Rust desktop app; keep independent of company projects.
- MIT licensed. Do not commit credentials, user configuration, backups or build outputs.
- Keep docs/tools.json current with source; run python scripts/sync_tools.py and --check.
- Run cargo fmt --check, cargo test --locked, cargo clippy --all-targets --all-features --locked -- -D warnings and cargo build --release --locked.
- Use disposable service fixtures for tests; never stop unrelated processes.
- Release workflow and installation instructions: docs/releasing.md. No ZIP release assets.
- Update product documentation and the website catalog snapshot with each release.
