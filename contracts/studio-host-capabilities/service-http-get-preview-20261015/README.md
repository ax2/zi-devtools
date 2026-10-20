# `service.http.get.v1@1.0.0-preview.1` provider snapshot

This is a Zi DevTools working snapshot of Studio's draft `docs/plugins/service-http-get-preview.md`, read on 2026-10-15. It is not a frozen Studio contract and does not authorize a plugin to declare or require this profile. The source document SHA-256 is recorded in the provider verification receipt. This folder exists so the proposed View adapter, schemas, and fixtures can be reviewed without importing a sibling checkout at build time.

## Supported request slice

The public SDK proposal is:

```js
window.zicode.services.http.get(
  { origin: "https://example.invalid", endpoint: "health" },
  { signal },
)
```

Only `health`, `info`, and `metrics` map to fixed `/actuator/health`, `/actuator/info`, and `/actuator/metrics` paths. The caller supplies an origin only. Userinfo, a base path, query, fragment, arbitrary endpoint, method, header, body, credential, proxy, or redirect is outside the profile. The Host owns plugin/runtime/session identity, permission checks, one-use authorization, confirmation, DNS pinning, network access, cancellation, resource slots, response budget, and auditing. The View must have no direct network fallback and should use `connect-src 'none'`.

The draft budgets are up to 3 seconds to connect, 8 seconds total, and 1 MiB of response bytes. Public destinations require HTTPS. HTTP is limited to explicitly confirmed loopback or RFC1918 destinations. System TLS verification is required. Only JSON (`application/json` or `+json`) is accepted. Health 503 with JSON remains diagnostic data.

The draft does not specify a complete JSON Schema for SDK responses or a stable error-code enumeration. The provider-side `sdk-result.schema.json` and fixtures in this folder therefore define only a proposed View-facing shape and examples; Studio conformance must settle any mismatch before the profile can be used.

## Current compatibility gaps

Studio labels the profile “not implemented or supported” and explicitly says providers must not declare it in a plugin manifest. No installable plugin package in this snapshot requires it. The existing standalone `spring-actuator` tool additionally supports optional Bearer authentication, arbitrary `metrics/<name>` selection, and a response/time budget larger than this profile. Those behaviors cannot be represented by this preview; do not claim full standalone parity.
