"""Differential syntax checks against a hash-pinned pre-acceleration WASI.

This is an unbudgeted functional check, not Host, cold or performance proof.
The old module is an independent algorithm reference; native/WASI parity alone
would let a bug shared by both new adapters go unnoticed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--reference', type=Path, required=True)
parser.add_argument('--reference-sha256', required=True)
parser.add_argument('--native', type=Path, default=ROOT/'target/debug/zi-fields-wasi.exe')
parser.add_argument('--wasm', type=Path, default=ROOT/'target/wasm32-wasip1/plugin/zi-fields-wasi.wasm')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
sha = lambda data: hashlib.sha256(data).hexdigest()
assert sha(args.reference.read_bytes()) == args.reference_sha256.lower()
validator = Draft202012Validator(json.loads(
    (ROOT/'proposals/studio-fields/v0.1.1/result.schema.json').read_text(encoding='utf-8')))
patterns = [
    'a{2}', 'a{2,2}', 'a{2,2}?', '(?:ab){3}', '(?:aba){2}', '中{2}', '🙂{2}',
    r'\x61{2}', r'\x{61}{2}', r'\u{4E2D}{2}', '[a]{2}', '[中]{2}',
    '(?i:a){2}', '(?i:k){2}', '(?i:1){2}', '(?i:中){2}',
    '(?m:a){2}', '(?s:a){2}', '(?U:a){2}', '(?x:a){2}',
    '(?x) a { 2 }', '(?-u:a){2}', r'(?-u:\xFF){2}',
    r'\.{2}', r'\{{2}', r'\}{2}', r'\n{2}', r'\x00{2}',
    '(a){0}b{2}', '(?P<dead>a){0}b{2}', '(?:a{2}){2}', '(a{2}){2}',
    'a{0}', 'a{1}', 'a{0,2}', 'a{2,3}', 'a{2,}', '[ab]{2}',
    '^a{2}$', 'a{2}|b{2}', 'a{2}()', '()a{2}', '(a){2}',
    'a{2}(', 'a{2,1}', 'a{4294967296}', r'(a)\1', r'(?=a)a{2}',
]
inputs = ['', 'a', 'aa', 'aaa', 'aaaaa', 'ababaababab', '甲aabb乙aa',
          '中中中🙂🙂', 'KKkkKK11', '{{}}..\n\n\0\0', 'aa\naa']
commands = dict(
    reference=['node', str(ROOT/'scripts/run_text_wasi.mjs'), str(args.reference)],
    native=[str(args.native)],
    wasi=['node', str(ROOT/'scripts/run_text_wasi.mjs'), str(args.wasm)],
)


def invoke(command, request, wasi):
    try:
        process = subprocess.run(command, input=request, capture_output=True, timeout=20)
        validator.validate(json.loads(process.stdout))
        stderr_valid = not process.stderr
        if wasi:
            diagnostic = process.stderr.decode('utf-8')
            diagnostic = re.sub(
                r'^\(node:\d+\) ExperimentalWarning: WASI is an experimental feature and might change at any time\r?\n'
                r'\(Use `node --trace-warnings \.\.\.` to show where the warning was created\)\r?\n',
                '', diagnostic, count=1)
            runner = json.loads(diagnostic)
            stderr_valid = (runner.get('exitCode') == 0 and runner.get('procExit') in (None, 0)
                            and isinstance(runner.get('memoryBytes'), int))
        return dict(passed=process.returncode == 0 and stderr_valid,
                    exitCode=process.returncode, stderrValid=stderr_valid,
                    result=process.stdout.decode('utf-8'), resultSha256=sha(process.stdout))
    except Exception as error:
        # Controlled generated fixtures only; retain a non-sensitive reason
        # and finish the matrix instead of losing prior evidence on an error.
        return dict(passed=False, reason=type(error).__name__)


rows = []
for index, pattern in enumerate(patterns):
    for input_index, text in enumerate(inputs):
        body = json.dumps(dict(contractVersion='1.1.0-proposal.2',
            pluginId='com.zicode.devtools.compare', sceneId='coding',
            capabilityId='devtools.compare.regex.matches', commandId=None,
            input=dict(text=text, pattern=pattern)),
            ensure_ascii=False, separators=(',', ':')).encode('utf-8')
        row = dict(id=f'syntax-{index:02d}-input-{input_index:02d}', pattern=pattern,
                   input=text, requestSha256=sha(body))
        results = {name: invoke(command, body, name != 'native')
                   for name, command in commands.items()}
        reference = results['reference'].get('result')
        row['passed'] = all(result['passed'] and result.get('result') == reference
                            for result in results.values())
        row['results'] = results
        rows.append(row)
    print(f'{index+1}/{len(patterns)} patterns; failures={sum(not r["passed"] for r in rows)}', flush=True)
proof = dict(referenceSha256=sha(args.reference.read_bytes()),
             nativeSha256=sha(args.native.read_bytes()), wasmSha256=sha(args.wasm.read_bytes()),
             rows=rows, failures=[r['id'] for r in rows if not r['passed']],
             limitations=['Pre-acceleration module as differential reference, not all regex semantics',
                          'No fuel accounting, cold-start, production isolation or Host acceptance'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
raise SystemExit(1 if proof['failures'] else 0)
