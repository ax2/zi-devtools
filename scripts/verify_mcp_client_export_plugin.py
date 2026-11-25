"""Verify MCP config text through the native adapter and actual Windows Studio Worker."""

import argparse
import json
from pathlib import Path
import tomllib

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.mcp-client-export"
TOML = "devtools.mcp.config.codex-toml"
JSON = "devtools.mcp.config.generic-json"
CONTRACT = "1.0.0-rc.1"
MAX_PATH = 1024
MAX_RESULT = 8192
WINDOWS_EXE = "ZiDevToolsMcp.exe"
QUOTED = r"D:\构建 字符's Suite\ZiDevToolsMcp.exe"
PORTABLE = r"D:\portable work\ZiDevToolsMcp-0.82.0-dev.1+win-windows-x64.EXE"


def request(capability, inputs):
    return dict(pluginId=PLUGIN, sceneId="programming", capabilityId=capability,
                commandId=None, input=inputs)


def fixtures():
    wire_base = "C:\\" + (("a" * 240) + "\\") * 4
    exact = wire_base + "中" * 10 + "x" * (MAX_PATH - len(wire_base.encode()) - 31 - len(WINDOWS_EXE))
    exact += "\\" + WINDOWS_EXE
    assert len(exact.encode()) == MAX_PATH
    for name, path in [
        ("drive-root", r"C:\ZiDevToolsMcp.exe"),
        ("apostrophe-unicode-spaces", QUOTED),
        ("portable-version-name", PORTABLE),
        ("forward-separators", "C:/zicode/work/ZiDevToolsMcp.exe"),
        ("shell-characters-retained-as-literal", r"C:\test (local) & $literal [config]\ZiDevToolsMcp.exe"),
        ("1024-bytes-exactly", exact),
    ]:
        for cap, fmt in [(TOML, "toml"), (JSON, "json")]:
            yield f"{name}-{fmt}", cap, dict(executablePath=path), fmt
    for name, path in [
        ("cannot-add-one-byte", exact.replace("x\\", "xx\\")),
        ("relative", r"tools\ZiDevToolsMcp.exe"),
        ("relative-drive", r"C:tools\ZiDevToolsMcp.exe"),
        ("remote-unc", r"\\server\share\ZiDevToolsMcp.exe"),
        ("external-device", r"\\?\C:\tools\ZiDevToolsMcp.exe"),
        ("uri", "file:///C:/tools/ZiDevToolsMcp.exe"),
        ("not-the-native-executable", r"C:\Windows\notepad.exe"),
        ("bad-portable-version", r"C:\test\ZiDevToolsMcp-fake-windows-x64.exe"),
        ("double-extension", r"C:\test\ZiDevToolsMcp.exe.cmd"),
        ("alternate-data-stream", r"C:\test\ZiDevToolsMcp.exe:other"),
        ("surrounding-quotes", '"C:\\test\\ZiDevToolsMcp.exe"'),
        ("traversal", r"C:\test\..\ZiDevToolsMcp.exe"),
        ("dot-segment", r"C:\test\.\ZiDevToolsMcp.exe"),
        ("empty-segment", r"C:\test\\ZiDevToolsMcp.exe"),
        ("too-wide-component", "C:\\" + "中" * 256 + "\\ZiDevToolsMcp.exe"),
        ("reserved-device-component", r"C:\CON.txt\ZiDevToolsMcp.exe"),
        ("bad-windows-character", r"C:\a*b\ZiDevToolsMcp.exe"),
        ("newline", "C:\\config\n[mcp_servers.other]\\ZiDevToolsMcp.exe"),
        ("unicode-separator", "C:\\line\u2028sep\\ZiDevToolsMcp.exe"),
        ("bidi-control", "C:\\evil\u202etxt\\ZiDevToolsMcp.exe"),
        ("null-byte", "C:\\a\x00b\\ZiDevToolsMcp.exe"),
        ("empty", ""),
    ]:
        yield name, TOML, dict(executablePath=path), "INVALID_PATH"
    yield "unexpected-setting", JSON, dict(executablePath=QUOTED, args=["other"]), "INVALID_REQUEST"
    yield "non-string-path", JSON, dict(executablePath={"path": QUOTED}), "INVALID_REQUEST"


def verify_snippet(format, text, path):
    if format == "toml":
        assert tomllib.loads(text) == {"mcp_servers": {"zi-knowledge": {
            "command": path, "enabled_tools": ["list_knowledge_sources", "search_knowledge"],
            "default_tools_approval_mode": "writes",
        }}}
        assert text.endswith("\n")
    else:
        assert json.loads(text) == {"mcpServers": {"zi-knowledge": {
            "command": path, "args": [],
        }}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("native", "worker", "module", "evidence"):
        parser.add_argument("--" + flag, required=True, type=Path)
    args = parser.parse_args()
    cases, failures = [], []
    for name, capability, inputs, expected in fixtures():
        try:
            envelope = request(capability, inputs)
            native = invoke([args.native], compact(envelope))
            outer = invoke([args.worker, "--zicode-plugin-worker"],
                           compact(dict(workerKind="wasi", workerPath=str(args.module), **envelope)))
            assert outer["ok"], outer
            result = outer["result"]
            assert result == native and result["contractVersion"] == CONTRACT
            assert len(compact(result)) <= MAX_RESULT
            success = expected in ("toml", "json")
            assert result["ok"] is success, result
            if success:
                assert result["error"] is None and result["data"]["format"] == expected
                verify_snippet(expected, result["data"]["text"], inputs["executablePath"])
            else:
                assert result["data"] is None and result["error"]["code"] == expected
                assert isinstance(result["error"]["message"], str)
            cases.append(dict(case=name, capabilityId=capability, inputSha256=digest(compact(inputs)),
                              ok=success, resultSha256=digest(compact(result)), resultBytes=len(compact(result))))
        except Exception as error:
            failures.append(dict(case=name, message=repr(error)[:900]))
    proof = dict(passed=len(cases), wasmSha256=digest(args.module.read_bytes()),
                 workerSha256=digest(args.worker.read_bytes()), workerProfile="compute.wasi.v1",
                 cases=cases, failures=failures)
    args.evidence.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(dict(passed=len(cases), failures=failures), ensure_ascii=False))
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
