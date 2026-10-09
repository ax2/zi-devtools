"""Optional independent Windows verification cache; never a product dependency."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import urllib.request
import zipfile

parser=argparse.ArgumentParser()
parser.add_argument('--cache-dir',type=Path,required=True)
parser.add_argument('--provenance',type=Path,required=True)
args=parser.parse_args()
cache=args.cache_dir.absolute()
assert sys.platform=='win32' and 'target' in cache.parts, 'Use a local verification target cache on Windows'
assert not any(p.is_symlink() or p.is_junction() for p in [cache,*cache.parents])
subprocess.run([sys.executable,'-m','pip','install','--target',str(cache),'wasmtime==36.0.0'],check=True)
url='https://github.com/bytecodealliance/wasmtime/releases/download/v36.0.2/wasmtime-v36.0.2-x86_64-windows-c-api.zip'
data=urllib.request.urlopen(url,timeout=60).read()
digest=hashlib.sha256(data).hexdigest()
assert digest=='ae93e1949d169ac4412c735c08ecb4dee841023549d83f69c6e98cefca22e734'
archive=zipfile.ZipFile(io.BytesIO(data))
name='wasmtime-v36.0.2-x86_64-windows-c-api/lib/wasmtime.dll'
dll=archive.read(name)
targets=list((cache/'wasmtime').rglob('_wasmtime.dll'));assert len(targets)==1
targets[0].write_bytes(dll)
provenance=dict(pythonBindingVersion='36.0.0',nativeEngineVersion='36.0.2',sourceUrl=url,releaseArchiveSha256=digest,dllPath=str(targets[0]),dllSha256=hashlib.sha256(dll).hexdigest(),limitations=['Official independent C-API engine, not Studio executable','Release archive verified and consumed in memory; no ZIP retained'])
args.provenance.parent.mkdir(parents=True,exist_ok=True)
args.provenance.write_text(json.dumps(provenance,indent=2)+'\n',encoding='utf-8')
print('Verification engine ready; start a fresh process with this cache as PYTHONPATH')
