# Local SQL tokenizer patch for WASI

The source is sqlformat 0.5.0. The upstream commit, release archive SHA-256,
and original per-file digests are in `upstream.json`. Both upstream licenses
are included.

The text core uses this copy when `target_os = "wasi"`. Native builds
resolve the original registry dependency at 0.5.0. The shared Cargo cache
and the existing frozen plugin packages are outside this patch.

The tokenizer delegates unambiguous prefix cases to the same winning
parser from the upstream precedence chain. A digit starts the number
parser. A dollar sign followed by a digit starts placeholder parsing.
A single ASCII delimiter, or an equals sign followed by a digit, has no
competing multi-character form. Short ASCII identifiers use the existing
word parser. The pinned vocabulary contains no single-letter keyword or
keyword with a digit in its second position. Quote prefixes continue
through the original parser chain.

SELECT, FROM, and WHERE use the original word boundaries. SELECT DISTINCT
and SELECT ALL continue through the original compound-keyword parser.
All successful keyword matches retain the original spelling and token
alias. Bounded prefix access respects UTF-8 boundaries and applies the
upstream word-character predicate. Multi-word matching finds the same
first ASCII-insensitive final word without allocating an uppercase copy
of the remaining SQL for each successful match.

Review token precedence, word boundaries, and the keyword table when
updating the pinned dependency. The Windows checks compare the frozen
67 requests byte for byte and exercise measured 8 KiB dense templates
across the five supported dialects in the existing 10 million fuel,
64 MiB memory and 48 KiB response budget. The result applies to the
measured templates. Add `--dense-sql` to `scripts/verify_text_wasi.py`
to feed the same nine templates to native/WASI byte parity and to export
raw vectors for the existing independent fuel checker.

Deep nesting and other heavily expanding queries
can still reach the normal worker limits.
