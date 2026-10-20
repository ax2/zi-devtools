# Report privacy allow-list

The proposed report is a general summary only. Include only the following fields:

- endpoint ID (`health`, `info`, or `metrics`), numeric HTTP status and response byte length;
- count of top-level JSON fields, not field names or values;
- health's normalized status enum (`UP`, `DOWN`, `OUT_OF_SERVICE`, `UNKNOWN`) when present;
- source tool ID/version, application version and a SHA-256 fingerprint over portable runtime fields (OS family, CPU architecture, build kind and application version).

Never include the origin/hostname, context path, endpoint URL, DNS addresses, raw response body, Actuator `info` values, metric names/values, health component details, auth headers, cookies, Host grants/tokens, raw network errors, local file paths, machine/user names or arbitrary plugin metadata. Fingerprinting uses a documented canonical JSON tuple and must not include machine-specific values. The workbench may separately hold a user-confirmed full response capture under its own retention and access controls; this export report does not duplicate it.
