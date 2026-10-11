"""Verify pinned upstream source and the two declared WASI-only patches."""
import hashlib
import json
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parents[1] / 'vendor/regex-automata'
workspace = root.parent.parent
manifest = tomllib.loads((workspace/'Cargo.toml').read_text(encoding='utf-8'))
assert (workspace/manifest['patch']['crates-io']['regex-automata']['path']).resolve() == root.resolve()
lock = tomllib.loads((workspace/'Cargo.lock').read_text(encoding='utf-8'))
engines = [p for p in lock['package'] if p['name'] == 'regex-automata']
assert len(engines) == 1 and engines[0]['version'] == '0.4.18' and 'source' not in engines[0]
provenance = json.loads((root/'upstream.json').read_text(encoding='utf-8-sig'))
assert provenance['version'] == '0.4.18'
assert provenance['registryChecksum'] == 'ad8553b9b26413251cbf30e620595c7a41b3887f03da04579c0e6b0d6a06b4b2'
patch = b'''            // ZiCode: keep the Windows64 program limit on WASI32. Only the
            // accounting changes; Sparse/Union heap bytes still use the
            // original accounting and the actual allocation is unchanged.
            #[cfg(all(target_arch = "wasm32", target_pointer_width = "32"))]
            let memory_usage = self.states.len() * 32 + self.memory_states;
            #[cfg(not(all(target_arch = "wasm32", target_pointer_width = "32")))]
            let memory_usage = self.memory_usage();
            if memory_usage > limit {'''
seen = set()
compiler_patch = b'''        // ZiCode: after entering Thompson compilation, each literal byte
        // necessarily adds one Range state. Reject an already impossible
        // minimum using the same Windows64 limit, before expanding repeats.
        // Other HIR kinds keep their original compiler path.
        #[cfg(all(target_arch = "wasm32", target_pointer_width = "32"))]
        if let (hir::HirKind::Literal(hir::Literal(bytes)), Some(limit)) =
            (expr.kind(), self.config.get_nfa_size_limit())
        {
            let minimum = (bytes.len() as u64)
                .saturating_mul(u64::from(n))
                .saturating_mul(32);
            if minimum > limit as u64 {
                return Err(BuildError::exceeded_size_limit(limit));
            }
            // Flatten repeated literal bytes, preserving the same Range
            // allocation order and per-state limit checks. Reversing this
            // double-ended iterator is equivalent to reversing each copy.
            if !bytes.is_empty() {
                return self.c_concat(
                    (0..n)
                        .flat_map(|_| bytes.iter().copied())
                        .map(|byte| self.c_range(byte, byte)),
                );
            }
        }
'''
for entry in provenance['files']:
    relative = entry['path']
    assert relative not in seen, relative
    seen.add(relative)
    path = root/relative
    assert path.resolve().is_relative_to(root.resolve()), relative
    assert not path.is_symlink(), relative
    data = path.read_bytes()
    if relative == 'src/nfa/thompson/builder.rs':
        # Git may check out CRLF, while upstream bytes use LF. Normalize only
        # this edited file; untouched upstream files must retain exact bytes.
        data = data.replace(b'\r\n', b'\n')
        assert data.count(patch) == 1, 'Unexpected builder patch'
        data = data.replace(patch, b'            if self.memory_usage() > limit {')
    if relative == 'src/nfa/thompson/compiler.rs':
        data = data.replace(b'\r\n', b'\n')
        assert data.count(compiler_patch) == 1, 'Unexpected compiler patch'
        data = data.replace(compiler_patch, b'')
    assert hashlib.sha256(data).hexdigest() == entry['sha256'], relative
actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
assert actual == seen | {'upstream.json', 'ZICODE-PATCH.md'}, actual ^ seen
print(f'PASS {len(seen)} upstream files; only the two declared WASI patches differ')
