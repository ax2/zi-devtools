# Structured JSON result capture (`result.json.v1`)

Status: Studio Host candidate, 2026-10-17. This is an opt-in extension for local WASI capabilities. It does not change the frozen DevTools packages, the default text result contract, or the plugin View SDK.

## Provider declaration

A schema-version-2 manifest declares both exact Host profiles:

```json
{
  "requiredProfiles": {
    "output.schema.v1": "=1.0.0-rc.1",
    "result.json.v1": "=1.0.0-preview.1"
  },
  "runtime": { "wasi": "runtime/tools.wasm" },
  "contributes": {
    "capabilities": [{
      "id": "example.structured",
      "inputSchema": { "type": "object" },
      "outputSchema": { "type": "object" },
      "resultCaptureMimeType": "application/json"
    }]
  }
}
```

The normal capability output must succeed with `ok: true` and an object-valued `data`. The declared `outputSchema` is checked on the whole output before capture. An omitted profile, unsupported MIME, missing schema, non-WASI runtime, business error, non-object `data`, or exceeded budget cannot create a JSON result reference. A capture failure leaves the original tool response available and returns `resultCaptureError`; it does not turn a successful computation into a failed computation.

Only the named capability opts in. A capability without `resultCaptureMimeType` retains `data.text` capture and `resultRef` schema version 1 with `text/plain`. A JSON capability returns `resultRef` schema version 2 with `application/json`, a Host-generated opaque `resultId`, UTF-8 byte length, SHA-256 content version and package version. The reference is not an authorization token.

## Studio-owned result actions

The trusted parent, not the plugin iframe, can request save or transfer with the captured reference. The Host keeps the original object in a 15-minute bounded result store; serialization is canonical for the same in-memory value and limited to 64 KiB per result, 64 records and 2 MiB total. Save preview shows JSON text and MIME. After explicit confirmation, the new artifact has `application/json` MIME and object-valued content. Existing grant scope, 120-second expiry, proof/version checks, `storage.plugin` permission, write gate, save-status and replay handling still apply. No arbitrary file path, overwrite or implicit export is added.

Receiver search lists only local authorized WASI tools whose top-level input schema has an **object** field that accepts the captured object. Other required fields may be bounded strings entered in the trusted UI; missing, extra or type-invalid values are rejected. The mapping preserves the object as an object. No automatic object-to-text conversion, schema defaults or hidden input values are allowed. Input object serialization is limited to 8 KiB and the complete invoke envelope to 48 KiB. Prepare previews the exact mapping and versions; only explicit confirmation executes the receiving Worker. Cancellation and same-Host transfer-status retain their existing behavior. Text results continue to map only to string fields.

This profile does not define arrays, primitive JSON results, JSONPath extraction, streaming, persisted workflows, remote transfer or schema migration. A separate versioned contract and conformance run are required for those shapes. In particular, an object field's acceptance must be checked against the receiver schema before a transfer is offered, and again at prepare/commit.

## Provider conformance request

DevTools may add this declaration to a new independently versioned plugin build after it has a tool that returns bounded object-valued `data` and a matching output schema. Supply a receiver WASI capability with an object input field, plus valid/invalid output examples. Studio needs to verify: install and profile negotiation; discovery and on-demand load; one real JSON invocation; object-type preservation through target search, preview, confirmed Worker execution and result display; confirmed JSON artifact save and readback; wrong scope, missing confirmation, schema mismatch, size excess, permission revocation, cancellation and a failing plugin that leaves another tool usable. Provider changes belong in the DevTools development session; Studio does not modify that repository.

Current evidence is static checks, unit tests and existing text native flows. No real DevTools package declares `result.json.v1` yet, so this document is a candidate interface, not full JSON native acceptance.
