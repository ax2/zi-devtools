# Studio model service preview snapshot

This folder vendors the public documentation snapshots used by the standalone DevTools provider candidate. It is data-only: DevTools does not import Studio source code, build configuration, user data, credentials, executables, or databases.

Origin: `zicode-studio-desktop/docs/plugins/model-catalog-preview.md` and `model-generation-preview.md`, read on 2026-10-14. Their SHA-256 digests are recorded in `SHA256SUMS.json`. The locally generated plugin manifest requires the exact preview profile strings from this snapshot.

The preview is desktop View-only. It does not enable `service.v1`, MCP, file access, provenance verification, or in-flight cancellation. See [model-generation-preview.md](model-generation-preview.md) for the current limits. Studio native acceptance is outside this provider snapshot and remains pending.
