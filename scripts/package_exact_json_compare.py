"""Freeze the two verified Studio exact JSON Compare candidates."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess

from revise_plugin_receipt import ENCODING, package_files, sha256

ROOT = Path(__file__).resolve().parents[1]
PREVIOUS_VERSION = "0.1.0"
VERSION = "0.1.1"
OUTPUT = ROOT / "release/plugin-candidates"
KINDS = {
    "compare": (
        "5b15da8793af1910a697693b25d2d3d5b7ddf0c409a2192ec085abf6c400c5be",
        "Two UTF-8 JSON inputs, each at most 8 KiB, retain the 48 KiB request/result contract.",
        "zi-studio-compare-wasi",
    ),
    "compare-json-file": (
        "3c62b04aa498cfa9a1207f9b095b2fbf37615f38a01678c7759ea67d2a61a9ab",
        "The two individually approved input files remain bounded to 1 MiB each and the complete result to 8 MiB.",
        "zi-json-compare-file-wasi",
    ),
}


def read_json(path: Path) -> dict:
    return json.loads(path.read_bytes())


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=KINDS)
    parser.add_argument("base", type=Path, help="Frozen 0.1.0 candidate with its external receipt")
    parser.add_argument("wasm", type=Path, help="Exact binary from the current provider source")
    parser.add_argument("proof", type=Path, help="Windows evidence for this precise WASM")
    args = parser.parse_args()

    output = OUTPUT / f"{args.kind}-{VERSION}-candidate-01"
    receipt_path = output.with_name(output.name + "-verification.json")
    if output.exists() or receipt_path.exists():
        raise SystemExit("Frozen candidates are immutable")
    manifest = read_json(args.base / "plugin.json")
    plugin = f"com.zicode.devtools.{args.kind}"
    receipt = read_json(args.base.with_name(args.base.name + "-verification.json"))
    base_wasm, limits, binary = KINDS[args.kind]
    if (
        manifest["id"] != plugin or manifest["version"] != PREVIOUS_VERSION
        or sorted(receipt["files"], key=lambda row: row["path"]) != package_files(args.base)
        or sha256((args.base / "runtime/tools.wasm").read_bytes()) != base_wasm
    ):
        raise SystemExit("Unexpected frozen base")
    wasm = args.wasm.read_bytes()
    wasm_sha = sha256(wasm)
    proof = read_json(args.proof)
    if proof["wasmSha256"] != wasm_sha or not proof["rows"] or any(not row["passed"] for row in proof["rows"]):
        raise SystemExit("Windows proof does not match the binary or contains unresolved cases")
    if subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT,
    ).strip():
        raise SystemExit("Freeze only from a clean source revision")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()

    shutil.copytree(args.base, output)
    manifest["version"] = VERSION
    manifest["description"] += " JSON 对比采用精确十进制数值，变化中保留原始数字。"
    write_json(output / "plugin.json", manifest)
    catalog = read_json(output / "catalog.json")
    catalog["packageVersion"] = VERSION
    for tool in catalog["tools"]:
        if tool["sourceToolId"] == "json-diff":
            tool["scope"] += " JSON 对比精确保留并比较十进制数字。"
    write_json(output / "catalog.json", catalog)
    (output / "runtime/tools.wasm").write_bytes(wasm)
    write_json(output / "provenance.json", {
        "sourceRevision": revision,
        "dirty": False,
        "signature": "unsigned",
        "wasmTarget": "wasm32-wasip1",
        "wasmProfile": "plugin",
        "moduleSha256": wasm_sha,
        "binary": binary,
        "baseCandidate": args.base.name,
        "baseRuntimeSha256": base_wasm,
        "windowsProofSha256": sha256(args.proof.read_bytes()),
    })
    (output / "README.md").write_text(
        f"# {manifest['name']} {VERSION} candidate\n\n"
        "JSON diff compares exact decimal values in both array modes and writes unchanged numeric "
        "tokens in before/after values. The verified original numeric-kind and signed-zero policies "
        f"are preserved. {limits} The existing JSON path query keeps its previous numeric policy.\n\n"
        "The package keeps the previous capability IDs and permission contract. "
        "This Windows-tested candidate is unsigned. Original fixtures inherited from 0.1.0 "
        "are reference vectors for the existing contract.\n",
        encoding="utf-8", newline="\n",
    )

    rows = package_files(output)
    index = json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode()
    write_json(receipt_path, {
        "schemaVersion": 1,
        "pluginId": plugin,
        "packageVersion": VERSION,
        "signatureVerified": False,
        "files": rows,
        "fileCount": len(rows),
        "totalBytes": sum(row["size"] for row in rows),
        "directoryIndexSha256": sha256(index),
        "directoryIndexEncoding": ENCODING,
    })
    print(f"{output}: {len(rows)} files, index {sha256(index)}, WASM {wasm_sha}")


if __name__ == "__main__":
    main()
