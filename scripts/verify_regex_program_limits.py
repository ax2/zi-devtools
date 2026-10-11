"""Probe native64/WASI32 regex program-limit parity; not fuel or Host acceptance.

Keep this diagnostic separate from the small fixed functional matrix. The
original three expectations remain fixed; additional compiler shapes use the
actual native adapter as the byte-for-byte reference, not a guessed size limit.
This is an unbudgeted diagnostic, not fuel, cold-start or Host acceptance.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import subprocess
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native', type=Path, default=ROOT/'target/debug/zi-fields-wasi.exe')
parser.add_argument('--wasm', type=Path, default=ROOT/'target/wasm32-wasip1/plugin/zi-fields-wasi.wasm')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
validator = Draft202012Validator(json.loads(
    (ROOT/'proposals/studio-fields/v0.1.1/result.schema.json').read_text(encoding='utf-8')))

def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')

cases = [(f'original-{count}', f'a{{{count}}}', expected_ok)
         for count, expected_ok in [(200000, True), (400000, False), (700000, False)]]
# These are distinct compiler paths: captures, bounded/lazy repetition, byte
# classes, UTF-8 sequences, assertions, alternation and nested repetition.
# Do not infer their accepted size from the single ASCII literal threshold.
cases += [(name, pattern, None) for name, pattern in [
    ('capture', '(a){200000}'),
    ('noncapture', '(?:a){400000}'),
    ('bounded', 'a{300000,400000}'),
    ('lazy', 'a{300000,400000}?'),
    ('byte-class', '[ab]{400000}'),
    ('unicode-literal', '中{150000}'),
    ('unicode-class', '[中日]{150000}'),
    ('anchored', '^a{400000}$'),
    ('alternation', '(?:ab|cd){100000}'),
    ('nested', '(?:a{1000}){400}'),
    ('dead-repetition', '(?:a{400000}){0}'),
    ('optional-repetition', '(?:a{400000})?'),
]]
cases = [(name, pattern, expected_ok, '') for name, pattern, expected_ok in cases]
# Exercise accepted repetitions against real text as well as compiling them.
# In particular, prefixes/suffixes and alternatives can select reverse search
# paths, while captures must keep the original byte offsets and clipping.
cases += [(name, pattern, None, text) for name, pattern, text in [
    ('match-long-ascii', 'a{20000}', 'a' * 20000),
    ('match-long-insufficient', 'a{20000}', 'a' * 19999),
    ('match-long-prefix', 'xa{20000}', 'zxa' + 'a' * 19999),
    ('match-long-suffix', 'a{20000}z', 'x' + 'a' * 20000 + 'z'),
    ('match-long-unicode', '中{10000}', 'x' + '中' * 10000),
    ('match-long-unicode-suffix', '(?:中日){5000}z', 'x' + '中日' * 5000 + 'z'),
    ('match-long-capture', '(a{20000})', 'x' + 'a' * 20000),
    ('match-long-dead-capture', '(a){0}b{20000}', 'b' * 20000),
    ('match-long-alternation', '(?:a{20000}z|a{20000}y)', 'a' * 20000 + 'y'),
    ('match-long-bounded', 'a{19999,20000}', 'a' * 20000),
    ('match-long-lazy', 'a{19999,20000}?', 'a' * 20000),
    ('match-zero-repeat', 'a{0}', '中a'),
]]

def invoke(command, body, wasi=False):
    try:
        result = subprocess.run(command, input=body, capture_output=True, timeout=20)
        value = json.loads(result.stdout)
        validator.validate(value)
        stderr_valid = not result.stderr
        runner = None
        if wasi:
            lines = result.stderr.decode('utf-8').splitlines()
            runner = json.loads(lines[-1])
            warning = lines[:-1]
            stderr_valid = (not warning or (len(warning) == 2 and re.fullmatch(
                r'\(node:\d+\) ExperimentalWarning: WASI is an experimental feature and might change at any time',
                warning[0]) is not None and warning[1] ==
                '(Use `node --trace-warnings ...` to show where the warning was created)'))
            stderr_valid &= (runner.get('exitCode') == 0 and
                             runner.get('procExit') in (None, 0) and
                             isinstance(runner.get('memoryBytes'), int))
        return dict(exitCode=result.returncode, result=result.stdout.decode('utf-8'),
                    stderrValid=stderr_valid, runner=runner,
                    passed=result.returncode == 0 and stderr_valid)
    except subprocess.TimeoutExpired:
        return dict(passed=False, reason='diagnostic_timeout')
    except (ValueError, UnicodeError, IndexError):
        return dict(passed=False, reason='invalid_result_json')
    except ValidationError:
        return dict(passed=False, reason='invalid_result_schema')
    except OSError:
        return dict(passed=False, reason='adapter_launch_failed')

rows = []
for name, pattern, expected_ok, text in cases:
    body = encoded(dict(contractVersion='1.1.0-proposal.2', pluginId='com.zicode.devtools.compare',
                        sceneId='coding', capabilityId='devtools.compare.regex.matches', commandId=None,
                        input=dict(text=text, pattern=pattern)))
    expected = None if expected_ok is None else encoded(dict(contractVersion='1.1.0-proposal.2', ok=expected_ok,
                            data=dict(text='没有匹配') if expected_ok else None,
                            error=None if expected_ok else dict(code='INVALID_INPUT', message='输入无法按该操作处理')))
    row = dict(id=name, pattern=pattern, inputBytes=len(text.encode('utf-8')),
               inputSha256=hashlib.sha256(text.encode('utf-8')).hexdigest(),
               requestSha256=hashlib.sha256(body).hexdigest(),
               reference='fixed_original_expectation' if expected is not None else 'actual_native_adapter')
    row['native'] = invoke([str(args.native)], body)
    if expected is not None:
        row['native']['passed'] &= row['native'].get('result', '').encode('utf-8') == expected
    elif row['native']['passed']:
        expected = row['native']['result'].encode('utf-8')
    row['expectedNativeResult'] = None if expected is None else expected.decode('utf-8')
    row['wasi'] = invoke(['node', str(ROOT/'scripts/run_text_wasi.mjs'), str(args.wasm)], body, wasi=True)
    row['wasi']['passed'] &= expected is not None and row['wasi'].get('result', '').encode('utf-8') == expected
    row['passed'] = row['native']['passed'] and row['wasi']['passed']
    rows.append(row)
    print(f"{name} {pattern}: native={row['native']['passed']} wasi={row['wasi']['passed']}", flush=True)

proof = dict(rows=rows, failures=[r['id'] for r in rows if not r['passed']],
             cargoLockSha256=hashlib.sha256((ROOT/'Cargo.lock').read_bytes()).hexdigest(),
             nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),
             wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),
             limitations=['Unbudgeted functional diagnostic; Node is not fuel or Host isolation proof',
                          'Three fixed original native-engine expectations plus actual-native compiler-shape and matching references',
                          'Selected empty and long-text cases; this is not complete regex semantics or matching-performance coverage'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
raise SystemExit(1 if proof['failures'] else 0)
