"""Verify original pixel colors through the native adapter and the real Windows Worker."""

import argparse
import hashlib
import json
from pathlib import Path
import random
import struct
import subprocess
from tempfile import TemporaryDirectory
import zlib


PLUGIN="com.zicode.devtools.screenshot-pixel"
CAPABILITY="devtools.screenshot.png.pixel"
TITLE="PNG 截图取色"
PROFILE="resource.wasi.input.draft2"
PNG="image/png"
MAGIC=b"\x89PNG\r\n\x1a\n"


def compact(data):
    return json.dumps(data,ensure_ascii=False,separators=(",",":")).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def chunk(tag,data):
    return struct.pack(">I",len(data))+tag+data+struct.pack(">I",zlib.crc32(tag+data))


def png(width,height,rows,png_type=6,depth=8,chunks=(),animated=False):
    header=MAGIC+chunk(b"IHDR",struct.pack(">IIBBBBB",width,height,depth,png_type,0,0,0))
    if animated:
        header+=chunk(b"acTL",struct.pack(">II",1,0))
        header+=chunk(b"fcTL",struct.pack(">IIIIIHHBB",0,width,height,0,0,10,100,0,0))
    return (header + b"".join(chunk(tag,value) for tag,value in chunks)
            + chunk(b"IDAT",zlib.compress(b"".join(b"\0"+row for row in rows)))+chunk(b"IEND",b""))


def patterned_png(width,height,seed):
    random_data=random.Random(seed)
    rows=[]
    for y in range(height):
        raw=bytearray(random_data.randbytes(4*width))
        raw[3::4]=bytes((x+y)%9*29 for x in range(width))
        rows.append(bytes(raw))
    return png(width,height,rows),tuple(rows[-1][-4:])


def paeth_png(width,height,seed):
    random_data=random.Random(seed)
    rows=[random_data.randbytes(4*width) for _ in range(height)]
    output=[]
    previous=bytes(4*width)
    for raw in rows:
        encoded=bytearray()
        for i,value in enumerate(raw):
            a=raw[i-4] if i>=4 else 0
            b=previous[i]
            c=previous[i-4] if i>=4 else 0
            p=a+b-c
            pa,pb,pc=abs(p-a),abs(p-b),abs(p-c)
            prediction=a if pa<=pb and pa<=pc else b if pb<=pc else c
            encoded.append((value-prediction)&255)
        output.append(b"\x04"+encoded)
        previous=raw
    data=MAGIC+chunk(b"IHDR",struct.pack(">IIBBBBB",width,height,8,6,0,0,0))+chunk(b"IDAT",zlib.compress(b"".join(output)))+chunk(b"IEND",b"")
    return data,tuple(rows[-1][-4:])


def adam7_png(width,height,seed):
    random_data=random.Random(seed)
    points=[random_data.randbytes(4) for _ in range(width*height)]
    passes=[(0,0,8,8),(4,0,8,8),(0,4,4,8),(2,0,4,4),(0,2,2,4),(1,0,2,2),(0,1,1,2)]
    rows=[b"\0"+b"".join(points[y*width+x] for x in range(start_x,width,dx))
          for start_x,start_y,dx,dy in passes if start_x<width for y in range(start_y,height,dy)]
    data=MAGIC+chunk(b"IHDR",struct.pack(">IIBBBBB",width,height,8,6,0,0,1))+chunk(b"IDAT",zlib.compress(b"".join(rows)))+chunk(b"IEND",b"")
    return data,tuple(points[-1])


