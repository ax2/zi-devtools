"""Verify the original-size conversion guest against the native shared codec and decoded fixtures."""

import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import struct
import subprocess
import zlib
from tempfile import TemporaryDirectory

from PIL import Image

PLUGIN = "com.zicode.devtools.image-convert"
CAPABILITY = "devtools.image.convert.original-size"
PROFILE = "resource.wasi.image.resize.v1"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
FORMATS = {"png":"image/png", "jpeg":"image/jpeg", "webp":"image/webp"}


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()


def digest(value):
    return hashlib.sha256(value).hexdigest()


def make_image(width,height,format="png",rgba=True,solid=False):
    mode = "RGBA" if rgba else "RGB"
    if solid:
        image = Image.new(mode,(width,height),(23,84,122,157) if rgba else (23,84,122))
    else:
        row = bytes(v for x in range(width) for v in ((x%256,(x*7)%256,118,120+x%136) if rgba
                                                       else (x%256,(x*7)%256,118)))
        image = Image.frombytes(mode,(width,height),row*height)
    b=BytesIO()
    image.save(b,format.upper(),**({"lossless":True} if format=="webp" else {}))
    return b.getvalue()


def animated(fmt):
    a=Image.new("RGBA",(4,4),(90,80,70,255))
    b=Image.new("RGBA",(4,4),(90,12,15,255))
    buf=BytesIO()
    a.save(buf,fmt.upper(),save_all=True,append_images=[b],duration=200,loop=0)
    return buf.getvalue()


def prior_worker_probe():
    # Exact synthetic 5000x32 bytes used by the previous resize Worker receipt.
    width,height=5000,32
    def chunk(name,data):
        return struct.pack(">I",len(data))+name+data+struct.pack(">I",zlib.crc32(name+data))
    rows=[]
    for y in range(height):
        row=bytes(channel for x in range(width) for channel in (x%256,y%256,(x+y)%256,255 if x%3 else 82))
        rows.append(b"\x00"+row)
    image=(b"\x89PNG\r\n\x1a\n"
           +chunk(b"IHDR",struct.pack(">IIBBBBB",width,height,8,6,0,0,0))
           +chunk(b"IDAT",zlib.compress(b"".join(rows)))+chunk(b"IEND",b""))
    assert digest(image)=="40d62e561510cf3051b64b4b71d67918105c989a0978f1914f2edf2098454b63"
    return image


def fixtures():
    png=make_image(128,32)
    jpeg=make_image(96,32,"jpeg",rgba=False)
    webp=make_image(96,32,"webp")
    for fmt, src in (("png",png),("jpeg",jpeg),("webp",webp)):
        for out in FORMATS:
            yield f"{fmt}-to-{out}", src,FORMATS[fmt],dict(format=out),None
    wide=make_image(5000,32)
    for fmt in FORMATS:
        yield f"same-extent-5000-to-{fmt}",wide,"image/png",dict(format=fmt),None
    yield "prior-resize-rejection-now-converts",prior_worker_probe(),"image/png",dict(format="webp"),None
    end_w=make_image(12000,4)
    end_h=make_image(4,12000)
    yield "native-wide-edge-12000",end_w,"image/png",dict(format="webp"),None
    yield "native-tall-edge-12000",end_h,"image/png",dict(format="png"),None
    yield "native-wide-edge-12001",make_image(12001,4),"image/png",dict(format="png"),"IMAGE_DIMENSION_LIMIT"
    yield "native-tall-edge-12001",make_image(4,12001),"image/png",dict(format="png"),"IMAGE_DIMENSION_LIMIT"
    solid=make_image(4000,4000,solid=True)
    for fmt in FORMATS:
        yield f"16m-pixel-solid-to-{fmt}",solid,"image/png",dict(format=fmt,quality=85),None
    yield "16m-pixel-limit-exceeded",make_image(4001,4000,solid=True),"image/png",dict(format="png"),"IMAGE_DIMENSION_LIMIT"
    yield "jpeg-minimum-quality",png,"image/png",dict(format="jpeg",quality=35),None
    yield "jpeg-maximum-quality",png,"image/png",dict(format="jpeg",quality=95),None
    yield "jpeg-ui-numeric-string",png,"image/png",dict(format="jpeg",quality="85"),None
    yield "jpeg-quality-out-of-range",png,"image/png",dict(format="jpeg",quality=96),"INVALID_PARAMETERS"
    yield "bad-quality",png,"image/png",dict(format="jpeg",quality="x"),"INVALID_PARAMETERS"
    yield "does-not-accept-resize-parameter",png,"image/png",dict(format="png",maxWidth=32),"IMAGE_OPERATION_FAILED"
    yield "unsupported-output",png,"image/png",dict(format="gif"),"INVALID_PARAMETERS"
    yield "wrong-input-media-type",jpeg,"image/png",dict(format="png"),"IMAGE_OPERATION_FAILED"
    yield "invalid-image",b"synthetic invalid image","image/png",dict(format="png"),"INVALID_IMAGE"
    for fmt in ("png","webp"):
        yield f"reject-animated-{fmt}",animated(fmt),FORMATS[fmt],dict(format="png"),"IMAGE_ANIMATED"


