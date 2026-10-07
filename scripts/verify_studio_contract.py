"""Verify the vendored contract without reading another repository."""
from pathlib import Path
import hashlib
import json

ROOT = Path(__file__).resolve().parents[1]
SNAPSHOT = ROOT / "contracts/studio-devtools/v1"
INDEX_SHA256 = "dd0688c009fa2d0600ab2d5af038a19efb5e1e26910619c1aede84aa668873f9"
index_bytes = (SNAPSHOT / "SHA256SUMS.json").read_bytes()
assert hashlib.sha256(index_bytes).hexdigest() == INDEX_SHA256, "Contract index changed"
index = json.loads(index_bytes)
assert len(index) == 10
for name, digest in index.items():
    assert Path(name).name == name and not (SNAPSHOT / name).is_symlink()
    assert hashlib.sha256((SNAPSHOT / name).read_bytes()).hexdigest() == digest, name
fixtures = json.loads((SNAPSHOT / "fixtures.json").read_bytes())
assert fixtures["contract"] == "zicode.devtools-plugin/1.0.0-rc.1"
assert len(fixtures["cases"]) == 16
manifest = json.loads((SNAPSHOT / "plugin.example.json").read_bytes())
assert manifest["id"] == "com.zicode.devtools.text"
assert len(manifest["contributes"]["capabilities"]) == 5
print("Contract snapshot verified: 11 files, 16 vectors, 5 pilot capabilities (not runtime acceptance)")
