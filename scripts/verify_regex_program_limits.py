"""Probe native64/WASI32 regex program-limit parity; not fuel or Host acceptance.

This diagnostic currently fails at a{400000}. Keep it separate from the fixed
functional matrix, and do not present that matrix as complete regex parity.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native', type=Path, default=ROOT/'target/debug/zi-fields-wasi.exe')
parser.add_argument('--wasm', type=Path, default=ROOT/'target/wasm32-wasip1/plugin/zi-fields-wasi.wasm')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()

def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')

rows = []
for count, expected_ok in [(200000, True), (400000, False), (700000, False)]:
    body = encoded(dict(contractVersion='1.1.0-proposal.2', pluginId='com.zicode.devtools.compare',
                        sceneId='coding', capabilityId='devtools.compare.regex.matches', commandId=None,
                        input=dict(text='', pattern=f'a{{{count}}}')))
    expected = encoded(dict(contractVersion='1.1.0-proposal.2', ok=expected_ok,
                            data=dict(text='没有匹配') if expected_ok else None,
                            error=None if expected_ok else dict(code='INVALID_INPUT', message='输入无法按该操作处理')))
    row = dict(repetition=count, requestSha256=hashlib.sha256(body).hexdigest(),
               expectedNativeResult=expected.decode('utf-8'))
    for name, command in [('native', [str(args.native)]),
                          ('wasi', ['node', str(ROOT/'scripts/run_text_wasi.mjs'), str(args.wasm)])]:
        try:
            result = subprocess.run(command, input=body, capture_output=True, timeout=20)
            row[name] = dict(exitCode=result.returncode, result=result.stdout.decode(errors='replace'),
                             passed=result.returncode == 0 and result.stdout == expected)
        except subprocess.TimeoutExpired:
            row[name] = dict(passed=False, reason='diagnostic_timeout')
    row['passed'] = row['native']['passed'] and row['wasi']['passed']
    rows.append(row)
    print(f"a{{{count}}}: native={row['native']['passed']} wasi={row['wasi']['passed']}")

proof = dict(rows=rows, failures=[r['repetition'] for r in rows if not r['passed']],
             nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),
             wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),
             limitations=['Unbudgeted functional diagnostic; Node is not fuel or Host isolation proof',
                          'Three original native-engine boundary expectations, not complete regex semantics'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
raise SystemExit(1 if proof['failures'] else 0)
