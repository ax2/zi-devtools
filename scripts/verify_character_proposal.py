"""Validate character input shapes only; never authorize resources or claim runtime support."""
import argparse
import copy
import hashlib
import json
import re
from pathlib import Path
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
FOLDER = ROOT / "proposals/studio-character/v0.1.0"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path)
args = parser.parse_args()
load = lambda path: json.loads(path.read_text(encoding="utf-8"))
proposal = load(FOLDER / "proposal.json")
schema = load(FOLDER / "operation.schema.json")
assert proposal["status"] == "provider_scope_proposal_not_Host_supported"
assert proposal["runtimeBudget"] is None and proposal["frozenContractModified"] is False
versions = {tool["id"]: tool["tool_version"] for tool in load(ROOT / "docs/tools.json")["tools"]}
operations = {op["id"]: op for op in proposal["operations"]}
assert set(operations) == {"ascii.lookup", "symbols.search", "art.banner", "art.image"}
assert len(schema["oneOf"]) == len(operations)
for branch in schema["oneOf"]:
    operation = operations[branch["properties"]["operationId"]["const"]]
    assert branch["properties"]["input"] == operation["inputSchema"]
    assert operation["sourceVersion"] == versions[operation["sourceToolId"]]
assert proposal["resourceLimits"] == dict(encodedBytes=10485760, pixels=20000000,
    formats=["png", "jpeg", "webp"], widthMin=8, widthMax=120, heightMax=120)
core_source = (ROOT / "crates/zi-character-core/src/lib.rs").read_text(encoding="utf-8")
categories_block = core_source.split("pub const CATEGORIES:", 1)[1].split("];", 1)[0]
assert operations["symbols.search"]["inputSchema"]["properties"]["category"]["enum"] == re.findall(r'"([^"]+)"', categories_block)
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)

def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result

def reject_constant(_):
    raise ValueError("non-JSON constant")

def valid(raw):
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique, parse_constant=reject_constant)
        json.dumps(value, ensure_ascii=False).encode("utf-8")
        validator.validate(value)
        return True
    except (UnicodeError, ValueError, ValidationError, RecursionError):
        return False

rows = []
def check(name, value, expected):
    raw = value if isinstance(value, bytes) else json.dumps(value, ensure_ascii=False).encode("utf-8")
    actual = valid(raw)
    assert actual == expected, name
    rows.append(dict(id=name, acceptedShape=actual, passed=True))

seeds = {"ascii.lookup": {"query": "7"}, "symbols.search": {"query": "加油", "category": "颜文字"},
         "art.banner": {"text": "ZI DEVTOOLS", "ink": "#"}, "art.image": {"resourceHandle": "synthetic-only-not-a-grant", "width": 64}}
for operation, fields in seeds.items():
    seed = dict(operationId=operation, input=fields)
    check(operation + "-shape", seed, True)
    for field in fields:
        missing = copy.deepcopy(seed)
        del missing["input"][field]
        check(operation + "-missing-" + field, missing, False)
        for invalid in [None, [], {}, True]:
            wrong = copy.deepcopy(seed)
            wrong["input"][field] = invalid
            check(operation + "-type-" + field + "-" + str(invalid), wrong, False)
    extra = copy.deepcopy(seed)
    extra["input"]["hidden"] = "forbidden"
    check(operation + "-extra-input", extra, False)
    check(operation + "-extra-outer", dict(seed, hidden="forbidden"), False)
    check(operation + "-unknown", dict(seed, operationId=operation + ".unknown"), False)
for length in [0, 40, 41]:
    check("banner-codepoints-" + str(length), dict(operationId="art.banner", input=dict(text="中" * length, ink="#")), length <= 40)
for ink, expected in [("", False), (" ", False), ("中", False), ("##", False), ("#\n", False), ("!", True), ("~", True)]:
    check("banner-ink-" + repr(ink), dict(operationId="art.banner", input=dict(text="A", ink=ink)), expected)
for width in [0, 7, 8, 120, 121, 8.5]:
    check("image-width-" + str(width), dict(operationId="art.image", input=dict(resourceHandle="synthetic", width=width)), width in [8, 120])
check("empty-resource-reference", dict(operationId="art.image", input=dict(resourceHandle="", width=8)), False)
check("invalid-category", dict(operationId="symbols.search", input=dict(query="", category="hidden")), False)
check("duplicate-key", b'{"operationId":"ascii.lookup","input":{"query":"A","query":"B"}}', False)
check("non-json-number", b'{"operationId":"ascii.lookup","input":{"query":NaN}}', False)
check("invalid-utf8", b"\xff", False)
check("escaped-unpaired-surrogate", b'{"operationId":"ascii.lookup","input":{"query":"\\ud800"}}', False)
proof = dict(status=proposal["status"], cases=rows,
    sourceFiles={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in FOLDER.glob("*.json")},
    limitations=["Input structure only; no Host request envelope, authority, transport, algorithms, runtime budget, WASI, View or Pi acceptance"])
if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(f"PASS {len(rows)} character proposal input-shape vectors; no resource authorization or runtime claim")
