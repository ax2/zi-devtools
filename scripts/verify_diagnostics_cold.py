"""Fresh independent Windows processes; not a Studio startup/Host acceptance test."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

parser=argparse.ArgumentParser()
parser.add_argument('--wasm',type=Path,required=True)
parser.add_argument('--engine-provenance',type=Path,required=True)
parser.add_argument('--fixtures',type=Path)
parser.add_argument('--ids',type=Path)
parser.add_argument('--output',type=Path)
parser.add_argument('--worker',action='store_true')
parser.add_argument('--trace-stages',action='store_true',help='Diagnostic only: retain phase timings even on timeout; never acceptance proof')
args=parser.parse_args()
PHASES = ['worker_ready', 'binding_loaded', 'engine_verified', 'engine_ready',
          'module_compiled', 'instantiated', 'executed']


def decode_stages(stderr):
    if not args.trace_stages:
        return [], not stderr
    try:
        stages = [json.loads(line) for line in stderr.decode('utf-8').splitlines()]
        names = [stage['phase'] for stage in stages]
        times = [stage['elapsedMs'] for stage in stages]
        valid = (names == PHASES[:len(names)]
                 and all(stage.get('kind') == 'cold-stage' for stage in stages)
                 and all(type(value) in (int, float) and value >= 0 for value in times)
                 and times == sorted(times))
        return stages, valid
    except (ValueError, UnicodeError, KeyError, TypeError):
        return [], False


if args.worker:
    start=time.perf_counter()
    def mark(phase):
        if args.trace_stages:
            print(json.dumps(dict(kind='cold-stage',phase=phase,
                                  elapsedMs=round((time.perf_counter()-start)*1000,3))),
                  file=sys.stderr,flush=True)
    mark('worker_ready')
    import wasmtime
    from wasmtime import _ffi
    mark('binding_loaded')
    identity=json.loads(args.engine_provenance.read_text(encoding='utf-8'))
    assert Path(_ffi.dll._name).resolve()==Path(identity['dllPath']).resolve()
    assert hashlib.sha256(Path(_ffi.dll._name).read_bytes()).hexdigest()==identity['dllSha256']
    mark('engine_verified')
    config=wasmtime.Config();config.consume_fuel=True;config.max_wasm_stack=2*1024*1024
    engine=wasmtime.Engine(config)
    engine_ready=time.perf_counter()
    mark('engine_ready')
    module=wasmtime.Module(engine,args.wasm.read_bytes())
    compiled=time.perf_counter()
    mark('module_compiled')
    request=sys.stdin.buffer.read(49153)
    with tempfile.TemporaryDirectory() as temporary:
        folder=Path(temporary);(folder/'stdin').write_bytes(request)
        wasi=wasmtime.WasiConfig();wasi.stdin_file=str(folder/'stdin');wasi.stdout_file=str(folder/'stdout');wasi.stderr_file=str(folder/'stderr')
        with wasmtime.Store(engine) as store:
            store.set_fuel(10_000_000);store.set_limits(memory_size=64*1024*1024);store.set_wasi(wasi)
            linker=wasmtime.Linker(engine);linker.define_wasi()
            instance=linker.instantiate(store,module);instantiated=time.perf_counter()
            mark('instantiated')
            instance.exports(store)['_start'](store);executed=time.perf_counter()
            mark('executed')
            assert not (folder/'stderr').read_bytes()
            result=(folder/'stdout').read_bytes();assert len(result)<=49152
            print(json.dumps(dict(engineSetupMs=round((engine_ready-start)*1000,3),compileMs=round((compiled-engine_ready)*1000,3),instantiateMs=round((instantiated-compiled)*1000,3),executeMs=round((executed-instantiated)*1000,3),fuelUsed=10_000_000-store.get_fuel(),resultSha256=hashlib.sha256(result).hexdigest())))
else:
    assert args.fixtures and args.output
    proof=json.loads((args.fixtures/'fuel-proof.json').read_text(encoding='utf-8'))
    available={case['id'] for case in proof['cases']}
    assert len(available)==len(proof['cases']), 'Duplicate fixture IDs'
    ids=None
    if args.ids:
        selected=json.loads(args.ids.read_text(encoding='utf-8'))
        assert isinstance(selected,list) and selected and all(isinstance(item,str) for item in selected), 'Nonempty ID list required'
        ids=set(selected)
        assert len(ids)==len(selected) and ids <= available, 'Duplicate or unknown selected ID'
    rows=[]
    for case in proof['cases']:
        if ids is not None and case['id'] not in ids:continue
        request=(args.fixtures/case['requestFile']).read_bytes()
        assert hashlib.sha256(request).hexdigest()==case['requestSha256']
        start=time.perf_counter()
        command=[sys.executable,str(Path(__file__).resolve()),'--worker','--wasm',str(args.wasm.resolve()),'--engine-provenance',str(args.engine_provenance.resolve())]
        if args.trace_stages:command.append('--trace-stages')
        process=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
        try:
            stdout,stderr=process.communicate(request,timeout=5)
            stages,stderr_valid=decode_stages(stderr)
            if args.trace_stages:stderr_valid &= len(stages)==len(PHASES)
            measured=json.loads(stdout) if process.returncode==0 and stderr_valid else {}
            elapsed=time.perf_counter()-start
            passed=process.returncode==0 and measured.get('resultSha256')==case['expectedSha256'] and elapsed<5
            rows.append(dict(id=case['id'],pid=process.pid,closed=True,elapsedMs=round(elapsed*1000,3),passed=passed,exitCode=process.returncode,**measured))
            if args.trace_stages:rows[-1].update(stages=stages,stageTraceValid=stderr_valid)
        except subprocess.TimeoutExpired:
            process.kill();stdout,stderr=process.communicate()
            rows.append(dict(id=case['id'],pid=process.pid,closed=True,passed=False,reason='independent_process_deadline',elapsedMs=round((time.perf_counter()-start)*1000,3)))
            if args.trace_stages:
                stages,stderr_valid=decode_stages(stderr)
                rows[-1].update(stages=stages,stageTraceValid=stderr_valid)
        finally:
            if process.poll() is None:process.kill();process.communicate()
        print(f"{case['id']}: {rows[-1]['elapsedMs']}ms pass={rows[-1]['passed']}",flush=True)
    assert len(rows)==(len(ids) if ids is not None else len(proof['cases'])), 'Incomplete replay'
    result=dict(diagnosticOnly=args.trace_stages,verifierSha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),wasmBytes=args.wasm.stat().st_size,engine=json.loads(args.engine_provenance.read_text(encoding='utf-8')),rows=rows,remaining=0,failures=[r['id'] for r in rows if not r['passed']],limitations=['Fresh Python/C-API process, not Studio Windows executable startup or Worker request reconstruction','Same raw portable requests, no skipped cases promoted to Host passes','Five-second independent outer deadline; timing includes process scheduling and engine compilation; no larger budget','Stage tracing changes timing and is diagnostic only, not acceptance proof'])
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    assert not result['failures'],result['failures']
