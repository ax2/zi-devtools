"""Diagnose bootstrap imports for failed cold cases, never acceptance evidence.

Keeps the five-second outer deadline. Import tracing adds overhead and stderr;
these measurements cannot replace ordinary cold proofs or erase their failures.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--fixtures', type=Path, required=True)
parser.add_argument('--engine-provenance', type=Path, required=True)
parser.add_argument('--ids', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
load = lambda p: json.loads(p.read_text(encoding='utf-8'))
proof = load(args.fixtures / 'fuel-proof.json')
module_sha = hashlib.sha256(args.wasm.read_bytes()).hexdigest()
assert proof['wasmSha256'] == module_sha
ids = set(load(args.ids))
assert ids and ids <= {case['id'] for case in proof['cases']}
rows = []
for case in proof['cases']:
    if case['id'] not in ids:
        continue
    request = (args.fixtures / case['requestFile']).read_bytes()
    assert hashlib.sha256(request).hexdigest() == case['requestSha256']
    started = time.perf_counter()
    child = subprocess.Popen([sys.executable, '-X', 'importtime',
        str(Path(__file__).with_name('verify_diagnostics_cold.py')),
        '--worker', '--wasm', str(args.wasm.resolve()),
        '--engine-provenance', str(args.engine_provenance.resolve())],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    spawned = time.perf_counter()
    timed_out = False
    try:
        stdout, stderr = child.communicate(request, timeout=max(0.001, 5 - (spawned - started)))
    except subprocess.TimeoutExpired:
        timed_out = True
        child.kill()
        stdout, stderr = child.communicate()
    finally:
        if child.poll() is None:
            child.kill(); child.communicate()
    finished = time.perf_counter()
    try:
        measured = json.loads(stdout) if child.returncode == 0 else {}
    except ValueError:
        measured = {}
    imports = [line for line in stderr.decode('utf-8', errors='replace').splitlines() if line.startswith('import time:')]
    row = dict(id=case['id'], pid=child.pid, closed=child.poll() is not None,
               elapsedMs=round((finished - started) * 1000, 3),
               processSpawnMs=round((spawned - started) * 1000, 3),
               timedOut=timed_out, exitCode=child.returncode,
               expectedResultMatches=measured.get('resultSha256') == case['expectedSha256'],
               workerMeasurements=measured, importTimings=imports)
    rows.append(row)
    print(f"{case['id']}: total={row['elapsedMs']}ms spawn={row['processSpawnMs']}ms timedOut={timed_out}", flush=True)
result = dict(wasmSha256=module_sha, diagnosticOnly=True, rows=rows, remaining=0,
              limitations=['Python import tracing adds overhead; not cold-budget acceptance',
                           'Only requested diagnostic cases; not a full replay',
                           'Five-second outer deadline retained; original failures remain authoritative'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
