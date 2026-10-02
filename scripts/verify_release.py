"""Verify the three release files against the supplied SHA256SUMS manifest.

This detects download/file corruption; it does not authenticate the publisher.
"""
import argparse
import hashlib
from pathlib import Path
import re
import tomllib


def verify(folder: Path, version: str) -> list[str]:
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Expected a numeric major.minor.patch version")
    expected = {
        f"ZiDevTools-{version}-windows-x64.exe",
        f"ZiDevTools-{version}-windows-x64.msi",
        f"ZiDevToolsMcp-{version}-windows-x64.exe",
    }
    folder = folder.resolve(strict=True)
    manifest = folder / "SHA256SUMS.txt"
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > 16384:
        raise ValueError("Missing, linked or oversized SHA256SUMS.txt")
    entries = {}
    for line in manifest.read_text(encoding="utf-8-sig").splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64})  ([A-Za-z0-9_.-]+)", line)
        if not match:
            raise ValueError("Invalid checksum entry")
        digest, name = match.groups()
        if name not in expected or name in entries:
            raise ValueError("Unexpected or duplicate release filename")
        entries[name] = digest.lower()
    if set(entries) != expected:
        raise ValueError("Checksum manifest must contain all three release files")
    for name in sorted(expected):
        file = folder / name
        if file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f"Missing, linked or empty file: {name}")
        digest = hashlib.sha256()
        with file.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        if digest.hexdigest() != entries[name]:
            raise ValueError(f"SHA-256 mismatch: {name}")
    return sorted(expected)


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-dir", type=Path, default=root / "release")
    parser.add_argument("--version", default=tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"])
    args = parser.parse_args()
    try:
        names = verify(args.artifact_dir, args.version)
    except (OSError, UnicodeError, ValueError) as error:
        parser.exit(1, f"FAIL: {error}\n")
    for name in names:
        print(f"PASS: {name}")


if __name__ == "__main__":
    main()
