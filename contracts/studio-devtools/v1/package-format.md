# Portable signed package format v1

The plugin is a ZIP container with extension .zicode-plugin. It contains plugin.json,
runtime/tools.wasm, views/main.html, the tool catalog/schemas/licenses and a reserved
root package.json signature manifest. Do not place an npm package.json at this root.
Container maximum: 50 MiB. Payload maximum: 2048 files, 50 MiB total, 10 MiB each;
signature manifest maximum: 512 KiB. WASI maximum remains 2 MiB.

package.json fields in this serialization order:

```json
{
  "schemaVersion": 1,
  "publisher": {
    "name": "zicode",
    "keyId": "SHA256_HEX_OF_RAW_PUBLIC_KEY",
    "publicKey": "BASE64_RAW_32_BYTE_ED25519_PUBLIC_KEY"
  },
  "files": [
    { "path": "plugin.json", "sha256": "SHA256_HEX_OF_EXACT_FILE_BYTES", "size": 123 }
  ],
  "signature": "BASE64_ED25519_SIGNATURE"
}
```

The values above are placeholders, not usable trust or signature data. List every payload
file exactly once, excluding package.json itself. Sign the UTF-8 compact JSON object with
fields schemaVersion, publisher, files in that order (no signature field, BOM or trailing
newline). Publisher fields order is name, keyId, publicKey. File fields order is path,
sha256,size. Keep that file-list order in the final manifest. Sort file paths deterministically;
the verifier uses the supplied list order. Strings use JSON escaping, UTF-8 non-ASCII text,
no slash escaping, and sizes are integer numbers. Test signer output against Studio's verifier.

Paths use relative forward slashes, no dot/dotdot segments, empty segments, colon, backslash,
absolute path, symlink, alternate stream or duplicate entry. Use portable ASCII filenames for
this profile. Reject .env files, private keys, PFX/P12/PEM, source maps and nested packages.
No executable launch hooks, native DLLs or EXEs. Runtime module is referenced only via runtime.wasi.

Publisher trust is user-managed; a valid signature does not grant trust or permissions.
Unsigned candidate directories are for local development only. A signer may be implemented
in the producer's Rust repository using these bytes, or vendored from a pinned MIT source with
notices; it may not import the Studio checkout. Do not hard-code signing keys.

Final package hash goes into an adjacent .sha256 and external verification.json. Internal
provenance must never refer to its enclosing package hash. Same plugin ID/version must not be
reissued with different bytes. New versions state migrations and compatible rollback behavior.