def fixtures():
    pixels=bytes((200,19,77,0,2,5,242,128,17,170,83,255,5,7,4,15))
    small=png(2,2,(pixels[:8],pixels[8:]))
    yield "zero-alpha-hidden-rgb",small,PNG,dict(x=0,y=0),(2,2,(200,19,77,0))
    yield "half-alpha-no-premultiply",small,PNG,dict(x=1,y=0),(2,2,(2,5,242,128))
    yield "bottom-left-original-coordinate",small,PNG,dict(x=0,y=1),(2,2,(17,170,83,255))
    yield "bottom-right-original-coordinate",small,PNG,dict(x=1,y=1),(2,2,(5,7,4,15))
    r=png(2,1,(bytes((21,90,200,250,17,4)),),png_type=2)
    yield "rgb-with-opaque-alpha",r,PNG,dict(x=1,y=0),(2,1,(250,17,4,255))
    g=png(2,1,(bytes((75,31)),),png_type=0)
    yield "greyscale-to-equal-channels",g,PNG,dict(x=0,y=0),(2,1,(75,75,75,255))
    pal=png(2,1,(b"\x01\x00",),png_type=3,chunks=((b"PLTE",bytes((8,32,16,220,34,120))),(b"tRNS",bytes((255,0)))))
    yield "palette-transparent-color",pal,PNG,dict(x=0,y=0),(2,1,(220,34,120,0))
    limit,pixel=patterned_png(128,128,518)
    assert len(limit)<=256*1024
    yield "max-pixels-128-square",limit,PNG,dict(x=127,y=127),(128,128,pixel)
    exact=limit[:-12]+chunk(b"tEXt",b"k\x00"+b"a"*(256*1024-len(limit)-14))+limit[-12:]
    assert len(exact)==256*1024
    yield "exact-file-and-pixel-limits",exact,PNG,dict(x=127,y=127),(128,128,pixel)
    wide,pixel=patterned_png(512,32,115)
    assert len(wide)<=256*1024
    yield "max-edge-and-total-pixels",wide,PNG,dict(x=511,y=31),(512,32,pixel)
    filtered,pixel=paeth_png(128,128,207)
    yield "paeth-at-max-pixels",filtered,PNG,dict(x=127,y=127),(128,128,pixel)
    interlaced,pixel=adam7_png(128,128,121)
    yield "adam7-at-max-pixels",interlaced,PNG,dict(x=127,y=127),(128,128,pixel)
    edge=png(513,1,(b"\0"*(513*4),))
    large=png(128,129,(b"\0"*(128*4) for _ in range(129)))
    deep=png(1,1,(bytes((8,0,1,0,3,0,4,0)),),depth=16)
    for name,raw,mime,opts,code in [
        ("outside-right",small,PNG,dict(x=2,y=0),"OUT_OF_BOUNDS"),
        ("outside-bottom",small,PNG,dict(x=0,y=2),"OUT_OF_BOUNDS"),
        ("negative",small,PNG,dict(x=-1,y=0),"INVALID_REQUEST"),
        ("fractional",small,PNG,dict(x=0.25,y=0),"INVALID_REQUEST"),
        ("missing-y",small,PNG,dict(x=0),"INVALID_REQUEST"),
        ("extra-scaling",small,PNG,dict(x=0,y=0,scale=2),"INVALID_REQUEST"),
        ("incorrect-mime",small,"image/jpeg",dict(x=0,y=0),"INVALID_RESOURCE"),
        ("unknown-byte-content",b"not an image",PNG,dict(x=0,y=0),"INVALID_PNG"),
        ("truncated-pixel-stream",limit[:-47],PNG,dict(x=0,y=0),"INVALID_PNG"),
        ("file-limit-plus-one",exact+b"\0",PNG,dict(x=0,y=0),"INPUT_LIMIT"),
        ("edge-limit-plus-one",edge,PNG,dict(x=0,y=0),"DIMENSION_LIMIT"),
        ("pixel-limit-plus-one-rectangle",large,PNG,dict(x=0,y=0),"DIMENSION_LIMIT"),
        ("initial-fuel-square",patterned_png(256,256,518)[0],PNG,dict(x=255,y=255),"DIMENSION_LIMIT"),
        ("initial-fuel-rectangle",patterned_png(512,128,115)[0],PNG,dict(x=511,y=127),"DIMENSION_LIMIT"),
        ("sixteen-bit-no-truncation",deep,PNG,dict(x=0,y=0),"UNSUPPORTED_DEPTH"),
        ("animated-no-flattening",png(2,2,(pixels[:8],pixels[8:]),animated=True),PNG,dict(x=0,y=0),"ANIMATED_PNG"),
    ]:
        yield name,raw,mime,opts,code


def request(case):
    _,raw,mime,options,_=case
    resource=dict(resourceId="8ca2268d-2019-4942-bec6-970925f0d8a9",mimeType=mime,byteLength=len(raw),contentVersion=digest(raw))
    info=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input=options)
    return info,resource


def native_result(native,case,scratch):
    info,resource=request(case)
    src=scratch/"fixture.png"
    src.write_bytes(case[1])
    payload=dict(**info,commandId=None,resourceInputProfile=PROFILE,resourceOutputProfile=None,
                 resources=[dict(index=0,**resource)])
    process=subprocess.run([native,src],input=compact(payload),capture_output=True,check=True,timeout=12,
                           creationflags=subprocess.CREATE_NO_WINDOW)
    assert src.read_bytes()==case[1]
    return json.loads(process.stdout)


def worker_result(worker,module,case):
    info,resource=request(case)
    header=compact(dict(profile=PROFILE,request=dict(workerKind="wasi",workerPath="",**info),resources=[resource],
                        module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+case[1]
    process=subprocess.run([worker,"--zicode-plugin-worker"],input=frame,capture_output=True,timeout=15,
                           creationflags=subprocess.CREATE_NO_WINDOW)
    payload=json.loads(process.stdout)
    assert process.returncode==0 and payload["ok"],payload
    return payload["result"]


def verify(case,worker,native,module,scratch):
    name,raw,mime,options,expected=case
    report=worker_result(worker,module,case)
    assert report==native_result(native,case,scratch),report
    if isinstance(expected,str):
        assert not report["ok"] and report["data"] is None and report["error"]["code"]==expected,report
        code=expected; result=None
    else:
        width,height,rgba=expected
        assert report["ok"] and report["error"] is None,report
        result=report["data"]
        assert (result["width"],result["height"],tuple(result["rgba"]))==(width,height,rgba),report
        assert (result["x"],result["y"])==(options["x"],options["y"])
        assert result["hexRgb"]=="#"+bytes(rgba[:3]).hex().upper() and result["hexRgba"]=="#"+bytes(rgba).hex().upper()
        code=None
    return dict(case=name,mime=mime,inputBytes=len(raw),sourceSha256=digest(raw),options=options,code=code,
                rgba=result["rgba"] if result else None)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases,failures=[],[]
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        for case in fixtures():
            try:
                cases.append(verify(case,args.worker,args.native,module,Path(temporary)))
            except (AssertionError,subprocess.SubprocessError,ValueError,TypeError) as error:
                failures.append(dict(case=case[0],message=str(error)[:500]))
    proof=dict(wasmSha256=digest(module),wasmBytes=len(module),workerSha256=digest(args.worker.read_bytes()),
               inputProfile=PROFILE,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(cases),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
