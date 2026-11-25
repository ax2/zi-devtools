"""Exercise prompt syntax and rendered bounds through the native and Windows Worker adapters."""

import argparse
import json
from pathlib import Path

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.prompt-expansion"
INSPECT = "devtools.prompt.variables"
EXPAND = "devtools.prompt.expand"
CONTRACT = "1.0.0-rc.1"
MAX_TEMPLATE = 16 * 1024
MAX_EXPANSION = 128 * 1024
MAX_PREVIEW = 16 * 1024
MAX_RESULT = 48 * 1024


def request(capability, args):
    return dict(pluginId=PLUGIN, sceneId="programming", capabilityId=capability,
                commandId=None, input=args)


def inspect(template, *names):
    text = f"变量 {len(names)} 项\n" + "\n".join(names) if names else "此模板没有变量。"
    return dict(variables=list(names), templateByteLength=len(template.encode()), text=text)


def expansion(output, *names):
    raw = output.encode()
    preview = raw[:MAX_PREVIEW].decode(errors="ignore")
    # Python's JSON encoder independently counts the actual UTF-8 wire size.
    if len(compact(preview)) - 2 > MAX_RESULT - 2048:
        low, high = 0, len(preview)
        while low < high:
            middle = (low + high + 1) // 2
            if len(compact(preview[:middle])) - 2 <= MAX_RESULT - 2048:
                low = middle
            else:
                high = middle - 1
        preview = preview[:low]
    kept = len(preview.encode())
    truncated = kept < len(raw)
    if truncated:
        preview += f"\n\n[预览显示前 {kept}/{len(raw)} 字节，已省略剩余内容]"
    return dict(variables=list(names), expandedByteLength=len(raw), previewByteLength=kept,
                truncated=truncated, text=preview)


