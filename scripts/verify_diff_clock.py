"""Verify a standalone native/WASI diff clock probe, never Host acceptance."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native', type=Path, default=ROOT / 'target/debug/examples/clock_probe.exe')
parser.add_argument('--wasm', type=Path, default=ROOT / 'target/wasm32-wasip1/plugin/examples/clock_probe.wasm')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()


def run(command):
    try:
        result = subprocess.run(command, capture_output=True, timeout=20)
        return dict(exitCode=result.returncode, closed=True,
                    stdout=result.stdout.decode('utf-8'), stderr=result.stderr.decode('utf-8'))
    except subprocess.TimeoutExpired:
        # subprocess.run kills and waits for its direct child on timeout.
        return dict(exitCode=None, closed=True, stdout='', stderr='', failure='probe_timeout')


rows = []
for mode in ['clock', 'expired', 'compare', 'limits']:
    native = run([str(args.native), mode])
    wasi = run(['node', '--no-warnings', str(ROOT / 'scripts/run_diff_clock_probe.mjs'),
                str(args.wasm), mode, 'allow'])
    try:
        meta = json.loads(wasi['stderr'])
    except ValueError:
        meta = {}
    imported_clock = any(item == dict(module='wasi_snapshot_preview1', name='clock_time_get', kind='function')
                         for item in meta.get('imports', []))
    passed = (native['exitCode'] == 0 and not native['stderr'] and wasi['exitCode'] == 0
              and native['stdout'] == wasi['stdout'] and bool(native['stdout'])
              and meta.get('diagnosticOnly') is True and meta.get('failure') is None
              and meta.get('mode') == mode and meta.get('clockPolicy') == 'allow'
              and meta.get('exitCode') == 0 and meta.get('monotonicReads', 0) > 0
              and meta.get('deniedMonotonicReads') == 0
              and imported_clock)
    if mode == 'expired':
        # One read creates Instant; further reads must come from the algorithm.
        passed &= meta.get('monotonicReads', 0) >= 2
    if mode == 'compare':
        passed &= len(native['stdout'].splitlines()) == 36
    rows.append(dict(id=mode, passed=passed, native=native, wasi=wasi, metadata=meta))

denied = run(['node', '--no-warnings', str(ROOT / 'scripts/run_diff_clock_probe.mjs'),
              str(args.wasm), 'compare', 'deny'])
try:
    # Rust can print its panic before the runner's final diagnostic record.
    denied_meta = json.loads(denied['stderr'].splitlines()[-1])
except (ValueError, IndexError):
    denied_meta = {}
rows.append(dict(id='compare-clock-denied', passed=(denied['exitCode'] not in (0, None)
                 and denied_meta.get('diagnosticOnly') is True
                 and denied_meta.get('exitCode') not in (0, None)
                 and denied_meta.get('mode') == 'compare'
                 and denied_meta.get('clockPolicy') == 'deny'
                 and denied_meta.get('deniedMonotonicReads', 0) > 0),
                 wasi=denied, metadata=denied_meta))
proof = dict(diagnosticOnly=True, rows=rows, failures=[row['id'] for row in rows if not row['passed']],
             nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),
             wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),
             wasmBytes=args.wasm.stat().st_size,
             runnerSha256=hashlib.sha256((ROOT / 'scripts/run_diff_clock_probe.mjs').read_bytes()).hexdigest(),
             verifierSha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
             nodeVersion=subprocess.run(['node', '--version'], capture_output=True, check=True).stdout.decode('ascii').strip(),
             cargoLockSha256=hashlib.sha256((ROOT / 'Cargo.lock').read_bytes()).hexdigest(),
             limitations=['Node functional clock diagnostic only; no fuel or cold-start acceptance',
                          'Expired-deadline reconstruction does not require identical approximate edit scripts',
                          'No plugin protocol, View, installation or production Host clock negotiation'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f"Diff clock diagnostic: {len(rows) - len(proof['failures'])}/{len(rows)} passed")
raise SystemExit(1 if proof['failures'] else 0)
