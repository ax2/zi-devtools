"""Functional proposal.3 parity and unavailable-clock checks, not Host acceptance."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from jsonschema import Draft202012Validator, ValidationError
from compare_vectors import cases, failure, request

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--native', type=Path, default=ROOT / 'target/debug/zi-compare-wasi.exe')
parser.add_argument('--wasm', type=Path, default=ROOT / 'target/wasm32-wasip1/plugin/zi-compare-wasi.wasm')
args = parser.parse_args()
imports = json.loads(subprocess.run(
    ['node', '-e', "const fs=require('node:fs');"
     "const m=new WebAssembly.Module(fs.readFileSync(process.argv[1]));"
     "console.log(JSON.stringify(WebAssembly.Module.imports(m)));", str(args.wasm)],
    capture_output=True, check=True, timeout=20).stdout)
assert imports and all(item['module'] == 'wasi_snapshot_preview1' for item in imports), \
    'Production module must not import diagnostic observers or custom Host APIs'
assert any(item['name'] == 'clock_time_get' for item in imports), 'Monotonic clock import missing'
validator = Draft202012Validator(json.loads(
    (ROOT / 'proposals/studio-fields/v0.1.2/result.schema.json').read_text(encoding='utf-8')))


def invoke(body, expected, policy=None):
    command = ([str(args.native)] if policy is None else
               ['node', '--no-warnings', str(ROOT / 'scripts/run_diff_clock_probe.mjs'),
                str(args.wasm), 'compare', policy])
    try:
        result = subprocess.run(command, input=body, capture_output=True, timeout=20)
        value = json.loads(result.stdout)
        validator.validate(value)
        metadata = None if policy is None else json.loads(result.stderr)
        valid_stderr = (not result.stderr if policy is None else
                        metadata.get('diagnosticOnly') is True and metadata.get('exitCode') == 0
                        and metadata.get('failure') is None and metadata.get('clockPolicy') == policy)
        return dict(passed=result.returncode == 0 and valid_stderr and result.stdout == expected
                    and len(result.stdout) <= 49152,
                    exitCode=result.returncode, closed=True,
                    resultSha256=hashlib.sha256(result.stdout).hexdigest(), metadata=metadata)
    except subprocess.TimeoutExpired:
        return dict(passed=False, closed=True, reason='functional_timeout')
    except (ValueError, UnicodeError, OSError, ValidationError):
        return dict(passed=False, closed=True, reason='invalid_result_or_launch_failure')


proof_rows = []
for name, body, expected in cases:
    native = invoke(body, expected)
    wasi = invoke(body, expected, 'allow')
    proof_rows.append(dict(id=name, expectedSha256=hashlib.sha256(expected).hexdigest(),
                           requestSha256=hashlib.sha256(body).hexdigest(), native=native, wasi=wasi,
                           passed=native['passed'] and wasi['passed']))
    print(f"{name}: {proof_rows[-1]['passed']}", flush=True)

expected = failure('RUNTIME_UNAVAILABLE', '运行环境未提供单调时钟')
denied = invoke(request('a\n', 'b\n'), expected, 'deny')
denied['passed'] &= denied.get('metadata', {}).get('deniedMonotonicReads') == 1
proof_rows.append(dict(id='text-clock-unavailable', wasi=denied, passed=denied['passed']))
# Existing clock-independent operations must remain usable when no clock exists.
for name in ['query', 'ordered', 'unordered', next(case[0] for case in cases if case[0].startswith('regex-'))]:
    _, body, expected = next(case for case in cases if case[0] == name)
    denied = invoke(body, expected, 'deny')
    denied['passed'] &= denied.get('metadata', {}).get('deniedMonotonicReads') == 0
    proof_rows.append(dict(id=name + '-without-clock', wasi=denied, passed=denied['passed']))

proof = dict(diagnosticOnly=True, rows=proof_rows, failures=[r['id'] for r in proof_rows if not r['passed']],
             productionImports=imports,
             wasmBytes=args.wasm.stat().st_size,
             wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),
             nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),
             limitations=['Functional Node WASI only, not original fuel or cold-start budget proof',
                          'No production Host negotiation, View or installation acceptance'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f"Proposal.3 functional vectors: {len(proof_rows)-len(proof['failures'])}/{len(proof_rows)} passed")
raise SystemExit(1 if proof['failures'] else 0)