def worker_run(worker,module,src,mime,options):
    r=dict(workerKind="wasi",workerPath="",pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",
           input=options,executionProfile=PROFILE,resourceOutputProfile=OUTPUT)
    header=compact(dict(profile=INPUT,request=r,resources=[dict(resourceId="12121212-1212-4212-8212-121212121212",
           byteLength=len(src),contentVersion=digest(src),mimeType=mime)],module=dict(byteLength=len(module),sha256=digest(module))))
    proc=subprocess.run([str(worker),"--zicode-plugin-worker"],
        input=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+src,
        capture_output=True,timeout=20,creationflags=subprocess.CREATE_NO_WINDOW)
    if proc.returncode!=0 or proc.stdout[:8]!=b"ZIWO\0\0\0\x01":
        raise RuntimeError(proc.stdout[:240].decode(errors="replace"))
    n=12+struct.unpack_from("<I",proc.stdout,8)[0]
    result=json.loads(proc.stdout[12:n])
    assert result["profile"]==OUTPUT
    return result,proc.stdout[n:]


def native_run(native,source,output,quality,format,width):
    proc=subprocess.run([str(native),str(source),str(output),str(width),format,str(quality)],
        capture_output=True,timeout=80,creationflags=subprocess.CREATE_NO_WINDOW)
    assert proc.returncode==0,proc.stderr
    return json.loads(proc.stdout),output.read_bytes()


def check_output(case,source,body,report,options,qa,native):
    src=Image.open(BytesIO(source))
    out=Image.open(BytesIO(body))
    assert src.size==out.size==(report["width"],report["height"])
    assert [report["sourceWidth"],report["sourceHeight"]]==list(src.size)
    fmt=options["format"]
    assert out.format.lower()==fmt and report["mimeType"]==FORMATS[fmt]
    if fmt in ("png","webp"):
        # Compare actual decoder pixels when source encoder/decoder is exact.
        if src.format!="JPEG":
            assert source_image_difference(src,out)==0,case
    else:
        assert out.mode=="RGB"
    src_path=qa/(case+".input")
    native_path=qa/(case+".output")
    src_path.write_bytes(source)
    native_report,native_bytes=native_run(native,src_path,native_path,int(options.get("quality",85)),fmt,src.width)
    assert native_bytes==body,(case,"native codec bytes differ")
    assert all(report[k]==v for k,v in native_report.items())
    assert report["byteLength"]==len(body)


def source_image_difference(before,after):
    from PIL import ImageChops
    delta=ImageChops.difference(before.convert("RGBA"),after.convert("RGBA"))
    return sum(delta.getextrema()[i][1] for i in range(4))


def main():
    args=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        args.add_argument("--"+name,type=Path,required=True)
    args=args.parse_args()
    module=args.module.read_bytes()
    rows,failures=[],[]
    with TemporaryDirectory() as folder:
        for case,src,mime,options,error in fixtures():
            try:
                envelope,body=worker_run(args.worker,module,src,mime,options)
                report=envelope["result"]
                if error:
                    assert not report["ok"] and report["error"]["code"]==error,report
                    assert not envelope["resources"] and not body,case
                else:
                    assert report["ok"],report
                    assert envelope["resources"]==[dict(index=0,mimeType=FORMATS[options["format"]],
                                                         byteLength=len(body),contentVersion=digest(body))]
                    check_output(case,src,body,report,options,Path(folder),args.native)
            except Exception as err:
                failures.append(dict(case=case,error=repr(err)[:500]))
                print("FAIL",case,repr(err)[:240],flush=True)
            else:
                rows.append(dict(case=case,inputSha256=digest(src),mime=mime,input=options,error=error,
                                 outputBytes=len(body),outputSha256=digest(body) if body else None,result=report))
                print("PASS",case,flush=True)
    receipt=dict(passed=len(rows),wasmSha256=digest(module),workerSha256=digest(args.worker.read_bytes()),
                 nativeSha256=digest(args.native.read_bytes()),workerProfile=INPUT,
                 outputProfile=OUTPUT,executionProfile=PROFILE,cases=rows,failures=failures)
    args.evidence.write_text(json.dumps(receipt,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__=="__main__":
    main()
