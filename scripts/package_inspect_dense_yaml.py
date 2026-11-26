"""Freeze an Inspect 0.1.5 candidate from a verified 0.1.4 directory."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess

from revise_plugin_receipt import ENCODING, package_files, sha256

VERSION = "0.1.5"
PLUGIN = "com.zicode.devtools.inspect"
BASE_WASM = "87eac509ffefdd8f4ba45a226afff3ecc24934618fc12b59bf2b03574448087f"
BASE_INDEX = "ad2349420097697c2b4c14fab5c1a0a97eb67382e87fcbf4569fa0d8f57c7d9e"
WORKER_ROWS = {"dense-sequence-8192", "dense-mapping-8192", "minified-integers-8192",
               "control-floats-8192", "control-ints-8192"}


def load(path: Path) -> dict:
    return json.loads(path.read_bytes())


def write(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


def index_digest(rows: list[dict]) -> str:
    return sha256(json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("wasm", type=Path)
    parser.add_argument("worker_proof", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    receipt_path = args.output.with_name(args.output.name + "-verification.json")
    if args.output.exists() or receipt_path.exists():
        raise SystemExit("Output already exists. Frozen candidates are immutable.")

    manifest = load(args.base / "plugin.json")
    old_receipt = load(args.base.with_name(args.base.name + "-verification.json"))
    base_files = package_files(args.base)
    if (manifest["id"], manifest["version"]) != (PLUGIN, "0.1.4"):
        raise SystemExit("Wrong base plugin")
    if (base_files != old_receipt["files"] or index_digest(base_files) != BASE_INDEX
            or old_receipt["directoryIndexSha256"] != BASE_INDEX
            or sha256((args.base / "runtime/tools.wasm").read_bytes()) != BASE_WASM):
        raise SystemExit("Base is not the independently frozen Inspect 0.1.4")

    wasm = args.wasm.read_bytes()
    wasm_sha = sha256(wasm)
    proof = load(args.worker_proof)
    rows = proof["results"]
    if (proof["wasm"] != wasm_sha or wasm_sha == BASE_WASM or len(rows) != len(WORKER_ROWS)
            or {row["id"] for row in rows} != WORKER_ROWS
            or any(row["exit"] != 0 or not row["ok"] or not row["resultOk"] for row in rows)):
        raise SystemExit("Expected five exact successful Windows Worker results")

    repo = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain"], cwd=repo, text=True).strip()
    if dirty:
        raise SystemExit("Package source checkout must be clean")

    shutil.copytree(args.base, args.output)
    manifest["version"] = VERSION
    write(args.output / "plugin.json", manifest)
    catalog_path = args.output / "catalog.json"
    catalog = load(catalog_path)
    catalog["packageVersion"] = VERSION
    catalog["source"] = {"revision": revision, "dirty": False}
    write(catalog_path, catalog)
    snapshot_path = args.output / "verification/tool-directory-snapshot.json"
    snapshot = load(snapshot_path)
    snapshot["packageVersion"] = VERSION
    write(snapshot_path, snapshot)
    (args.output / "runtime/tools.wasm").write_bytes(wasm)
    (args.output / "verification/strict-yaml-worker.json").unlink()
    write(args.output / "verification/yaml-density-worker.json", proof)
    view_sha = sha256((args.output / "views/main.html").read_bytes())
    write(args.output / "provenance.json", {
        "packageId": PLUGIN, "packageVersion": VERSION, "sourceRevision": revision,
        "baseCandidate": args.base.name, "baseRuntimeSha256": BASE_WASM, "baseIndexSha256": BASE_INDEX,
        "runtimeWasmSha256": wasm_sha, "viewSha256": view_sha, "signature": None,
        "proof": "Windows production Worker; Studio installation acceptance is a separate step",
    })
    (args.output / "README.md").write_text(
        "# ZiDevTools Inspect 0.1.5 candidate\n\n"
        "Dense flat numeric YAML sequences and maps can convert within the same Worker budget. "
        "The eight tools, profiles, 8 KiB input, and 48 KiB output limits are unchanged. "
        "Conversion still requires unique keys and lossless numbers. Mixed and nested YAML uses "
        "the general parser and can exhaust the existing budget.\n\n"
        "A separate Worker receipt covers the three previously failing documents and two controls. "
        "Portable fixture bytes, the View, and the licenses come from the verified 0.1.4 package. "
        "The standalone converter is unchanged. This candidate is unsigned.\n",
        encoding="utf-8", newline="\n",
    )
    (args.output / "MIGRATION.md").write_text(
        "# Inspect 0.1.4 to 0.1.5\n\n"
        "Install the separately versioned plugin in Studio and review enablement. "
        "Inputs, outputs, capability IDs, and permissions use the same contract. "
        "Conversion reads without writing the source file.\n",
        encoding="utf-8", newline="\n",
    )

    files = package_files(args.output)
    index = index_digest(files)
    write(receipt_path, {
        "schemaVersion": 1, "pluginId": PLUGIN, "packageVersion": VERSION,
        "signatureVerified": False, "files": files, "fileCount": len(files),
        "totalBytes": sum(row["size"] for row in files),
        "directoryIndexSha256": index, "directoryIndexEncoding": ENCODING,
    })
    print(f"{args.output}: {len(files)} files, index {index}, WASM {wasm_sha}")


if __name__ == "__main__":
    main()
