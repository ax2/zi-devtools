"""Build only the SQLite WASI guest using the locked amalgamation and a local Zig 0.15.2."""

import argparse
import hashlib
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE_SHA = "c01235302fe80da901fb70c7622c39147e29d9f29b7f6eb746b23517f320c90d"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--zig", required=True, type=Path, help="Path to the official Zig 0.15.2 compiler")
    parser.add_argument("--sqlite-source", required=True, type=Path,
                        help="sqlite3.c from Cargo's locked libsqlite3-sys 0.30.1")
    parser.add_argument("--work", required=True, type=Path, help="Writable directory for build-only C artifacts")
    parser.add_argument("--package", choices=("zi-table-sqlite-wasi", "zi-sqlite-browser-wasi", "zi-knowledge-keyword-wasi", "zi-knowledge-index-build-wasi"),
                        default="zi-table-sqlite-wasi", help="SQLite guest to build")
    args = parser.parse_args()
    compiler, sqlite, work = (value.resolve() for value in (args.zig, args.sqlite_source, args.work))
    if hashlib.sha256(sqlite.read_bytes()).hexdigest() != SOURCE_SHA:
        parser.error("SQLite source differs from locked libsqlite3-sys 0.30.1")
    if subprocess.check_output([compiler, "version"], text=True).strip() != "0.15.2":
        parser.error("use Zig 0.15.2 for the verified build")
    local, shared = work / "local-cache", work / "global-cache"
    for name in ("h", "o", "z", "tmp"):
        (local / name).mkdir(parents=True, exist_ok=True)
    shared.mkdir(parents=True, exist_ok=True)
    context = dict(os.environ, ZIG_LOCAL_CACHE_DIR=str(local), ZIG_GLOBAL_CACHE_DIR=str(shared),
                   TEMP=str(work), TMP=str(work))
    obj, archive = work / "sqlite3.o", work / "libsqlite3.a"
    fts_flags = ["-DSQLITE_ENABLE_FTS5=1"] if args.package in {"zi-knowledge-keyword-wasi", "zi-knowledge-index-build-wasi"} else []
    subprocess.run([compiler, "cc", "-target", "wasm32-wasi", "-Os", "-ffunction-sections",
                    "-fdata-sections", "-DSQLITE_MAX_MMAP_SIZE=0", "-DSQLITE_DISABLE_DIRSYNC=1",
                    "-DSQLITE_THREADSAFE=0", "-DSQLITE_TEMP_STORE=3", "-DSQLITE_OMIT_LOAD_EXTENSION=1",
                    "-DSQLITE_DEFAULT_FOREIGN_KEYS=1", *fts_flags, sqlite, "-c", "-o", obj],
                   cwd=work, env=context, check=True)
    subprocess.run([compiler, "ar", "rcs", archive, obj], cwd=work, env=context, check=True)
    context.update(LIBSQLITE3_SYS_USE_PKG_CONFIG="1", SQLITE3_LIB_DIR=str(work),
                   SQLITE3_INCLUDE_DIR=str(sqlite.parent), SQLITE3_STATIC="1")
    subprocess.run(["cargo", "build", "--offline", "--locked", "--target", "wasm32-wasip1",
                    "--profile", "plugin", "-p", args.package],
                   cwd=ROOT, env=context, check=True)


if __name__ == "__main__":
    main()
