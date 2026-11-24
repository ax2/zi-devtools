# JSON Schema for ZiCode Studio

Paste a JSON instance and a Draft 4, 6 or 7 Schema. Schema without a dialect declaration uses Draft 7. Current-document fragments are supported. Each UTF-8 input can contain up to 8 KiB. The tool displays up to 100 masked errors with JSON Pointer paths, truncating earlier for long reports. It requests no file, network or model access.

Draft 2019-09, Draft 2020-12, external references, root Schema files and the standalone size allowances remain outside this plugin candidate. Verify the external receipt before installing the unsigned directory.

## Reproduce

```powershell
$env:CARGO_PROFILE_PLUGIN_OPT_LEVEL = "z"
cargo rustc --locked --profile plugin --target wasm32-wasip1 -p zi-json-schema-wasi -- -C link-arg=--compress-relocations
```

Run `scripts/verify_json_schema_plugin.py` with the resulting module and an existing Windows production Worker, then give the exact module and verification record to `scripts/package_json_schema_plugin.py`.
