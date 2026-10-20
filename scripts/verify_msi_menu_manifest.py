#!/usr/bin/env python3
"""Verify the menu capability declaration embedded in a built MSI."""
from __future__ import annotations

import argparse
import base64
import ctypes
import hashlib
import json
import re
from pathlib import Path

REQUIRED_PROPERTIES = ("ProductVersion", "ZiMenuBinarySha256", "ZiMenuCapabilities")
REQUIRED_ACTIONS = (
    "ZiMenuPrepare", "ZiMenuRollback", "ZiMenuExecute", "ZiMenuCommitReady",
    "ZiMenuCommitReceipt", "ZiMenuFinalize",
)
DLL_FROM_BINARY = 0x001
IGNORE_FAILURE = 0x040
ROLLBACK = 0x100
COMMIT = 0x200
IN_SCRIPT = 0x400
EXPECTED_CUSTOM_ACTIONS = {
    "ZiMenuPrepare": (DLL_FROM_BINARY, "ZiMenuActions", "ZiMenuPrepare"),
    "ZiMenuRollback": (DLL_FROM_BINARY | IN_SCRIPT | ROLLBACK, "ZiMenuActions", "ZiMenuRollback"),
    "ZiMenuExecute": (DLL_FROM_BINARY | IN_SCRIPT, "ZiMenuActions", "ZiMenuExecute"),
    "ZiMenuCommitReady": (DLL_FROM_BINARY | IN_SCRIPT | COMMIT, "ZiMenuActions", "ZiMenuCommitReady"),
    "ZiMenuCommitReceipt": (DLL_FROM_BINARY | IGNORE_FAILURE, "ZiMenuActions", "ZiMenuCommitReceipt"),
    "ZiMenuFinalize": (DLL_FROM_BINARY | IGNORE_FAILURE, "ZiMenuActions", "ZiMenuFinalize"),
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


class MsiDatabase:
    def __init__(self, path: Path) -> None:
        api = ctypes.WinDLL("msi")
        signatures = {
            "MsiOpenDatabaseW": ([ctypes.c_wchar_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
            "MsiDatabaseOpenViewW": ([ctypes.c_uint, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
            "MsiViewExecute": ([ctypes.c_uint, ctypes.c_uint], ctypes.c_uint),
            "MsiViewFetch": ([ctypes.c_uint, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
            "MsiRecordGetStringW": ([ctypes.c_uint, ctypes.c_uint, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
            "MsiRecordGetInteger": ([ctypes.c_uint, ctypes.c_uint], ctypes.c_int),
            "MsiRecordReadStream": ([ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
            "MsiCloseHandle": ([ctypes.c_uint], ctypes.c_uint),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(api, name)
            function.argtypes, function.restype = arguments, result
        self.api = api
        self.handle = ctypes.c_uint()
        code = api.MsiOpenDatabaseW(str(path), None, ctypes.byref(self.handle))
        require(code == 0, f"Cannot open MSI database (Windows Installer error {code})")

    def rows(self, sql: str, text_fields: tuple[int, ...], integer_fields: tuple[int, ...] = ()) -> list[dict]:
        view = ctypes.c_uint()
        code = self.api.MsiDatabaseOpenViewW(self.handle, sql, ctypes.byref(view))
        require(code == 0, f"Cannot query MSI database (Windows Installer error {code})")
        rows = []
        try:
            code = self.api.MsiViewExecute(view, 0)
            require(code == 0, f"Cannot execute MSI query (Windows Installer error {code})")
            while True:
                record = ctypes.c_uint()
                code = self.api.MsiViewFetch(view, ctypes.byref(record))
                if code == 259:
                    break
                require(code == 0 and len(rows) < 512, "MSI query failed or returned too many rows")
                try:
                    row = {}
                    for index in text_fields:
                        size = ctypes.c_uint(256)
                        buffer = ctypes.create_unicode_buffer(size.value)
                        code = self.api.MsiRecordGetStringW(record, index, buffer, ctypes.byref(size))
                        if code == 234:
                            require(size.value <= 32767, "MSI field exceeds the supported length")
                            buffer = ctypes.create_unicode_buffer(size.value + 1)
                            code = self.api.MsiRecordGetStringW(record, index, buffer, ctypes.byref(size))
                        require(code == 0, f"Cannot read MSI field {index}")
                        row[index] = buffer.value
                    for index in integer_fields:
                        row[index] = self.api.MsiRecordGetInteger(record, index)
                    rows.append(row)
                finally:
                    self.api.MsiCloseHandle(record)
            return rows
        finally:
            self.api.MsiCloseHandle(view)

    def close(self) -> None:
        if self.handle.value:
            self.api.MsiCloseHandle(self.handle)
            self.handle = ctypes.c_uint()

    def binary_stream_sha256(self, name: str) -> str:
        view = ctypes.c_uint()
        query = f"SELECT `Data` FROM `Binary` WHERE `Name` = '{name}'"
        code = self.api.MsiDatabaseOpenViewW(self.handle, query, ctypes.byref(view))
        require(code == 0, f"Cannot query MSI binary stream (Windows Installer error {code})")
        try:
            code = self.api.MsiViewExecute(view, 0)
            require(code == 0, f"Cannot execute MSI binary query (Windows Installer error {code})")
            record = ctypes.c_uint()
            code = self.api.MsiViewFetch(view, ctypes.byref(record))
            require(code == 0, f"MSI binary stream is missing: {name}")
            try:
                digest = hashlib.sha256()
                total = 0
                buffer = ctypes.create_string_buffer(64 * 1024)
                while True:
                    size = ctypes.c_uint(len(buffer) - 1)
                    code = self.api.MsiRecordReadStream(record, 1, buffer, ctypes.byref(size))
                    require(code == 0, f"Cannot read MSI binary stream (Windows Installer error {code})")
                    require(size.value <= len(buffer) - 1, "MSI binary stream returned an invalid chunk size")
                    if size.value == 0:
                        break
                    total += size.value
                    require(total <= 128 * 1024 * 1024, "MSI binary stream exceeds the supported size")
                    digest.update(buffer.raw[:size.value])
                require(total > 0, f"MSI binary stream is empty: {name}")
                return digest.hexdigest()
            finally:
                self.api.MsiCloseHandle(record)
        finally:
            self.api.MsiCloseHandle(view)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--msi", type=Path, required=True)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--menu-actions-dll", type=Path, required=True)
    parser.add_argument("--app-version", required=True)
    parser.add_argument("--exe-sha256", required=True)
    parser.add_argument("--capabilities-base64", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    msi = args.msi.resolve(strict=True)
    exe = args.exe.resolve(strict=True)
    menu_actions_dll = args.menu_actions_dll.resolve(strict=True)
    expected_exe_sha = hashlib.sha256(exe.read_bytes()).hexdigest()
    expected_menu_actions_sha = hashlib.sha256(menu_actions_dll.read_bytes()).hexdigest()
    require(expected_exe_sha == args.exe_sha256.lower(), "EXE changed after capability inspection")
    try:
        declaration = base64.b64decode(args.capabilities_base64, validate=True).decode("utf-8")
    except (ValueError, UnicodeDecodeError) as error:
        raise SystemExit("Invalid inspected capability declaration") from error
    prefix = "ZiDevTools/ShellCapabilities/1|"
    require(declaration.startswith(prefix) and len(declaration.encode("ascii")) <= 255, "MSI capability declaration header or limit mismatch")
    identifiers = declaration[len(prefix):].split(",") if declaration[len(prefix):] else []
    require(len(identifiers) == len(set(identifiers)), "Capability declaration contains duplicates")
    require(all(re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}(?::[a-z0-9][a-z0-9-]{0,63})?", item) for item in identifiers), "Capability declaration has an invalid action id")
    require(not identifiers or identifiers[0] == "choose-folders", "Unknown MSI folder-context flag")
    require("choose-folders" not in identifiers[1:], "Duplicate MSI folder-context flag")

    db = MsiDatabase(msi)
    try:
        properties = {
            row[1]: row[2]
            for row in db.rows(
                "SELECT `Property`, `Value` FROM `Property`",
                text_fields=(1, 2),
            )
        }
        for name in REQUIRED_PROPERTIES:
            require(name in properties, f"MSI property missing: {name}")
        require(properties["ProductVersion"] == args.app_version, "MSI ProductVersion differs from the source package version")
        require(properties["ZiMenuBinarySha256"].lower() == expected_exe_sha, "MSI EXE hash differs from the inspected executable")
        require(properties["ZiMenuCapabilities"] == declaration, "MSI capability declaration differs from the inspected executable")
        custom_actions = {
            row[1]: (row[2], row[3], row[4])
            for row in db.rows(
                "SELECT `Action`, `Type`, `Source`, `Target` FROM `CustomAction`",
                text_fields=(1, 3, 4), integer_fields=(2,),
            )
        }
        require(
            all(custom_actions.get(name) == EXPECTED_CUSTOM_ACTIONS[name] for name in REQUIRED_ACTIONS),
            "MSI menu transaction action type, Binary source, or DLL entry point differs from the shipping template",
        )
        execute_actions = {
            row[1]: row[2]
            for row in db.rows(
                "SELECT `Action`, `Sequence` FROM `InstallExecuteSequence`",
                text_fields=(1,), integer_fields=(2,),
            )
        }
        require(all(name in execute_actions for name in REQUIRED_ACTIONS), "MSI execute sequence omits a menu transaction action")
        embedded_menu_actions_sha = db.binary_stream_sha256("ZiMenuActions")
        require(embedded_menu_actions_sha == expected_menu_actions_sha, "MSI menu transaction DLL differs from the built DLL")
        file_rows = db.rows("SELECT `File` FROM `File`", text_fields=(1,))
        require(any(row[1] == "ZiDevToolsExe" for row in file_rows), "MSI does not contain the inspected desktop executable")
    finally:
        db.close()

    proof = {
        "schemaVersion": 3,
        "msi": msi.name,
        "msiSha256": hashlib.sha256(msi.read_bytes()).hexdigest(),
        "productVersion": args.app_version,
        "desktopExeSha256": expected_exe_sha,
        "menuActionsDllSha256": expected_menu_actions_sha,
        "menuActionsBinarySha256": embedded_menu_actions_sha,
        "menuActionsBinaryExact": True,
        "capabilityCount": len(identifiers) - int(bool(identifiers and identifiers[0] == "choose-folders")),
        "capabilityDeclarationSha256": hashlib.sha256(declaration.encode()).hexdigest(),
        "capabilityPropertyExact": True,
        "transactionActions": list(REQUIRED_ACTIONS),
        "customActionBindings": {
            name: {"type": custom_actions[name][0], "binary": custom_actions[name][1], "entryPoint": custom_actions[name][2]}
            for name in REQUIRED_ACTIONS
        },
        "embeddedMenuActionDll": True,
        "desktopExeInPackage": True,
        "scope": "MSI database content check only; install/upgrade/repair/uninstall behavior is covered by separate disposable-runner smoke tests.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"PASS MSI menu manifest: {proof['capabilityCount']} declared capabilities; exact EXE/DLL digests, six action bindings and transaction chain")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
