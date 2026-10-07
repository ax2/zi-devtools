// Functional verification runner for our own module, NOT a production sandbox.
// https://nodejs.org/api/wasi.html; Node supplies no fuel accounting.
import { readFileSync } from 'node:fs';
import { WASI } from 'node:wasi';
const wasi = new WASI({version:'preview1', args:[], env:{}, preopens:{}, returnOnExit:true});
const module = await WebAssembly.compile(readFileSync(process.argv[2]));
let exited = null;
const imports = wasi.getImportObject();
const original = imports.wasi_snapshot_preview1.proc_exit;
imports.wasi_snapshot_preview1.proc_exit = code => { exited = code; return original(code); };
const instance = await WebAssembly.instantiate(module, imports);
const code = wasi.start(instance);
process.stderr.write(JSON.stringify({exitCode:code ?? 0, procExit:exited, memoryBytes:instance.exports.memory.buffer.byteLength}));
process.exitCode = code ?? 0;
