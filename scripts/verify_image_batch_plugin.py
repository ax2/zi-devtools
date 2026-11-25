"""Prove ordered, complete image batches through native and actual Windows Worker adapters."""

import argparse
import hashlib
import json
from pathlib import Path
import random
import struct
import subprocess
from tempfile import TemporaryDirectory
import zlib

PLUGIN = "com.zicode.devtools.image-batch"
CAPABILITY = "devtools.image.batch.resize"
TITLE = "图片批处理（小型 PNG）"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
MIME = "image/png"
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def digest(value):
    return hashlib.sha256(value).hexdigest()


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def chunk(name, data):
    return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name+data))


def png(width, height, seed=1, noisy=True, animated=False):
    if noisy:
        rng = random.Random(seed)
        pixels = [b"\0" + rng.randbytes(4*width) for _ in range(height)]
    else:
        pixels = [b"\0" + bytes((seed, 120, 20, 175))*width for _ in range(height)]
    result = PNG_MAGIC + chunk(b"IHDR", struct.pack(">IIBBBBB", width,height,8,6,0,0,0))
    if animated:
        result += chunk(b"acTL", struct.pack(">II", 1,0))
        result += chunk(b"fcTL", struct.pack(">IIIIIHHBB", 0,width,height,0,0,10,100,0,0))
    return result + chunk(b"IDAT", zlib.compress(b"".join(pixels))) + chunk(b"IEND", b"")


def dimensions(raw, mime):
    assert mime == MIME and raw.startswith(PNG_MAGIC) and raw[12:16] == b"IHDR"
    return struct.unpack_from(">II", raw, 16)


def png_pixels(raw):
    """Independent pixel reread for constructed and exported filter-zero RGBA PNGs."""
    width,height=dimensions(raw,MIME)
    assert raw[24:29] == b"\x08\x06\0\0\0"
    index=8
    pieces=[]
    while index<len(raw):
        size=struct.unpack_from(">I",raw,index)[0]
        if raw[index+4:index+8]==b"IDAT":
            pieces.append(raw[index+8:index+8+size])
        index+=size+12
    pixels=zlib.decompress(b"".join(pieces))
    assert len(pixels)==height*(1+4*width) and all(pixels[row*(1+4*width)]==0 for row in range(height))
    return pixels


def fixtures():
    # Ordered fixtures use different source aspects so reversal is detectable.
    typed = [(MIME,png(64,53)),(MIME,png(51,64,2))]
    first,second = typed
    props = dict(maxWidth=32)
    yield "two-inputs-and-two-output-dimensions",typed,props,[(32,27),(32,40)]
    yield "same-batch-reversed",typed[::-1],props,[(32,40),(32,27)]
    yield "one-input",[first],props,[(32,27)]
    noisy=[(MIME,png(64,64,2)),(MIME,png(64,64,3))]
    yield "exact-edge-unchanged",noisy,{},[(64,64),(64,64)]
    yield "exact-edge-at-maximum-resized-width",noisy,dict(maxWidth=48),[(48,48),(48,48)]
    yield "exact-edge-at-minimum-width",noisy,dict(maxWidth=1),[(1,1),(1,1)]
    yield "near-maximum-source-with-almost-no-resize",[(MIME,png(49,64)),(MIME,png(49,64,2))],dict(maxWidth=48),[(48,63),(48,63)]
    yield "opposite-aspect-ratios",[(MIME,png(64,2)),(MIME,png(2,64,2))],props,[(32,1),(2,64)]
    yield "no-upscaling",[(MIME,png(13,13))],props,[(13,13)]
    small = png(64,64,2,False)
    padded=small[:-12]+chunk(b"tEXt",b"k\0"+b"x"*(32*1024-len(small)-14))+small[-12:]
    assert len(padded)==32*1024
    yield "exact-byte-boundary",[(MIME,padded),(MIME,padded)],{},[(64,64),(64,64)]
    yield "edge-limit",[first,(MIME,png(65,5))],props,"INPUT_LIMIT"
    yield "height-limit",[first,(MIME,png(5,65))],props,"INPUT_LIMIT"
    yield "byte-limit",[(MIME,padded+b"x")],props,"INPUT_LIMIT"
    yield "too-many-inputs",[first,second,first],props,"INVALID_RESOURCE"
    yield "malformed-second-file",[first,(MIME,b"not an image")],props,"INVALID_IMAGE"
    yield "unsupported-second-mime",[first,("image/jpeg",second[1])],props,"INVALID_RESOURCE"
    yield "invalid-second-stream",[first,(MIME,second[1][:-35])],props,"INVALID_IMAGE"
    yield "animated-second-file",[first,(MIME,png(16,16,animated=True))],props,"ANIMATED_IMAGE"
    gray16=PNG_MAGIC+chunk(b"IHDR",struct.pack(">IIBBBBB",2,1,16,0,0,0,0))+chunk(b"IDAT",zlib.compress(b"\x00\xa2\xf1\0\1"))+chunk(b"IEND",b"")
    yield "sixteen-bit-second-image",[first,(MIME,gray16)],props,"COLOR_DEPTH"
    yield "invalid-width",typed,dict(maxWidth=0),"INVALID_REQUEST"
    yield "width-between-resize-and-no-resize",typed,dict(maxWidth=49),"INVALID_REQUEST"
    yield "unsupported-format",typed,dict(maxWidth=32,format="gif"),"INVALID_REQUEST"


