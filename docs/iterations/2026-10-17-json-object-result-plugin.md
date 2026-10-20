# JSON object result plugin — 2026-10-17

## Scope

Added a JSON object summary operation to the standalone JSON tool and a separate provider candidate for the vendored `result.json.v1@1.0.0-preview.1` profile. The provider exposes `devtools.json.object.inspect` and `devtools.json.object.receive`; each declares the whole-output schema and opts into `application/json` capture. The receiver accepts a required object-valued `value` field. No Studio checkout is used as a build dependency; the profile and source metadata are vendored under `contracts/studio-result-json/1.0.0-preview.1/`.

## Behavior and limits

The standalone tool preserves existing JSON format, minify, and validate actions, adding object summary as action four. Summary output includes the original object, root key count/list, value types, maximum nested depth, and compact UTF-8 serialized size. The provider rejects duplicate keys, non-object roots, extra fields, malformed envelopes, values over 8 KiB, more than 128 root keys, depth over 64, more than 4096 nodes, requests/results over 48 KiB, and unsupported capabilities. Plugin-compatible standalone mode routes the summary through the same object WASI adapter and applies the strict JSON parser; default standalone JSON behavior stays on its existing path.

## Evidence

- `cargo test --offline --locked -p zi-json-core -p zi-json-object-wasi --lib`: 11 passed.
- `cargo test --offline --lib tools::tests::plugin_mode_preserves_legacy_and_tool_drafts`: passed.
- Actual `wasm32-wasip1` module and native adapter: 10 requests produced byte-identical responses; object values survived inspect and receive unchanged; duplicate, shape, 8 KiB, 128-field, and 48 KiB limits were exercised.
- Official Wasmtime C-API 36.0.2: all 10 exact cases passed with 10,000,000 fuel, 64 MiB guest memory, 2 MiB stack and 5-second deadline; maximum measured fuel was 563,526.
- Ten fresh-process Wasmtime checks passed; slowest was 475.680 ms.
- Unsigned candidate: `release/plugin-candidates/json-object-0.1.0-candidate-07`; 37 files, 328,768 bytes; directory index SHA-256 `51e2e1c9892ef6aff9ef848735007d0bf1cefd7fbb5ada2e9020952458246f17`. External receipt: same path with `-verification.json`. Candidate-07 preserves the package bytes of Candidate-06 while refreshing the external source-file hashes after the canonical tool catalog changed. The package exposes only the summary capability as an optional Pi tool; the receiver remains available only to confirmed Host transfer. The packaged portable replay script ran all ten WASI requests and matched stored expected bytes.
- Catalog/map sync and Studio result profile are candidates only; `acceptedCapabilities` stays 0.

## Remaining validation

Studio must verify installation/profile negotiation, an actual JSON capture and reference, receiver search/preview/confirmed execution, object type preservation, save/readback, scope/confirmation/schema/size failures, permission revocation, cancellation and plugin failure isolation. A Studio Host input-schema compatibility issue with `maxProperties` is under resolution; keep the provider's bounded object limits intact and rerun Host acceptance after the Host supports the keyword. This candidate is unsigned and does not claim Host acceptance. Candidate-01 is an incomplete packaging attempt produced while fixing malformed vendored `SOURCE.json`; it has no receipt and must not be used. Candidate-02 through Candidate-06 are earlier complete provider packages; Candidate-07 is the current handoff candidate and includes the four-field contract envelope required by the actual Worker protocol. No app version, installer, ZIP, tag or public release was created.
