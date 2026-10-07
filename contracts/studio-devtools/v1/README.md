# ZiDevTools plugin contract snapshot

Contract: zicode.devtools-plugin/1.0.0-rc.1. Status: pilot contract, integration pending.

Copy this complete directory as a pinned vendor contract in the producer repository.
No runtime/build dependency on the Studio source tree is permitted. Check SHA256SUMS.json.
SPEC.md is the versioned copy of docs/plugins/devtools-interoperability.md.

- plugin.example.json: manifest template; actual runtime/tools.wasm and views/main.html
  must be supplied by the producer. This directory is not an installable plugin.
- input.schema.json / result.schema.json: strict tool payloads, plus SPEC.md byte limits.
- delivery.schema.json / catalog.example.json: signed catalog metadata; replace placeholders.
- fixtures.json: deterministic tool cases; generator fields must be expanded by the harness.

Run each fixture through both standalone-core and the actual built WASI adapter. Record
results in verification.json with compiler versions, command exits, platform and package hash.
Testing only the core is insufficient. Host installation, iframe, permission and Pi Broker
acceptance remain Studio-owned checks. Unsupported profiles must remain blocked.

Do not publish signing keys or copy local app data. Package through an independently
implemented/vendored versioned packaging tool following SPEC.md and package-format.md;
do not call scripts from a sibling checkout at build time. Preserve MIT notices when reusing code.
