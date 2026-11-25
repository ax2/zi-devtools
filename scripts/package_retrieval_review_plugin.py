"""Freeze the verified read-only retrieval evaluation report review plugin."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_retrieval_review_plugin import (
    CAPABILITY, CONTRACT, MAX_RESULT, MIME, PLUGIN, PROFILE, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "rag-eval"
SLUG = "retrieval-review"
VERSION = "0.1.0"
TITLE = "检索评测报告复核"
SCOPE = "复核本机 ZiDevTools 的仅检索或双路检索 JSON 报告，查看 Top K 命中、首个正确名次与 MRR。"


def manifest():
    boolean = {"type":"boolean"}
    nil = {"type":"null"}
    contract = {"type":"string","const":CONTRACT}

    def nullable(field):
        return {"oneOf":[field,nil]}

    case_number = {"type":"integer","minimum":1,"maximum":10}
    rank = nullable({"type":"integer","minimum":1,"maximum":8})
    mrr = nullable({"type":"number","minimum":0,"maximum":1})
    single_row = obj(caseNumber=case_number,firstRank=rank,recordedRank=rank,failed=boolean)
    pair_row = obj(caseNumber=case_number,keywordRank=rank,hybridRank=rank,keywordFailed=boolean,
                   hybridFailed=boolean,included=boolean)
    single = obj(retrieval={"type":"string","enum":["keyword","hybrid"]},failed=integer(10),
                 hits=integer(10),mrr=mrr,rows={"type":"array","maxItems":10,"items":single_row})
    comparison = obj(paired=integer(10),excluded=integer(10),keywordHits=integer(10),hybridHits=integer(10),
                     keywordOnly=integer(10),hybridOnly=integer(10),both=integer(10),neither=integer(10),
                     keywordMrr=mrr,hybridMrr=mrr,rows={"type":"array","maxItems":10,"items":pair_row})
    data = obj(format={"type":"string","enum":["retrieval-only","comparison"]},
               topK={"type":"integer","enum":[3,5,8]},total=case_number,completed=integer(10),
               cancelled=boolean,matchesReport=boolean,
               differences={"type":"array","maxItems":9,"items":string(32)},
               single=nullable(single),comparison=nullable(comparison),text=string(2400))
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),data=data,error=nil)
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    return dict(
        schemaVersion=2,id=PLUGIN,name="ZiDevTools Retrieval Evaluation Review",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,
            resourceInputMimeTypes=[MIME,"application/json","text/plain"],
            inputSchema=obj(),outputSchema={"type":"object","oneOf":[success,failure]},
            maxResultBytes=MAX_RESULT,directory=dict(categoryId="files",
                aliases=["检索评测","复核 MRR","RAG 评测报告","Review retrieval evaluation"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次确认的本机检索评测报告快照。")],
                         optional=[]),
    )


def main():
    parser,args=candidate_arguments(__doc__)
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package=ROOT/"release/plugin-candidates"/f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt=package.with_name(package.name+"-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module=args.module.read_bytes()
    proof=json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors=list(fixtures())
    expected=[(name,digest(data),mime,not isinstance(value,str))
              for name,data,inputs,mime,value in vectors]
    actual=[(row["case"],row["inputSha256"],row["mime"],row["ok"])
            for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["workerProfile"]!=PROFILE or proof["failures"] or proof["passed"]!=len(vectors) or actual!=expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/retrieval-review/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",
                                                  inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_retrieval_review_plugin.py",cases=[
        dict(id=name,inputSha256=digest(data),inputBytes=len(data),input=inputs,mime=mime,ok=not isinstance(value,str))
        for name,data,inputs,mime,value in vectors
    ]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
                                         builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",
                                         wasmProfile="plugin",moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=digest(path.read_bytes()))
           for path in sorted(package.rglob("*")) if path.is_file()]
    index=digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
                       directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(row["size"] for row in files),
                       directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
                                                   objectKeyOrder=["path","size","sha256"],ensureAscii=False,
                                                   separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__ == "__main__":
    main()
