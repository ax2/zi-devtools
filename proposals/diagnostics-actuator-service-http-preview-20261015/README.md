# Spring Actuator service profile View adapter (review bundle)

This is a **review-only, not-installable** bundle for Studio's proposed `service.http.get.v1@1.0.0-preview.1`. The profile is not implemented or supported. The bundle deliberately has no root plugin manifest and the current diagnostics manifest copied under `plugins/diagnostics/` does not require this profile.

The View calls only `window.zicode.services.http.get({origin, endpoint}, {signal})`. It has no browser network fallback, WASI network access, credentials, arbitrary path, headers, or request body. Its local validation and schemas are provider proposals. The Host must perform all authoritative URL/DNS/IP checks, user confirmation, one-use authorization, redirect/proxy/TLS policy, response limits, and cancellation.

## Known incompatibilities with standalone Spring Actuator 1.1.1

- Studio's profile is unimplemented and must not be declared in an installable manifest. This bundle is not an integration acceptance result.
- Only `/actuator/health`, `/actuator/info`, and `/actuator/metrics` are fixed endpoints. A named `metrics/<name>` query is not supported.
- The profile rejects all credentials. Optional Bearer authentication in the desktop tool is outside this adapter.
- Origin-only input excludes a Spring context path or custom Actuator base path.
- Studio's 3 s connect / 8 s total / 1 MiB response budgets are tighter than the desktop tool's 5 s / 20 s / 2 MiB limits.
- Studio has not frozen the response schema or stable error-code set. The included SDK result schema is a provider proposal only.

## Evidence

`verification/desktop-proof.json` and `verification/mobile-proof.json` record Edge runs at 1180×900 light and 390×844 dark, DPR 1, six SDK-shaped mock calls each, and zero page errors. Screenshots are hash-bound in the source receipt. Tests cover health 200/503, info/metrics, malformed origins, unsupported endpoint, error-code handling, AbortSignal, and suppression of a late result. No HTTP server, DNS, TLS, proxy, Host permission, or confirmation was exercised.

Run the provider View tests from this bundle with Edge or Chrome:

```powershell
python scripts/verify_actuator_view.py --width 1180 --height 900 --theme light --output target/desktop-proof.json --screenshot target/desktop.png
python scripts/verify_actuator_view.py --width 390 --height 844 --theme dark --output target/mobile-proof.json --screenshot target/mobile.png
```
