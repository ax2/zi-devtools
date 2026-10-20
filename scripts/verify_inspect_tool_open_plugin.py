"""Verify Inspect tool.open.v1 metadata, callback behavior and exact package receipt."""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import urllib.request

from cdp_client import DevTools

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "release/plugin-candidates/inspect-0.1.1-candidate-02"
EXPECTED_OUTPUT = ROOT / "release/plugin-candidates/inspect-0.1.2-candidate-01"
OPEN_PROFILE = "=1.0.0-preview.1"
OPEN_TOOL = "tool.open.v1"
MUTABLE_BASE_FILES = {
    "plugin.json",
    "catalog.json",
    "README.md",
    "provenance.json",
    "views/main.html",
    "verification/view-proof.json",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def tree_files(folder):
    result = {}
    for path in sorted(folder.rglob("*")):
        if path.is_symlink():
            raise AssertionError(f"Symlink in package tree: {path}")
        if path.is_file():
            result[path.relative_to(folder).as_posix()] = path.read_bytes()
    return result


def portable_path_key(path):
    """Compare slash-normalized paths by Unicode code point, case-sensitively; do not normalize."""
    return tuple(ord(character) for character in path)


def verify_portable_path_order():
    # Python's platform-specific Path ordering is not part of the package contract.
    # These vectors make ASCII case and non-ASCII ordering explicit on every host.
    forward = ["A.txt", "a.txt", "z.txt", "é.txt", "中.txt"]
    reverse = list(reversed(forward))
    assert sorted(forward, key=portable_path_key) == forward
    assert sorted(reverse, key=portable_path_key) == forward
    assert portable_path_key("e\u0301.txt") != portable_path_key("é.txt")


def directory_index(files):
    verify_portable_path_order()
    rows = [
        {"path": name, "size": len(data), "sha256": sha(data)}
        for name, data in sorted(files.items(), key=lambda item: portable_path_key(item[0]))
    ]
    canonical = json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return sha(canonical), rows


parser = argparse.ArgumentParser()
parser.add_argument("--package", type=Path, default=EXPECTED_OUTPUT)
parser.add_argument(
    "--screenshots",
    type=Path,
    default=ROOT / "screenshots/inspect-tool-open-20261017",
)
args = parser.parse_args()
package = args.package.resolve(strict=True)
package_root = (ROOT / "release/plugin-candidates").resolve(strict=True)
if not package.is_relative_to(package_root) or package == package_root:
    raise SystemExit("Package must be inside release/plugin-candidates")

manifest = json.loads((package / "plugin.json").read_text(encoding="utf-8"))
assert manifest["schemaVersion"] == 2
assert manifest["id"] == "com.zicode.devtools.inspect" and manifest["version"] == "0.1.2"
assert manifest["requiredProfiles"].get(OPEN_TOOL) == OPEN_PROFILE
assert manifest["requiredProfiles"].get("tool.directory.v1") == "=1.0.0-preview.1"
main_views = [view for view in manifest["contributes"]["views"] if view["location"] == "main"]
assert len(main_views) == 1 and main_views[0]["id"] == "devtools.inspect.main"
capabilities = {capability["id"] for capability in manifest["contributes"]["capabilities"]}
assert len(capabilities) == 8
assert len({capability["id"] for capability in manifest["contributes"]["capabilities"]}) == 8

view_path = package / main_views[0]["entry"]
view_source = view_path.read_text(encoding="utf-8")
assert "sdk.navigation.onOpenTool(openTool)" in view_source
assert "function openTool({capabilityId}={})" in view_source
assert "capabilities.invoke" not in view_source[view_source.index("function openTool(") : view_source.index("async function run(){")]
assert "'not_found'" in view_source and "'unsupported'" in view_source and "return 'opened'" in view_source

base_files = tree_files(BASE)
package_files_before_proof = tree_files(package)
for name, data in base_files.items():
    if name not in MUTABLE_BASE_FILES:
        assert package_files_before_proof.get(name) == data, f"Frozen base payload changed: {name}"
assert hashlib.sha256((BASE / "runtime/tools.wasm").read_bytes()).digest() == hashlib.sha256(
    (package / "runtime/tools.wasm").read_bytes()
).digest(), "WASM changed in this UI-only candidate"

node = shutil.which("node")
assert node, "Node is required for the slow callback timing sample"
slow_sample = package / "verification/samples/slow-view-callback.js"
assert slow_sample.is_file()
slow_test = subprocess.run(
    [node, str(slow_sample)],
    text=True,
    capture_output=True,
    timeout=8,
    check=True,
)
slow_result = json.loads(slow_test.stdout)
assert slow_result["status"] == "unsupported" and slow_result["lateAckIgnored"]


class Handler(http.server.BaseHTTPRequestHandler):
    html = ""

    def do_GET(self):
        body = self.html.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


sdk = r'''<script>
window.__capabilityCalls=[];
window.zicode={ready:async()=>{},context:{get:async()=>({})},setDirty:()=>{},
navigation:{onOpenTool:callback=>{window.__openTool=callback;return{dispose:()=>{window.__openTool=null}}}},
capabilities:{invoke:async id=>{(window.__capabilityCalls||(window.__capabilityCalls=[])).push(id);return{}}}};
</script>'''
needle = "<script>\n(()=>{\\'use strict\\';"
if needle not in view_source:
    needle = "<script>\n(()=>{'use strict';"
Handler.html = view_source.replace(needle, sdk + "\n" + needle, 1)
assert Handler.html != view_source, "SDK injection point not found"
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
server.daemon_threads = True
threading.Thread(target=server.serve_forever, daemon=True).start()
edge = shutil.which("msedge.exe") or shutil.which("chrome.exe")
for candidate in [
    Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Microsoft/Edge/Application/msedge.exe",
    Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Google/Chrome/Application/chrome.exe",
]:
    if edge is None and candidate.is_file():
        edge = str(candidate)
assert edge, "Microsoft Edge or Chrome is required for the actual View checks"
local_open = urllib.request.build_opener(urllib.request.ProxyHandler({})).open
screenshots = []
args.screenshots.mkdir(parents=True, exist_ok=True)
try:
    for theme, width, height in [("light", 1180, 900), ("dark", 1180, 900)]:
        with tempfile.TemporaryDirectory(prefix="zi-inspect-open-profile-") as profile:
            process = subprocess.Popen(
                [
                    edge,
                    "--headless=new",
                    "--disable-gpu",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-extensions",
                    "--disable-background-networking",
                    "--no-proxy-server",
                    f"--user-data-dir={profile}",
                    "--remote-debugging-port=0",
                    "--window-size=1280,1000",
                    "about:blank",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            cdp = None
            try:
                port_file = Path(profile) / "DevToolsActivePort"
                deadline = time.monotonic() + 15
                while not port_file.is_file() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.05)
                assert port_file.is_file(), "Chromium did not open DevTools"
                port = int(port_file.read_text(encoding="utf-8").splitlines()[0])
                with local_open(f"http://127.0.0.1:{port}/json/list", timeout=3) as response:
                    target = next(row for row in json.load(response) if row.get("type") == "page")
                cdp = DevTools(target["webSocketDebuggerUrl"])
                cdp.socket.settimeout(20)
                for method in ["Page.enable", "Runtime.enable", "Log.enable"]:
                    cdp.command(method)
                cdp.command("Emulation.setDeviceMetricsOverride", {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": False})
                cdp.command("Emulation.setEmulatedMedia", {"features": [{"name": "prefers-color-scheme", "value": theme}]})
                cdp.command("Page.navigate", {"url": f"http://127.0.0.1:{server.server_port}/"})
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline and not cdp.evaluate("!!window.__openTool&&!!document.querySelector('#run')&&!document.querySelector('#run').disabled"):
                    time.sleep(0.05)
                assert cdp.evaluate("!!window.__openTool&&!!document.querySelector('#run')&&!document.querySelector('#run').disabled")

                opened = cdp.evaluate("window.__openTool({capabilityId:'devtools.inspect.color.convert'})")
                assert opened == "opened"
                assert cdp.evaluate("document.querySelector('#operation').value") == "color.convert"
                assert cdp.evaluate("window.__capabilityCalls.length") == 0
                assert "已定位到" in cdp.evaluate("document.querySelector('#status').textContent")
                image = base64.b64decode(cdp.command("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False})["data"])
                screenshot_path = args.screenshots / f"inspect-tool-open-{theme}.png"
                screenshot_path.write_bytes(image)
                screenshots.append({"theme": theme, "width": width, "height": height, "path": screenshot_path.relative_to(ROOT).as_posix(), "sha256": sha(image)})
                cdp.evaluate("document.querySelector('#operation').value='unicode.inspect';document.querySelector('#operation').dispatchEvent(new Event('change',{bubbles:true}))")
                unknown = cdp.evaluate("window.__openTool({capabilityId:'devtools.inspect.not-registered'})")
                assert unknown == "not_found"
                assert cdp.evaluate("document.querySelector('#operation').value") == "unicode.inspect"
                assert cdp.evaluate("window.__capabilityCalls.length") == 0
                errors = [event for event in cdp.events if event.get("method") == "Runtime.exceptionThrown" or (event.get("method") == "Log.entryAdded" and event.get("params", {}).get("entry", {}).get("level") == "error")]
                assert not errors, errors
            finally:
                if cdp:
                    try:
                        cdp.command("Browser.close")
                    except Exception:
                        pass
                    cdp.close()
                if process.poll() is None:
                    process.wait(timeout=5)
finally:
    server.shutdown()
    server.server_close()

proof = {
    "viewSha256": sha(view_path.read_bytes()),
    "wasmSha256": sha((package / "runtime/tools.wasm").read_bytes()),
    "profile": {"id": OPEN_TOOL, "version": OPEN_PROFILE},
    "cases": [
        {"id": "declared-color-converter", "result": "opened", "selectedAction": "color.convert", "capabilityInvocations": 0},
        {"id": "unknown-capability", "result": "not_found", "selectionPreserved": True, "capabilityInvocations": 0},
        {"id": "slow-view-callback", **slow_result},
    ],
    "browserResults": screenshots,
    "limitations": [
        "SDK-shaped local bridge tests provider View routing; this is not Studio Host installation or scene lifecycle acceptance.",
        "No capability execution, permission grant, file write or service operation is exercised by tool-open callbacks.",
        "Studio must verify host channel/request correlation, timeout, stale frame, permission revocation and scene switching.",
    ],
}
proof_path = package / "verification/view-proof.json"
proof_path.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

files = tree_files(package)
index_sha, rows = directory_index(files)
receipt = {
    "packageId": manifest["id"],
    "packageVersion": manifest["version"],
    "status": "unsigned provider candidate; not Studio Host accepted",
    "requiredProfiles": manifest["requiredProfiles"],
    "fileCount": len(rows),
    "totalBytes": sum(row["size"] for row in rows),
    "directoryIndexEncoding": {
        "hash": "SHA-256",
        "encoding": "UTF-8",
        "content": "files array in listed order",
        "objectKeyOrder": ["path", "size", "sha256"],
        "ensureAscii": False,
        "separators": [",", ":"],
        "trailingNewline": False,
    },
    "directoryIndexSha256": index_sha,
    "files": rows,
    "runtimeWasmSha256": proof["wasmSha256"],
    "viewSha256": proof["viewSha256"],
    "sourceFiles": json.loads((package / "provenance.json").read_text(encoding="utf-8"))["sourceFiles"],
    "frozenBaseCandidate": "release/plugin-candidates/inspect-0.1.1-candidate-02",
    "baseFileCount": len(base_files),
    "basePayloadUnmodifiedExceptDeclaredPackageFiles": True,
    "cases": proof["cases"],
    "screenshots": screenshots,
    "limitations": proof["limitations"],
}
receipt_path = package.with_name(package.name + "-verification.json")
receipt_path.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

# Re-read the output bytes; the receipt must match exactly what a receiver will inspect.
rechecked = tree_files(package)
assert rechecked == files
assert directory_index(rechecked)[0] == receipt["directoryIndexSha256"]
print(
    f"PASS Inspect tool.open.v1 candidate receipt: {len(rows)} files, "
    f"{receipt['totalBytes']} bytes, index {index_sha}; screenshots={len(screenshots)}"
)