def request(sources, options):
    resources = [dict(resourceId=f"7ea4b8db-6688-461a-bd02-f6a8ea44e5a{index}", mimeType=mime,
                      byteLength=len(raw),contentVersion=digest(raw)) for index,(mime,raw) in enumerate(sources)]
    return dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input=options,
                resourceOutputProfile=OUTPUT), resources


def native_result(native, sources, options, scratch):
    opts,rs = request(sources,options)
    files = []
    output = scratch/"output"
    output.mkdir()
    for index,(_,blob) in enumerate(sources):
        item = scratch/f"source-{index}"
        item.write_bytes(blob)
        files.append(item)
    env = dict(**opts,commandId=None,resourceInputProfile=INPUT,resources=[dict(index=i,**r) for i,r in enumerate(rs)])
    result = subprocess.run([native,output,*files],input=compact(env),capture_output=True,check=True,timeout=12,
                            creationflags=subprocess.CREATE_NO_WINDOW)
    assert [file.read_bytes() for file in files] == [blob for _,blob in sources]
    return json.loads(result.stdout), [file.read_bytes() for file in sorted(output.iterdir())]


def worker_result(worker, module, sources, options):
    opts,resources = request(sources,options)
    header = compact(dict(profile=INPUT, request=dict(workerKind="wasi",workerPath="",**opts), resources=resources,
                          module=dict(byteLength=len(module),sha256=digest(module))))
    command = subprocess.run([worker,"--zicode-plugin-worker"],
        input=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+b"".join(raw for _,raw in sources),
        capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert command.returncode==0 and command.stdout.startswith(b"ZIWO\0\0\0\x01"),command.stdout[:350]
    end = 12 + struct.unpack_from("<I", command.stdout, 8)[0]
    wire = json.loads(command.stdout[12:end])
    outputs = []
    for index, item in enumerate(wire["resources"]):
        assert item["index"] == index
        blob = command.stdout[end:end+item["byteLength"]]
        assert item["contentVersion"] == digest(blob)
        end += len(blob)
        outputs.append((item["mimeType"],blob))
    assert end == len(command.stdout)
    return wire["result"], outputs


def verify(case, worker, native, module, scratch):
    name,sources,options,expected = case
    report,outputs = worker_result(worker,module,sources,options)
    native_report,native_outputs = native_result(native,sources,options,scratch)
    assert report == native_report, (report,native_report)
    assert [data for _,data in outputs] == native_outputs
    if isinstance(expected,str):
        assert not report["ok"] and report["error"]["code"] == expected, report
        assert outputs == []
        error = expected
    else:
        assert report["ok"] and len(outputs) == len(expected), report
        assert [dimensions(blob,mime) for mime,blob in outputs] == expected
        for index, (item,(mime,blob)) in enumerate(zip(report["data"]["items"],outputs,strict=True)):
            assert item["inputIndex"] == index == item["outputIndex"] and mime == report["data"]["mimeType"]
            assert (item["sourceWidth"],item["sourceHeight"]) == dimensions(sources[index][1],sources[index][0])
            assert (item["width"],item["height"]) == expected[index] and item["byteLength"] == len(blob)
            if (item["width"],item["height"]) == (item["sourceWidth"],item["sourceHeight"]):
                assert png_pixels(blob)==png_pixels(sources[index][1])
        error = None
    return dict(case=name,sources=[dict(mime=mime,sha256=digest(raw),bytes=len(raw)) for mime,raw in sources],
        options=options,error=error,outputs=[dict(mime=mime,sha256=digest(raw),bytes=len(raw)) for mime,raw in outputs])


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases,failures = [], []
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        scratch=Path(temporary)
        for case in fixtures():
            location=scratch/case[0]
            location.mkdir()
            try:
                cases.append(verify(case,args.worker,args.native,module,location))
            except (AssertionError,TypeError,ValueError,subprocess.SubprocessError) as error:
                failures.append(dict(case=case[0],failure=str(error)[:600]))
    evidence=dict(workerSha256=digest(args.worker.read_bytes()),wasmSha256=digest(module),wasmBytes=len(module),
                  inputProfile=INPUT,outputProfile=OUTPUT,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(evidence,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(cases),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
