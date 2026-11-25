"""Freeze the Studio-only strict YAML update on the existing Inspect contract."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess

from revise_plugin_receipt import ENCODING, package_files, sha256

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.1.4"
BASE_WASM = "1db3fc37df01cd3a8371320d4f897a7fdf750c87e6202b736ed6870f046b8f29"
OUTPUT = ROOT / f"release/plugin-candidates/inspect-{VERSION}-candidate-01"
YAML_IDS = {"devtools.inspect.yaml.to_json", "devtools.inspect.yaml.from_json"}
LIBRARIES = {
    "yaml-rust2-0.13.0": ("Apache-LICENSE", "MIT-LICENSE"),
    "arraydeque-0.5.1": ("LICENSE",),
    "hashlink-0.12.2": ("LICENSE-APACHE", "LICENSE-MIT"),
}


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


def write_text(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8", newline="\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("base", type=Path, help="Frozen Inspect 0.1.3 candidate directory")
    parser.add_argument("wasm", type=Path, help="Built strict Inspect WASI")
    parser.add_argument("worker_proof", type=Path, help="Windows Worker vectors for this exact WASM")
    parser.add_argument("crate_sources", type=Path, help="Cargo registry sources for the new licenses")
    args = parser.parse_args()
    receipt_path = OUTPUT.with_name(OUTPUT.name + "-verification.json")
    if OUTPUT.exists() or receipt_path.exists():
        raise SystemExit("Output already exists. Frozen candidates are immutable.")
    base_receipt = read_json(args.base.with_name(args.base.name + "-verification.json"))
    manifest = read_json(args.base / "plugin.json")
    if manifest["version"] != "0.1.3" or manifest["id"] != "com.zicode.devtools.inspect":
        raise SystemExit("Wrong base identity")
    if sha256((args.base / "runtime/tools.wasm").read_bytes()) != BASE_WASM:
        raise SystemExit("Unexpected frozen base WASM")
    if package_files(args.base) != sorted(base_receipt["files"], key=lambda item: item["path"]):
        raise SystemExit("Frozen base files no longer match the independent receipt")
    wasm = args.wasm.read_bytes()
    proof = read_json(args.worker_proof)
    if not proof["rows"] or proof["wasmSha256"] != sha256(wasm):
        raise SystemExit("Missing or mismatched Worker proof")
    if any(not row["passed"] and not row.get("existingBudgetLimit") for row in proof["rows"]):
        raise SystemExit("A Worker regression or strict case remains unresolved")

    files = {(crate, file): (args.crate_sources / crate / file).read_bytes()
             for crate, names in LIBRARIES.items() for file in names}
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    shutil.copytree(args.base, OUTPUT)
    manifest["version"] = VERSION
    manifest["description"] = "Bounded format conversion and inspection. YAML and JSON conversion reject lossy numbers and duplicate keys."
    for capability in manifest["contributes"]["capabilities"]:
        if capability["id"] in YAML_IDS:
            capability["description"] = "Convert one document locally. Duplicate keys and numbers that cannot round-trip are refused."
    for tool in manifest["contributes"]["piTools"]:
        if tool["capability"] in YAML_IDS:
            tool["description"] += " Duplicate keys and numbers that cannot round-trip are refused."
    write_json(OUTPUT / "plugin.json", manifest)

    catalog = read_json(OUTPUT / "catalog.json")
    catalog["packageVersion"] = VERSION
    catalog["source"] = {"revision": revision, "dirty": False}
    write_json(OUTPUT / "catalog.json", catalog)
    directory = OUTPUT / "verification/tool-directory-snapshot.json"
    snapshot = read_json(directory)
    snapshot["packageVersion"] = VERSION
    write_json(directory, snapshot)

    fixture_path = OUTPUT / "fixtures/inspect.json"
    fixtures = read_json(fixture_path)
    for case in fixtures["cases"]:
        if case["id"] in {"yaml-duplicate-last", "json-duplicate-last"}:
            case["id"] = case["id"].replace("last", "rejected")
            case["expectedText"] = None
            case["expectedError"] = "INVALID_INPUT"
    write_json(fixture_path, fixtures)

    view_path = OUTPUT / "views/main.html"
    view = view_path.read_text(encoding="utf-8")
    old_hint = "重复键使用最后一个值；保留大整数的文本表示，每次转换一个 YAML 文档。"
    if view.count(old_hint) != 1:
        raise SystemExit("Unexpected Inspect View hint")
    write_text(view_path, view.replace(old_hint, "每次转换一个文档。要求键唯一，数字可精确往返。"))
    (OUTPUT / "runtime/tools.wasm").write_bytes(wasm)
    for (crate, file), data in files.items():
        destination = OUTPUT / "licenses" / crate / file
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    notice_path = OUTPUT / "NOTICE"
    notice = notice_path.read_text(encoding="utf-8")
    write_text(notice_path, notice + "\nNew runtime dependencies:\n"
        "arraydeque 0.5.1: MIT OR Apache-2.0\n"
        "hashlink 0.12.2: MIT OR Apache-2.0\n"
        "yaml-rust2 0.13.0: MIT OR Apache-2.0\n")
    for filename in ["cold-proof.json", "schema-proof.json", "view-proof.json"]:
        (OUTPUT / "verification" / filename).unlink()
    write_json(OUTPUT / "verification/strict-yaml-worker.json", proof)
    write_json(OUTPUT / "provenance.json", {
        "packageId": manifest["id"], "packageVersion": VERSION, "sourceRevision": revision,
        "baseCandidate": args.base.name, "baseRuntimeSha256": BASE_WASM, "runtimeWasmSha256": sha256(wasm),
        "viewSha256": sha256(view_path.read_bytes()), "signature": None,
        "proof": "Windows production Worker; Studio installation acceptance is a separate step",
    })
    write_text(OUTPUT / "README.md",
        "# ZiDevTools Inspect 0.1.4 candidate\n\n"
        "Studio YAML and JSON conversion rejects duplicate keys, non-finite numeric values and numbers "
        "that cannot be written without changing their decimal value. Exact signed and unsigned 64-bit "
        "integers are preserved. Both directions use at most 8 KiB of input and the existing 48 KiB result "
        "contract and Worker limits. Dense YAML may exhaust the existing Worker computation budget.\n\n"
        "The eight capability identities and original Host profiles are preserved. The standalone "
        "conversion keeps its existing compatibility behavior. Inspect 0.1.3 remains frozen. "
        "This candidate is unsigned.\n")
    write_text(OUTPUT / "MIGRATION.md",
        "# Inspect 0.1.3 to 0.1.4\n\n"
        "The same Studio capability IDs now refuse duplicate keys and unrepresentable numbers instead "
        "of writing a modified document. The provider does not modify source files. Convert quoted "
        "number strings as strings where the source document intends text.\n")

    rows = package_files(OUTPUT)
    index = json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    write_json(receipt_path, {
        "schemaVersion": 1, "pluginId": manifest["id"], "packageVersion": VERSION,
        "signatureVerified": False, "files": rows, "fileCount": len(rows),
        "totalBytes": sum(row["size"] for row in rows),
        "directoryIndexSha256": sha256(index), "directoryIndexEncoding": ENCODING,
    })
    print(f"{OUTPUT}: {len(rows)} files, index {sha256(index)}, WASM {sha256(wasm)}")


if __name__ == "__main__":
    main()
