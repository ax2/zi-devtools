# `tool.open.v1` preview profile snapshot

Studio receiver version: `1.0.0-preview.1`. This vendored provider-side summary records the exact contract used by the independent ZiDevTools candidate. It does not include Studio source or make a Host acceptance claim.

## Manifest requirement

The plugin manifest uses schema version 2 and requires the exact profile:

```json
{
  "schemaVersion": 2,
  "requiredProfiles": {
    "tool.open.v1": "=1.0.0-preview.1"
  }
}
```

The plugin must contribute a `location: "main"` View and at least one declared capability. A host request may name only a capability ID in that same plugin's validated `contributes.capabilities` list. Do not accept an operation name, path, URL, selector, script or arbitrary navigation string.

## View API

After the host opens the main View, the injected public SDK exposes `window.zicode.navigation.onOpenTool(callback)`. The callback receives `{ capabilityId }` and returns exactly one of `opened`, `not_found` or `unsupported`. It may be synchronous or asynchronous and can be unsubscribed with the returned subscription's `dispose()` method.

The callback only selects or focuses the corresponding interface. It must not invoke a capability, grant permission, execute work, write files, start services or transfer input/results. The host acknowledges the active request with `openToolResult` on the current View channel. Missing/throwing/timed-out callbacks are `unsupported`; late results from a destroyed View, old request ID or other scene/workspace cannot satisfy the active request. Host budget is 5 seconds; View budget is 4 seconds. Each plugin has one pending intent; a newer intent replaces it. Scene/workspace changes and View destruction clear the pending intent. A location acknowledgement is not capability authorization or execution success.

## Provider test vectors

| Case | Input | Required View result | Required effect |
| --- | --- | --- | --- |
| Declared capability | `devtools.inspect.color.convert` | `opened` | Select only `color.convert`; invoke count stays zero |
| Unknown capability | `devtools.inspect.not-registered` | `not_found` | Keep current selection; invoke count stays zero |
| Slow handler sample | Declared ID, callback resolves after 4.5 seconds | Host reports `unsupported` after the View's 4-second budget; late acknowledgement is ignored | Do not retry or invoke the tool |

The slow handler is a separate receiver-boundary sample. The production View callback stays immediate and does not include an artificial delay.
