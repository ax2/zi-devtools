# Local patch to regex-automata 0.4.18

Original project: https://github.com/rust-lang/regex/tree/master/regex-automata
License: MIT OR Apache-2.0; both original license files are retained.
Registry archive checksum and original per-file SHA-256 are in upstream.json.
The shared Cargo registry is never modified.

The first source edit is in src/nfa/thompson/builder.rs, check_size_limit.
On wasm32/32-bit targets the intermediate state's limit accounting uses the
original Windows64 size of 32 bytes instead of WASI32's 16 bytes. Additional
Sparse transitions and Union alternatives retain their original accounting.
Actual allocations, regex syntax, configured limits and non-WASI behavior
are unchanged. This is not a reduction of the configured limit and does not
claim complete cross-platform regex equivalence.

The second source edit is in src/nfa/thompson/compiler.rs, c_exactly. After
actually entering Thompson compilation, repeated literal bytes necessarily
create at least one Range state each (see c_literal/c_range). On WASI32 only,
if these states alone exceed the same Windows64 program limit, return the
original exceeded_size_limit error before expanding the repetition. This uses
parsed HIR literal bytes, including UTF-8, and saturating arithmetic. Zero
repetitions, other HIR nodes, unset limits and non-WASI targets are unchanged.
It is a proven lower bound, not a guessed compiled size or syntax restriction.

Within the same guarded compiler path, nonempty literal bytes use a flat
double-ended iterator across repeats instead of nested c/c_literal/c_concat.
Range creation order and per-state size checks remain unchanged; reverse
iteration reverses each identical repeated literal. Empty literal HIR keeps
the original fallback; zero repetitions still produce the original empty
concatenation. Matching, Unicode offsets and capture reports require the
actual native/WASI regression matrix and unchanged fuel budget verification.

When upgrading the dependency, review the intermediate State layout, Transition
and StateID layouts, compiler paths and all other size gates again. Re-run the
native/WASI fixed vectors, separate program-limit diagnostic and unchanged
resource budgets. A green functional diagnostic is not Host acceptance.
