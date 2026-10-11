// Standalone diagnostic runner, not a sandbox or production Host.
import { readFileSync } from 'node:fs';
import { WASI } from 'node:wasi';

const [path, mode, clockPolicy = 'allow'] = process.argv.slice(2);
if (!path || !['clock', 'expired', 'compare', 'limits'].includes(mode) || !['allow', 'deny'].includes(clockPolicy)) {
  throw new Error('expected module, mode, allow|deny');
}
const wasi = new WASI({version:'preview1', args:['clock_probe', mode], env:{}, preopens:{}, returnOnExit:true});
const module = await WebAssembly.compile(readFileSync(path));
const importList = WebAssembly.Module.imports(module);
const imports = wasi.getImportObject();
const originalClock = imports.wasi_snapshot_preview1.clock_time_get;
let monotonicReads = 0;
let deniedMonotonicReads = 0;
imports.wasi_snapshot_preview1.clock_time_get = (...args) => {
  if (args[0] === 1) {
    monotonicReads++;
    if (clockPolicy === 'deny') {
      deniedMonotonicReads++;
      // WASI Preview1 ERRNO_NOSYS=52, as generated in wasi 0.11.1.
      // Return an actual syscall error so Rust handles the unavailable clock.
      return 52;
    }
  }
  return originalClock(...args);
};
let exitCode = 1;
let failure = null;
let memoryBytes = null;
try {
  const instance = await WebAssembly.instantiate(module, imports);
  exitCode = wasi.start(instance) ?? 0;
  memoryBytes = instance.exports.memory.buffer.byteLength;
} catch (error) {
  failure = error.message;
}
process.stderr.write(JSON.stringify({diagnosticOnly:true, mode, clockPolicy, monotonicReads, deniedMonotonicReads,
  exitCode, failure, memoryBytes, imports:importList}) + '\n');
process.exitCode = exitCode;
