"""Manual complete-cost comparison, never a substitute for 10M acceptance.

The larger diagnostic fuel ceiling only lets us measure complete compilation.
Memory/stack/deadline remain bounded. Report both costs and the original 10M
threshold; retain the ordinary acceptance runner's failures separately.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import time

import wasmtime
from wasmtime import _ffi

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--baseline', type=Path, required=True)
parser.add_argument('--candidate', type=Path, required=True)
parser.add_argument('--fixtures', type=Path, required=True)
parser.add_argument('--engine-provenance', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--scaling-native', type=Path,
                    help='Also measure repeat-count scaling against this actual native reference')
args = parser.parse_args()
load = lambda path: json.loads(path.read_text(encoding='utf-8'))
sha = lambda data: hashlib.sha256(data).hexdigest()
provenance = load(args.engine_provenance)
assert Path(_ffi.dll._name).resolve() == Path(provenance['dllPath']).resolve()
assert sha(Path(_ffi.dll._name).read_bytes()) == provenance['dllSha256']
ceiling = 1_000_000_000
config = wasmtime.Config()
config.consume_fuel = True
config.epoch_interruption = True
config.max_wasm_stack = 2 * 1024 * 1024
engine = wasmtime.Engine(config)
modules = {}
for name, path in [('baseline', args.baseline), ('candidate', args.candidate)]:
    data = path.read_bytes()
    assert len(data) <= 2 * 1024 * 1024
    modules[name] = (wasmtime.Module(engine, data), sha(data))
cases = [r for r in load(args.fixtures/'fuel-proof.json')['cases']
         if r['id'] == 'regex-large-valid-repeat' or r['id'].startswith('regex-repeat-match-')]
assert len(cases) == 11
requests = {}
if args.scaling_native:
    for count in [1000, 5000, 10000, 20000, 50000, 100000, 200000, 300000, 400000]:
        body = json.dumps(dict(contractVersion='1.1.0-proposal.2',
            pluginId='com.zicode.devtools.compare', sceneId='coding',
            capabilityId='devtools.compare.regex.matches', commandId=None,
            input=dict(text='', pattern=f'a{{{count}}}')),
            ensure_ascii=False, separators=(',', ':')).encode('utf-8')
        reference = subprocess.run([str(args.scaling_native)], input=body,
                                   capture_output=True, timeout=20)
        assert reference.returncode == 0 and not reference.stderr
        envelope = json.loads(reference.stdout)
        assert envelope['contractVersion'] == '1.1.0-proposal.2'
        assert isinstance(envelope['ok'], bool)
        case = dict(id=f'regex-scaling-{count}', requestSha256=sha(body),
                    expectedSha256=sha(reference.stdout), repeatCount=count,
                    reference='actual_native_adapter', nativeAccepted=envelope['ok'])
        cases.append(case)
        requests[case['id']] = (body, reference.stdout)
rows = []
for case in cases:
    if case['id'] in requests:
        body, expected = requests[case['id']]
    else:
        body = (args.fixtures/case['requestFile']).read_bytes()
        expected = (args.fixtures/case['expectedFile']).read_bytes()
    assert sha(body) == case['requestSha256'] and sha(expected) == case['expectedSha256']
    row = dict(id=case['id'], requestSha256=sha(body), expectedSha256=sha(expected))
    if 'repeatCount' in case:
        row.update(repeatCount=case['repeatCount'], reference=case['reference'],
                   nativeAccepted=case['nativeAccepted'],
                   expectedResult=expected.decode('utf-8'))
    for name, (module, digest) in modules.items():
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            (folder/'stdin').write_bytes(body)
            wasi = wasmtime.WasiConfig()
            wasi.stdin_file = str(folder/'stdin')
            wasi.stdout_file = str(folder/'stdout')
            wasi.stderr_file = str(folder/'stderr')
            with wasmtime.Store(engine) as store:
                store.set_limits(memory_size=64*1024*1024)
                store.set_fuel(ceiling)
                store.set_epoch_deadline(1)
                store.set_wasi(wasi)
                linker = wasmtime.Linker(engine)
                linker.define_wasi()
                timer = threading.Timer(5, engine.increment_epoch)
                instance = None
                trap = None
                started = time.monotonic()
                timer.start()
                try:
                    instance = linker.instantiate(store, module)
                    instance.exports(store)['_start'](store)
                except Exception as error:
                    trap = str(error).splitlines()[-1]
                finally:
                    timer.cancel()
                    timer.join()
                elapsed = time.monotonic()-started
                fuel = ceiling-store.get_fuel()
                output = (folder/'stdout').read_bytes()
                stderr = (folder/'stderr').read_bytes()
                memory = None if instance is None else instance.exports(store)['memory'].data_len(store)
        matches = trap is None and not stderr and output == expected
        row[name] = dict(wasmSha256=digest, fuelUsed=fuel, memoryBytes=memory,
                         elapsedMs=round(elapsed*1000, 3), trap=trap, expectedResultMatches=matches,
                         withinOriginalFuel=matches and fuel <= 10_000_000)
    row['fuelSaved'] = row['baseline']['fuelUsed']-row['candidate']['fuelUsed']
    rows.append(row)
    print(case['id'], 'baseline', row['baseline']['fuelUsed'], 'candidate', row['candidate']['fuelUsed'], flush=True)
proof = dict(diagnosticOnly=True, diagnosticFuelCeiling=ceiling, originalAcceptanceFuel=10_000_000,
             engineProvenance=provenance, rows=rows,
             limitations=['Larger fuel ceiling measures complete cost; not acceptance or budget negotiation',
                          'Warm engine comparison; not cold-start or actual Host/Pi proof',
                          '11 fixed cases plus optional native-referenced scaling, not full regex performance coverage'])
if args.scaling_native:
    proof['nativeSha256'] = sha(args.scaling_native.read_bytes())
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
raise SystemExit(0 if all(r[n]['expectedResultMatches'] for r in rows for n in modules) else 1)
