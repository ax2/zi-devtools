"""Independent Wasmtime budget runner. Does not claim Studio Host acceptance."""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import time

import wasmtime

parser = argparse.ArgumentParser()
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--native', type=Path)
parser.add_argument('--fixtures', type=Path)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--allow-traps', action='store_true')
parser.add_argument('--serializer-boundary', action='store_true')
args = parser.parse_args()
portable={}
if args.fixtures:
    saved=json.loads((args.fixtures/'fuel-proof.json').read_text(encoding='utf-8'))
    cases=[]
    for case in saved['cases']:
        request=(args.fixtures/case['requestFile']).read_bytes()
        expected=(args.fixtures/case['expectedFile']).read_bytes()
        assert hashlib.sha256(request).hexdigest()==case['requestSha256']
        assert hashlib.sha256(expected).hexdigest()==case['expectedSha256']
        portable[case['id']]=expected
        cases.append((case['id'],request,case['checks'],case['expectedError']))
else:
    from diagnostics_vectors import cases
assert args.native or args.fixtures, 'Native adapter or validated portable expectations required'
if args.serializer_boundary:
    cases=[]
    for kind,prefix in [('ascii',''),('utf8','中'),('escaped','中\0"\\')]:
        value=dict(contractVersion='1.0.0-rc.1',ok=True,data=dict(text=prefix),error=None)
        overhead=len(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode())
        for size in [49151,49152,49153]:
            envelope=json.loads(json.dumps(value))
            envelope['data']['text']+='a'*(size-overhead)
            cases.append((f'serializer-{kind}-{size}',envelope,{},'OUTPUT_TOO_LARGE' if size>49152 else None))
args.output.mkdir(parents=True, exist_ok=True)
config = wasmtime.Config()
config.consume_fuel = True
config.epoch_interruption = True
config.max_wasm_stack = 2 * 1024 * 1024
engine = wasmtime.Engine(config)
module_bytes = args.wasm.read_bytes()
assert len(module_bytes) <= 2 * 1024 * 1024
module = wasmtime.Module(engine, module_bytes)
rows = []
failures = []
for index, (name, value, checks, error) in enumerate(cases):
    data = value if isinstance(value, bytes) else json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()
    with tempfile.TemporaryDirectory() as temporary:
        folder = Path(temporary)
        (folder/'stdin').write_bytes(data)
        wasi = wasmtime.WasiConfig()
        wasi.stdin_file = str(folder/'stdin')
        wasi.stdout_file = str(folder/'stdout')
        wasi.stderr_file = str(folder/'stderr')
        with wasmtime.Store(engine) as store:
            store.set_limits(memory_size=64*1024*1024)
            store.set_fuel(10_000_000)
            store.set_epoch_deadline(1)
            store.set_wasi(wasi)
            linker = wasmtime.Linker(engine)
            linker.define_wasi()
            timer = threading.Timer(5, engine.increment_epoch)
            start = time.monotonic()
            trap = None
            timer.start()
            try:
                instance = linker.instantiate(store, module)
                instance.exports(store)['_start'](store)
            except Exception as ex:
                trap = str(ex).splitlines()[-1]
            finally:
                timer.cancel()
                timer.join()
            elapsed = time.monotonic()-start
            remaining = store.get_fuel()
            output = (folder/'stdout').read_bytes()
            stderr = (folder/'stderr').read_bytes()
            memory = instance.exports(store)['memory'].data_len(store) if trap is None else None
    if args.native:
        native=subprocess.run([str(args.native.resolve())],input=data,capture_output=True,timeout=5,check=True)
        assert not native.stderr
        expected=native.stdout
        if portable:assert expected==portable[name]
    else:expected=portable[name]
    envelope = json.loads(expected)
    assert len(expected) <= 48*1024
    if error:
        assert not envelope['ok'] and envelope['data'] is None and envelope['error']['code'] == error, (name, envelope)
    else:
        assert envelope['ok'] and envelope['error'] is None, (name, envelope)
        report = {} if args.serializer_boundary else json.loads(envelope['data']['text'])
        for path, wanted in checks.items():
            actual = report
            for part in path.split('/'):
                actual = actual[int(part)] if isinstance(actual, list) else actual[part]
            assert actual == wanted, (name, path, actual, wanted)
    success = trap is None and not stderr and output == expected and elapsed < 5 and memory <= 64*1024*1024
    if not success:
        failures.append(name)
    stem = f'{index:03d}-{name}'
    (args.output/(stem+'.request')).write_bytes(data)
    (args.output/(stem+'.expected.json')).write_bytes(expected)
    rows.append(dict(id=name,passBudget=success,trap=trap,fuelUsed=10_000_000-remaining,fuelRemaining=remaining,elapsedMs=round(elapsed*1000),memoryBytes=memory,requestBytes=len(data),resultBytes=len(output),requestFile=stem+'.request',expectedFile=stem+'.expected.json',requestSha256=hashlib.sha256(data).hexdigest(),expectedSha256=hashlib.sha256(expected).hexdigest(),checks=checks,expectedError=error))
    print(f'{name}: fuel={10_000_000-remaining} pass={success}', flush=True)
proof = dict(fixtureKind='shared-serializer-only' if args.serializer_boundary else 'plugin-runtime',wasmBytes=len(module_bytes),wasmSha256=hashlib.sha256(module_bytes).hexdigest(),wasmtimeVersion=importlib.metadata.version('wasmtime'),budget=dict(fuel=10_000_000,deadlineSeconds=5,memoryBytes=64*1024*1024,guestStackBytes=2*1024*1024),cases=rows,failures=failures,limitations=['Independent Wasmtime runner; fuel accounting depends on runtime version','Not Studio installation, Pi, permission, View or Host acceptance','No inherited environment or preopened directories; stdout is a runner file, not production pipe testing'])
(args.output/'fuel-proof.json').write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
assert args.allow_traps or not failures, failures