def fixtures():
    text = "中文 {{z}}\r\n{{_a}}🙂{{z}}\t{{{{literal}}}}"
    yield "original-variable-order-and-uniqueness", INSPECT, dict(template=text), inspect(text, "_a", "z")
    yield "escapes-unicode-and-original-layout", EXPAND, dict(template=text, values={"z":" X ", "_a":"代入"}), expansion(
        "中文  X \r\n代入🙂 X \t{{literal}}", "_a", "z")
    literal = " \r\n保留 {单括号} 🌱\t "
    yield "no-variables", EXPAND, dict(template=literal), expansion(literal)
    yield "literal-braces", EXPAND, dict(template="{{{{key}}}}"), expansion("{{key}}")
    yield "unchanged-original-escape-adjacency", EXPAND, dict(template="{{{{{{key}}}}}}", values={"key":"A"}), expansion("{{A}}","key")
    yield "one-pass-values-are-not-reparsed", EXPAND, dict(template="{{a}} / {{b}}",
        values={"a":"{{b}}","b":"{{{{not-a-second-pass}}}}"}), expansion("{{b}} / {{{{not-a-second-pass}}}}","a","b")
    yield "unused-values-are-ignored", EXPAND, dict(template="unchanged", values={"later":"ignored"}), expansion("unchanged")
    yield "missing-variable", EXPAND, dict(template="before {{missing}}"), "EXPANSION_FAILED"
    yield "whitespace-is-not-a-value", EXPAND, dict(template="{{miss}}", values={"miss":"\u3000 \t"}), "EXPANSION_FAILED"
    yield "nonempty-whitespace-is-preserved", EXPAND, dict(template="{{name}}", values={"name":" 原样 \r\n"}), expansion(" 原样 \r\n","name")
    yield "empty-template", INSPECT, dict(template="\t \r\n"), "TEMPLATE_INVALID"
    yield "disallowed-control", INSPECT, dict(template="a\x00b"), "TEMPLATE_INVALID"
    yield "open-without-close", INSPECT, dict(template="开始 {{x"), "TEMPLATE_INVALID"
    yield "close-without-open", INSPECT, dict(template="末尾 }}"), "TEMPLATE_INVALID"
    yield "forbid-empty-name", INSPECT, dict(template="{{}}"), "TEMPLATE_INVALID"
    yield "forbid-space-in-name", INSPECT, dict(template="{{ name }}"), "TEMPLATE_INVALID"
    yield "forbid-unicode-name", INSPECT, dict(template="{{标题}}"), "TEMPLATE_INVALID"
    yield "forbid-name-leading-digit", INSPECT, dict(template="{{3times}}"), "TEMPLATE_INVALID"
    name = "A" + "x"*31
    yield "exact-name-limit", INSPECT, dict(template="{{"+name+"}}"), inspect("{{"+name+"}}",name)
    yield "name-one-over", INSPECT, dict(template="{{"+name+"x}}"), "TEMPLATE_INVALID"
    names = [f"v{i:02}" for i in range(12)]
    body = " ".join("{{"+n+"}}" for n in reversed(names))
    yield "twelve-distinct-names-sorted", INSPECT, dict(template=body), inspect(body,*names)
    yield "thirteenth-name-rejected", INSPECT, dict(template=body+"{{v12}}"), "TEMPLATE_INVALID"
    yield "thirteenth-binding-rejected", EXPAND, dict(template="{{v00}}",
          values={f"v{i:02}":"x" for i in range(13)}), "INVALID_INPUT"
    yield "numeric-binding-rejected", EXPAND, dict(template="{{n}}", values={"n":42}), "INVALID_INPUT"
    yield "bindings-object-required", EXPAND, dict(template="{{n}}", values=["n"]), "INVALID_INPUT"
    yield "inspect-has-no-binding-field", INSPECT, dict(template="{{n}}", values={}), "INVALID_INPUT"
    yield "reject-unknown-input", EXPAND, dict(template="{{n}}", execute=True), "INVALID_INPUT"
    wide = "星"*(MAX_TEMPLATE//3)+"a"*(MAX_TEMPLATE%3)
    assert len(wide.encode())==MAX_TEMPLATE
    yield "sixteen-kib-unicode", EXPAND, dict(template=wide), expansion(wide)
    yield "extra-template-byte", INSPECT, dict(template=wide+"Z"), "TEMPLATE_INVALID"
    opens = "{{{{"*(MAX_TEMPLATE//4)
    closes = "}}}}"*(MAX_TEMPLATE//4)
    yield "dense-open-literals-inspect", INSPECT, dict(template=opens), inspect(opens)
    yield "dense-open-literals-expand", EXPAND, dict(template=opens), expansion("{{"*(MAX_TEMPLATE//4))
    yield "dense-close-literals-inspect", INSPECT, dict(template=closes), inspect(closes)
    yield "dense-close-literals-expand", EXPAND, dict(template=closes), expansion("}}"*(MAX_TEMPLATE//4))
    packed = "{{x}}"*(MAX_TEMPLATE//5)+"!"*(MAX_TEMPLATE%5)
    yield "dense-one-variable-repeated", EXPAND, dict(template=packed, values={"x":"z"}), expansion(
        "z"*(MAX_TEMPLATE//5)+"!"*(MAX_TEMPLATE%5), "x")
    separated = "a{{x}}"*(MAX_TEMPLATE//6)+"!"*(MAX_TEMPLATE%6)
    yield "dense-one-byte-separators", EXPAND, dict(template=separated, values={"x":"值"}), expansion(
        "a值"*(MAX_TEMPLATE//6)+"!"*(MAX_TEMPLATE%6), "x")
    single_open = ("a{{{{"*(MAX_TEMPLATE//5))+"!"*(MAX_TEMPLATE%5)
    yield "one-byte-separated-open-literals", EXPAND, dict(template=single_open), expansion(
        "a{{"*(MAX_TEMPLATE//5)+"!"*(MAX_TEMPLATE%5))
    single_close = ("a}}}}"*(MAX_TEMPLATE//5))+"!"*(MAX_TEMPLATE%5)
    yield "one-byte-separated-close-literals", EXPAND, dict(template=single_close), expansion(
        "a}}"*(MAX_TEMPLATE//5)+"!"*(MAX_TEMPLATE%5))
    count = MAX_EXPANSION//128
    output = "样"*42+"xy"
    assert len(output.encode())==128
    body = "{{x}}"*count
    yield "max-native-expansion-prefix-only", EXPAND, dict(template=body, values={"x":output}), expansion(output*count,"x")
    yield "one-byte-expansion-over-max", EXPAND, dict(template=body+".", values={"x":output}), "EXPANSION_FAILED"
    yield "cut-at-whole-utf8-character", EXPAND, dict(template="x{{a}}{{a}}", values={"a":"🧪"*4096}), expansion(
        "x"+"🧪"*8192,"a")
    control = "\x01"*127+"z"
    yield "escape-heavy-result-stays-within-wire-limit", EXPAND, dict(template="{{c}}"*128, values={"c":control}), expansion(control*128,"c")
    value = "y"*(48*1024-len(compact(request(EXPAND, dict(template="{{x}}",values={"x":""})))))
    yield "exact-wire-request-limit", EXPAND, dict(template="{{x}}",values={"x":value}), expansion(value,"x")
    yield "wire-request-one-byte-over", EXPAND, dict(template="{{x}}",values={"x":value+"z"}), "INPUT_TOO_LARGE"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--" + name,required=True,type=Path)
    config = parser.parse_args()
    cases, failures = [], []
    for name, capability, inputs, expected in fixtures():
        envelope = request(capability,inputs)
        try:
            native = invoke([config.native], compact(envelope))
            worker = invoke([config.worker, "--zicode-plugin-worker"],
                            compact(dict(workerKind="wasi",workerPath=str(config.module),**envelope)))
            assert worker["ok"], worker
            result = worker["result"]
            assert result == native, (name, "native/WASI mismatch")
            assert result["contractVersion"] == CONTRACT
            assert len(compact(result)) <= MAX_RESULT
            if isinstance(expected,str):
                assert not result["ok"] and result["error"]["code"] == expected, result
                assert result["data"] is None and result["error"]["message"]
            else:
                assert result["ok"] and result["error"] is None and result["data"] == expected, (
                    name, {k:v for k,v in result["data"].items() if k!="text"},
                    {k:v for k,v in expected.items() if k!="text"})
            cases.append(dict(case=name,capabilityId=capability,inputSha256=digest(compact(inputs)),
                              ok=result["ok"],resultSha256=digest(compact(result)),resultBytes=len(compact(result))))
        except Exception as error:
            failures.append(dict(case=name,message=repr(error)[:900]))
            print(f"FAIL {name} {repr(error)[:350]}",flush=True)
        else:
            print(f"PASS {name}",flush=True)
    receipt = dict(passed=len(cases),wasmSha256=digest(config.module.read_bytes()),
                   workerSha256=digest(config.worker.read_bytes()),workerProfile="compute.wasi.v1",
                   cases=cases,failures=failures)
    config.evidence.write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
