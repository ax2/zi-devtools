# AGENTS.md

## Project contract

- This is a Windows-first, Rust-only desktop application. UI and service-management logic must remain in Rust.
- Keep the application standalone. Do not add runtime, source, configuration, package, or build dependencies on company projects.
- The local-service YAML model and process-management code belong to this repository. Compatibility with an external project's private paths or configuration is not a product requirement.
- Never copy credentials, tokens, cookies, private keys, or complete environment files into source, logs, tests, screenshots, or the technical archive.
- Windows stage releases retain an unpacked runnable directory and EXE. Do not create ZIP archives unless explicitly requested.
- Before a stage release, update the source README and the long-term archive at `C:\Users\ax2\Documents\技术博客\zi-devtools`.

## Verification

- Run `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release`.
- Exercise service start/stop with a disposable fixture instead of a production or credential-dependent service.
- Record evidence and known limitations in the current Stage work log.

## Tool catalog and public release

- Keep docs/tools.json as the canonical implemented/planned catalog. Update status, scope, version and date with every tool change.
- Run python scripts/sync_tools.py and python scripts/sync_tools.py --check. Only mark implemented after validation and a working UI entry.
- Follow docs/releasing.md. Publish only curated source/product docs; never publish local backups, logs, user config, build caches or historical archives.
- Sync the landing page catalog snapshot on release; the website reads the same public catalog for current status.

## Independent Studio plugin delivery

- Keep the standalone native Rust UI and EXE; plugin Views may use self-contained HTML and only the versioned public Host SDK.
- Share pure Rust algorithms between standalone and wasm32-wasip1; verify identical strict-mode vectors through actual native and WASI adapters.
- Build only from this repository and its vendored contract snapshot. No sibling checkout imports, shared user databases, EXE delegation or local HTTP workarounds.
- Track every catalog tool migration separately from standalone implementation. Candidate is not Host acceptance.
- Without an authorized signing key, deliver an unsigned directory with external verification; do not publish a signed-container claim.
- Reuse existing Cargo toolchain/cache/target and preserve unpacked native release directories.
