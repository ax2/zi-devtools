# Spring Actuator plugin candidate 0.1.1 — 2026-10-16

## Objective and result

Prepared an unsigned, review-only Studio plugin directory for source tool `spring-actuator` 1.1.1. The candidate covers only fixed health/info/metrics HTTP GET and summary display; the standalone tool's full scope remains incomplete in the plugin. The app remains at its existing `0.82.0-dev.105` version. Plugin candidate version advanced from the earlier immutable 0.1.0 snapshot to 0.1.1 after the schema and confirmation contract changed.

The background task proposal now follows Studio's draft `plugin.task.lifecycle.v1@1.0.0-preview.1`: opaque Host-owned `taskRef`, separate durable `idempotencyKey` and bridge `requestId`, five proposed `/v2/plugin-tasks/*` routes, Host-injected identity, registered handlers only, snapshot/replay, cancellation finality, restart interruption, revocation, and independent result-resource handling. `confirmationId` is Host-generated and bound to task, step, target, expiry and permission generation; negative vectors cover missing, stale, mismatched and replayed IDs with zero duplicate requests. The task profile is not declared by the manifest.

## Verification

- Contract verification uses the project-local `target/plugin-verification-deps` environment, installed from `plugins/spring-actuator/scripts/requirements-verification.txt` (`jsonschema[format]==4.26.0`). Run with `PYTHONPATH=target/plugin-verification-deps` and `PYTHONDONTWRITEBYTECODE=1` so the candidate stays free of bytecode. `verify-contract.py`: PASS; 32 strict JSON documents, all 22 schemas checked with the Draft 2020-12 validator and format checker, 6 local references, 21 schema vectors, 10 cursor schema/semantic vectors, 7 event schema/terminal vectors, 8 confirmation binding/expiry/replay vectors, 10 report-resource semantic vectors, duplicate-key fixture rejected. This is standard schema validation plus provider semantic fixtures, not Studio runtime conformance.
- `node plugins/spring-actuator/scripts/test-view.cjs`: PASS; 4 successful response cases, decline/profile handling, 6 host-error cases, 9 input guards, 3 canonical-origin cases, cancellation waits for Host commit finality.
- `python scripts/sync_tools.py --check`: PASS; 91 implemented and 63 planned/in-progress catalog records.
- `python scripts/sync_studio_plugin_map.py --check`: PASS; 154 mapped catalog entries and 0 accepted capabilities.
- Candidate copy was independently tested; `python scripts/verify_spring_actuator_candidate.py` verifies every file hash and directory index. The current candidate-02 receipt records 42 files / 130,016 bytes and excludes `__pycache__`/`.pyc`.

## Artifacts and limits

- Candidate: `release/plugin-candidates/spring-actuator-0.1.1-candidate-02/` (generated after excluding Python bytecode cache; exact inventory is in the receipt).
- External SHA-256 receipt: `release/plugin-candidates/spring-actuator-0.1.1-candidate-02-verification.json`.
- Source revision recorded in the receipt: `65f9aa63c831a4540bdd5731aa4da3de76dbd4b8`; source tree was dirty. The receipt records per-tool catalog diffs against that revision.
- No signed `.zicode-plugin` was created because no authorized publisher key was provided. `packageSha256` is `null`.
- Studio's task and result/resource profiles remain unimplemented/unpublished drafts. The HTTP View candidate has not been exercised through real Pi→Host→View, system TLS roots, or public DNS. No Studio acceptance is claimed; the matrix remains 0/154.
