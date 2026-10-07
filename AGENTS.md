# Zi DevTools contributor instructions

- Windows-first Rust desktop app; keep independent of company projects.
- MIT licensed. Do not commit credentials, user configuration, backups or build outputs.
- Keep docs/tools.json current with source; run python scripts/sync_tools.py and --check.
- Run cargo fmt --check, cargo test --locked, cargo clippy --all-targets --all-features --locked -- -D warnings and cargo build --release --locked.
- Use disposable service fixtures for tests; never stop unrelated processes.
- Release workflow and installation instructions: docs/releasing.md. No ZIP release assets.
- Update product documentation and the website catalog snapshot with each release.

## Independent Studio plugin delivery

- Keep the standalone native Rust UI and EXE; plugin Views may use self-contained HTML and only the versioned public Host SDK.
- Share pure Rust algorithms between standalone and wasm32-wasip1; verify identical strict-mode vectors through actual native and WASI adapters.
- Build only from this repository and its vendored contract snapshot. No sibling checkout imports, shared user databases, EXE delegation or local HTTP workarounds.
- Track every catalog tool migration separately from standalone implementation. Candidate is not Host acceptance.
- Without an authorized signing key, deliver an unsigned directory with external verification; do not publish a signed-container claim.
- Reuse existing Cargo toolchain/cache/target and preserve unpacked native release directories.
