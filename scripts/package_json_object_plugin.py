#!/usr/bin/env python3
"""Assemble a hash-indexed unsigned result.json provider candidate."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from tool_directory import apply_manifest, directory_snapshot, SNAPSHOT
import copy

ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / "release/plugin-candidates"
PROFILE = ROOT / "contracts/studio-result-json/1.0.0-preview.1"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def within(path: Path, parent: Path) -> Path:
    resolved = path.resolve(strict=True)
    root = parent.resolve(strict=True)
    if not resolved.is_relative_to(root):
        raise RuntimeError(f"Path escapes {root}: {path}")
    cursor = resolved
    while cursor != root:
        if cursor.is_symlink():
            raise RuntimeError(f"Linked path rejected: {cursor}")
        cursor = cursor.parent
    return resolved


def write(output: Path, relative: str, data: bytes) -> None:
    target = output / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)


def json_bytes(value) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wasm", type=Path, required=True)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--proof", type=Path, required=True)
    parser.add_argument("--fuel-proof", type=Path, required=True)
    parser.add_argument("--cold-proof", type=Path, required=True)
    parser.add_argument("--engine-provenance", type=Path, required=True)
    parser.add_argument("--cases-dir", type=Path, required=True)
    parser.add_argument("--refresh-proof", action="store_true",
                        help="create the missing external receipt only after exact candidate file verification")
    parser.add_argument("--output", type=Path,
                        default=RELEASE / "json-object-0.1.1-candidate-02")
    args = parser.parse_args()
    wasm = within(args.wasm, ROOT)
    native = within(args.native, ROOT)
    proof_path = within(args.proof, ROOT)
    fuel_path = within(args.fuel_proof, ROOT)
    cold_path = within(args.cold_proof, ROOT)
    engine_path = within(args.engine_provenance, ROOT)
    cases_dir = within(args.cases_dir, ROOT)
    output = args.output.absolute()
    if output.exists() and not args.refresh_proof:
        raise RuntimeError("Candidate already exists; select a new directory and never overwrite prior evidence")
    if args.refresh_proof and not output.is_dir():
        raise RuntimeError("Refresh requires an existing candidate directory")
    if not output.resolve().is_relative_to(RELEASE.resolve(strict=True)):
        raise RuntimeError("Candidate directory must be inside release/plugin-candidates")
    if any(parent.is_symlink() for parent in [output, *output.parents] if parent.exists()):
        raise RuntimeError("Linked candidate path rejected")

    module = wasm.read_bytes()
    proof = json.loads(proof_path.read_text(encoding="utf-8"))
    fuel = json.loads(fuel_path.read_text(encoding="utf-8"))
    cold = json.loads(cold_path.read_text(encoding="utf-8"))
    engine = json.loads(engine_path.read_text(encoding="utf-8"))
    assert proof["wasmSha256"] == sha(module) and proof["wasmBytes"] == len(module)
    assert proof["nativeSha256"] == sha(native.read_bytes())
    assert len(proof["cases"]) >= 10
    assert fuel["wasmSha256"] == cold["wasmSha256"] == sha(module)
    assert fuel["wasmBytes"] == cold["wasmBytes"] == len(module)
    assert fuel["engineProvenance"] == engine and fuel["failures"] == []
    assert cold["engine"]["dllSha256"] == engine["dllSha256"] and cold["remaining"] == 0 and cold["failures"] == []
    case_ids = {row["id"] for row in proof["cases"]}
    assert case_ids == {row["id"] for row in fuel["cases"]} == {row["id"] for row in cold["rows"]}
    for row in proof["cases"]:
        for field, digest_field in (("requestFile", "requestSha256"), ("expectedFile", "expectedSha256")):
            contents = (cases_dir / row[field]).read_bytes()
            assert sha(contents) == row[digest_field]

    manifest = json.loads((ROOT / "plugins/json-object/plugin.json").read_text(encoding="utf-8"))
    catalog = json.loads((ROOT / "plugins/json-object/catalog.json").read_text(encoding="utf-8"))
    required = {"output.schema.v1": "=1.0.0-rc.1", "result.json.v1": "=1.0.0-preview.1", "tool.directory.v1": "=1.0.0-preview.1"}
    assert manifest["schemaVersion"] == 2 and manifest["id"] == "com.zicode.devtools.json-object"
    assert manifest["requiredProfiles"] == required
    assert apply_manifest(copy.deepcopy(manifest)) == manifest
    assert manifest["runtime"] == {"wasi": "runtime/tools.wasm"}
    source_meta = json.loads((PROFILE / "SOURCE.json").read_text(encoding="utf-8"))
    profile_bytes = (PROFILE / "result-json-profile.md").read_bytes()
    assert source_meta["profileId"] == "result.json.v1" and source_meta["profileVersion"] == "1.0.0-preview.1"
    assert sha(profile_bytes) == source_meta["sourceSha256"]
    assert "result.json.v1" in profile_bytes.decode("utf-8")
    caps = manifest["contributes"]["capabilities"]
    assert len(caps) == 2 and all(row.get("resultCaptureMimeType") == "application/json" for row in caps)
    assert all(
        row["outputSchema"]["properties"]["data"]["type"] == "object"
        and row["outputSchema"]["properties"]["contractVersion"]["const"] == "1.0.0-rc.1"
        and row["outputSchema"]["properties"]["ok"]["const"] is True
        and row["outputSchema"]["properties"]["error"]["type"] == "null"
        and row["outputSchema"]["required"] == ["contractVersion", "ok", "data", "error"]
        for row in caps
    )
    pi_tools = manifest["contributes"]["piTools"]
    assert len(pi_tools) == 1 and pi_tools[0]["capability"] == "devtools.json.object.inspect"
    assert "devtools.json.object.receive" not in {row["capability"] for row in pi_tools}
    source_version = next(tool["tool_version"] for tool in json.loads((ROOT / "docs/tools.json").read_text(encoding="utf-8"))["tools"] if tool["id"] == "json")
    assert catalog["tools"][0]["version"] == source_version
    assert catalog["contract"] == "zicode.devtools-plugin/1.0.0-rc.1"
    assert catalog["packageId"] == manifest["id"] and catalog["packageVersion"] == manifest["version"]
    assert {row["capabilityId"] for row in catalog["tools"]} == {row["id"] for row in caps}
    assert all(row["profile"] == "result.json.v1" for row in catalog["tools"])

    git = lambda *cmd: subprocess.check_output(["git", *cmd], cwd=ROOT).decode().strip()
    source_state = {"revision": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}
    catalog["source"] = source_state

    payload = {
        "LICENSE": ROOT / "LICENSE",
        "README.md": ROOT / "plugins/json-object/README.md",
        "plugin.json": ROOT / "plugins/json-object/plugin.json",
        "catalog.json": ROOT / "plugins/json-object/catalog.json",
        "fixtures/result-json-cases.json": ROOT / "plugins/json-object/fixtures/result-json-cases.json",
        "schemas/output.schema.json": ROOT / "plugins/json-object/output.schema.json",
        "contracts/result-json-profile.md": PROFILE / "result-json-profile.md",
        "contracts/SOURCE.json": PROFILE / "SOURCE.json",
    }
    replay_readme = ("# Replaying provider functional cases\n\n"
        "Requires Python 3.10+ and Node.js with WASI Preview 1 support. From this directory run:\n\n"
        "```powershell\npython verification/verify_json_object_wasi.py --output $env:TEMP\\json-object-replay-proof.json\n```\n\n"
        "The packaged script checks the module hash, runs every stored request through the WASI module, and compares exact response bytes to the included expected files. This proves provider behavior only; it does not simulate Studio Host capture, transfer confirmation, save, permissions or installation. Native/WASI and Wasmtime fuel/cold proofs are included as signed-by-hash evidence, not a signature.\n").encode()
    expected_files = {relative: (json_bytes(catalog) if relative == "catalog.json" else path.read_bytes())
                      for relative, path in payload.items()}
    expected_files.update({
        "verification/tool-directory-snapshot.json": json_bytes(directory_snapshot(manifest)),
        "contracts/tool-directory/schema.json": (SNAPSHOT / 'schema.json').read_bytes(),
        "runtime/tools.wasm": module,
        "verification/native-wasi-parity.json": proof_path.read_bytes(),
        "verification/wasmtime-fuel-proof.json": fuel_path.read_bytes(),
        "verification/cold-start-proof.json": cold_path.read_bytes(),
        "verification/engine-provenance.json": engine_path.read_bytes(),
        "verification/verify_json_object_wasi.py": (ROOT / "scripts/verify_json_object_wasi.py").read_bytes(),
        "verification/run_text_wasi.mjs": (ROOT / "scripts/run_text_wasi.mjs").read_bytes(),
        "verification/README.md": replay_readme,
        "verification/verify_json_object_fuel.py": (ROOT / "scripts/verify_json_object_fuel.py").read_bytes(),
    })
    for row in proof["cases"]:
        for field in ("requestFile", "expectedFile"):
            expected_files["verification/cases/" + row[field]] = (cases_dir / row[field]).read_bytes()
    if args.refresh_proof:
        actual_files = {path.relative_to(output).as_posix(): path.read_bytes()
                        for path in output.rglob("*") if path.is_file()}
        assert set(actual_files) == set(expected_files), "Existing candidate file list differs; refusing receipt refresh"
        assert all(actual_files[key] == value for key, value in expected_files.items()), "Existing candidate bytes differ; refusing receipt refresh"
    else:
        output.mkdir(parents=True)
        for relative, contents in expected_files.items():
            write(output, relative, contents)

    files = []
    for path in sorted(output.rglob("*")):
        if path.is_file():
            contents = path.read_bytes()
            files.append({"path": path.relative_to(output).as_posix(), "sha256": sha(contents), "size": len(contents)})
    assert len(files) <= 256 and sum(row["size"] for row in files) <= 10 * 1024 * 1024
    index = json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode()
    sources = [ROOT / path for path in (
        "Cargo.toml", "Cargo.lock", "crates/zi-json-core/Cargo.toml", "crates/zi-json-core/src/lib.rs",
        "crates/zi-json-object-wasi/Cargo.toml", "crates/zi-json-object-wasi/src/lib.rs",
        "crates/zi-json-object-wasi/src/main.rs", "crates/zi-text-core/src/lib.rs",
        "plugins/json-object/plugin.json", "plugins/json-object/catalog.json",
        "plugins/json-object/output.schema.json", "plugins/json-object/README.md",
        "scripts/verify_json_object_wasi.py", "scripts/run_text_wasi.mjs", "scripts/verify_json_object_fuel.py",
        "scripts/package_json_object_plugin.py", "docs/tools.json",
    )]
    receipt = {
        "packageId": manifest["id"], "packageVersion": manifest["version"], "status": "unsigned provider candidate; not Host-accepted",
        "source": source_state, "requiredProfiles": required,
        "wasm": {"sha256": sha(module), "bytes": len(module)},
        "nativeAdapter": {"sha256": sha(native.read_bytes())},
        "contractSnapshot": {"sourceSha256": source_meta["sourceSha256"],
                             "profileSha256": sha(profile_bytes)},
        "sourceFiles": [{"path": path.relative_to(ROOT).as_posix(), "sha256": sha(path.read_bytes())} for path in sources],
        "files": files, "directoryIndexSha256": sha(index), "packageSha256": None,
        "functionalProof": proof, "fuelProof": fuel, "coldStartProof": cold,
        "engineProvenance": engine,
        "limitations": ["Unsigned directory only; no installer or signature.",
                        "Studio Host negotiation, capture store, receiver search/prepare/confirm, JSON save/readback, permissions and lifecycle are unverified.",
                        "Node WASI functional parity is not a production sandbox or fuel acceptance test."],
    }
    receipt_path = output.parent / (output.name + "-verification.json")
    if receipt_path.exists():
        raise RuntimeError("External receipt already exists; refusing to overwrite it")
    receipt_path.write_bytes(json_bytes(receipt))
    print(f"Unsigned candidate: {output}; {len(files)} files, {sum(row['size'] for row in files)} bytes; index SHA-256 {sha(index)}")
    print(f"External receipt: {receipt_path}")


if __name__ == "__main__":
    main()
