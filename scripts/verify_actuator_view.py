"""Exercise the unregistered Spring Actuator View adapter with SDK-shaped fixtures."""
import argparse
import base64
import hashlib
import http.server
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.error
import urllib.request

from cdp_client import DevTools

ROOT = Path(__file__).resolve().parents[1]
VIEW = ROOT / 'plugins/diagnostics/views/spring-actuator-preview.html'
FIXTURES = ROOT / 'contracts/studio-host-capabilities/service-http-get-preview-20261015/fixtures/view-cases.json'
parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--screenshot', type=Path)
parser.add_argument('--width', type=int, default=1254)
parser.add_argument('--height', type=int, default=907)
parser.add_argument('--theme', choices=['light','dark'], default='dark')
args = parser.parse_args()
fixture = json.loads(FIXTURES.read_text(encoding='utf-8'))
contract_dir=FIXTURES.parent.parent
request_schema=json.loads((contract_dir/'schemas/sdk-request.schema.json').read_text(encoding='utf-8'))
result_schema=json.loads((contract_dir/'schemas/sdk-result.schema.json').read_text(encoding='utf-8'))
case_ids={case['id'] for case in fixture['cases']}
assert len(case_ids)==len(fixture['cases'])
assert request_schema['additionalProperties'] is False
assert request_schema['required']==['origin','endpoint']
assert request_schema['properties']['endpoint']['enum']==['health','info','metrics']
assert len(result_schema['oneOf'])==2
for case in fixture['cases']:
    request=case.get('request')
    if request:
        if case['id']=='reject-auth-field':
            assert set(request)=={'origin','endpoint','authorization'},case['id']
        else:
            assert set(request)=={'origin','endpoint'},case['id']
        if case['id']=='reject-unlisted-endpoint':
            assert request['endpoint']=='metrics/jvm.memory.used',case['id']
        else:
            assert request['endpoint'] in {'health','info','metrics'},case['id']
    response=case.get('result')
    if response:
        assert set(response)=={'endpoint','status','contentType','data','error'},case['id']
        if response['error'] is None:
            assert isinstance(response['status'],int) and response['data'] is not None,case['id']
        else:
            assert response['data'] is None and isinstance(response['error'].get('code'),str),case['id']
source = VIEW.read_text(encoding='utf-8')
for forbidden in ['fetch(', 'XMLHttpRequest', 'WebSocket(', 'Authorization:']:
    assert forbidden not in source, f'View contains a direct-network or credential primitive: {forbidden}'
assert "connect-src 'none'" in source

BOOTSTRAP = r'''<script>
window.__browserErrors=[];window.addEventListener('error',e=>window.__browserErrors.push(String(e.message||'page error')));
window.addEventListener('unhandledrejection',e=>window.__browserErrors.push(String(e.reason?.message||e.reason||'unhandled rejection')));
window.__calls=[];window.__mode='normal';window.__aborted=false;
window.zicode={ready:async()=>{},context:{get:async()=>({})},services:{http:{get:(request,options)=>{
window.__calls.push({request,signal:options?.signal,keys:Object.keys(request).sort()});
if(!(options?.signal instanceof AbortSignal))throw Error('AbortSignal missing');
if(window.__mode==='dns_changed')return Promise.resolve({endpoint:request.endpoint,status:null,contentType:null,data:null,error:{code:'dns_changed'}});
if(window.__mode==='pending')return new Promise(resolve=>options.signal.addEventListener('abort',()=>{window.__aborted=true;setTimeout(()=>resolve({endpoint:request.endpoint,status:200,contentType:'application/json',data:{late:'must-not-appear'},error:null}),120)},{once:true}));
if(window.__mode==='health_200')return Promise.resolve({endpoint:'health',status:200,contentType:'application/json',data:{status:'UP'},error:null});
if(request.endpoint==='health')return Promise.resolve({endpoint:'health',status:503,contentType:'application/problem+json',data:{status:'DOWN',components:{db:{status:'DOWN'}}},error:null});
if(request.endpoint==='info')return Promise.resolve({endpoint:'info',status:200,contentType:'application/json',data:{app:{name:'synthetic-demo'}},error:null});
return Promise.resolve({endpoint:'metrics',status:200,contentType:'application/vnd.spring-boot.actuator.v3+json',data:{names:['jvm.memory.used']},error:null});
}}}};
window.addEventListener('load',()=>{(async()=>{
const wait=async test=>{const end=Date.now()+5000;while(Date.now()<end){if(test())return;await new Promise(r=>setTimeout(r,10))}throw Error('View QA timeout')};
const origin=document.querySelector('#origin'),endpoint=document.querySelector('#endpoint'),run=document.querySelector('#run'),cancel=document.querySelector('#cancel'),status=document.querySelector('#status'),result=document.querySelector('#result');
await wait(()=>!run.disabled);
const choose=(address,op)=>{origin.value=address;origin.dispatchEvent(new Event('input',{bubbles:true}));endpoint.value=op;endpoint.dispatchEvent(new Event('change',{bubbles:true}))};
const send=async(address,op,mode='normal')=>{window.__mode=mode;choose(address,op);run.click()};
await send('http://127.0.0.1:8080','health','health_200');await wait(()=>status.textContent.includes('health JSON'));
if(!result.textContent.includes('UP'))throw Error('health 200 JSON was not rendered');
await send('http://127.0.0.1:8080','health');await wait(()=>status.textContent.includes('health 503 JSON'));
if(!result.textContent.includes('DOWN'))throw Error('health 503 JSON was not retained');
await send('https://service.example.invalid','info');await wait(()=>status.textContent.includes('info JSON'));
await send('https://service.example.invalid','metrics');await wait(()=>status.textContent.includes('metrics JSON'));
const before=window.__calls.length;
for(const [address,needle] of [['https://service.example.invalid/context','不能包含路径'],['https://user:synthetic@service.example.invalid','不能包含用户名或密码'],['https://service.example.invalid?x=1','不能包含路径、查询参数或片段'],['https://service.example.invalid/#x','不能包含路径、查询参数或片段']]){
origin.value=address;origin.dispatchEvent(new Event('input',{bubbles:true}));run.click();if(!status.textContent.includes(needle))throw Error('origin rejection failed: '+address);
}
choose('https://service.example.invalid','info');endpoint.value='metrics/jvm.memory.used';endpoint.dispatchEvent(new Event('change',{bubbles:true}));run.click();
if(!status.textContent.includes('不在 Host profile 允许列表'))throw Error('unlisted endpoint was not rejected');
if(window.__calls.length!==before)throw Error('invalid request reached the SDK');
await send('https://service.example.invalid','health','dns_changed');await wait(()=>status.textContent.includes('解析地址发生变化'));
if(result.textContent.includes('DOWN')||!result.textContent.includes('dns_changed'))throw Error('host error handling leaked or lost state');
await send('https://service.example.invalid','info','pending');await wait(()=>!cancel.disabled);cancel.click();await wait(()=>window.__aborted&&status.textContent.includes('请求取消'));
await new Promise(r=>setTimeout(r,180));if(result.textContent.includes('must-not-appear'))throw Error('late result replaced cancelled request state');
const expected=[
 ['http://127.0.0.1:8080','health'],['http://127.0.0.1:8080','health'],['https://service.example.invalid','info'],['https://service.example.invalid','metrics'],
 ['https://service.example.invalid','health'],['https://service.example.invalid','info']
];
if(window.__calls.length!==6||JSON.stringify(window.__calls.map(c=>[c.request.origin,c.request.endpoint]))!==JSON.stringify(expected))throw Error('SDK call set differs from fixed profile');
if(window.__calls.some(c=>JSON.stringify(c.keys)!==JSON.stringify(['endpoint','origin'])))throw Error('SDK request exposed an extra field');
if(!window.__calls[5].signal.aborted)throw Error('SDK cancellation signal was not aborted');
if(window.__browserErrors.length)throw Error('page errors: '+window.__browserErrors.join('; '));
const proof={pass:true,profile:'service.http.get.v1@1.0.0-preview.1',cases:['health-loopback-confirmed-200','health-loopback-confirmed-503','info-https-200','metrics-https-200','reject-base-path','reject-userinfo','reject-query','reject-fragment','reject-unlisted-endpoint','reject-auth-field','dns-change-error','cancel-discards-late-result'],sdkCalls:window.__calls.length,requestKeys:[...new Set(window.__calls.map(c=>c.keys.join(',')))],hostConfirmation:'SDK handoff observed; confirmation itself requires Studio Host',directNetworkPrimitives:false,connectSrcNone:true,lateResultIgnored:true,errors:window.__browserErrors,viewport:{width:innerWidth,height:innerHeight,devicePixelRatio,prefersDark:matchMedia('(prefers-color-scheme: dark)').matches}};
document.documentElement.setAttribute('data-actuator-proof',encodeURIComponent(JSON.stringify(proof)));
})().catch(e=>document.documentElement.setAttribute('data-actuator-error',encodeURIComponent(String(e))))});
</script>'''

class Handler(http.server.BaseHTTPRequestHandler):
    body = b''
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(self.body)))
        self.end_headers()
        self.wfile.write(self.body)
    def log_message(self, *_):
        pass

browser = shutil.which('msedge.exe') or shutil.which('chrome.exe')
for candidate in [Path('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'),
                  Path('C:/Program Files/Google/Chrome/Application/chrome.exe')]:
    if browser is None and candidate.is_file():
        browser = str(candidate)
assert browser, 'Microsoft Edge or Google Chrome is required for the View proof'
Handler.body = source.replace('<meta http-equiv="Content-Security-Policy"', BOOTSTRAP + '\n<meta http-equiv="Content-Security-Policy"', 1).encode('utf-8')
assert Handler.body != source.encode('utf-8')
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
server.daemon_threads = True
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
profile = tempfile.mkdtemp(prefix='zi-actuator-view-')
local_open=urllib.request.build_opener(urllib.request.ProxyHandler({})).open
process=None;cdp=None
try:
    screenshot=args.screenshot or args.output.with_suffix('.png')
    screenshot.parent.mkdir(parents=True,exist_ok=True)
    command=[browser,'--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check',
             '--disable-extensions','--disable-background-networking','--no-proxy-server',
             '--remote-debugging-port=0',f'--user-data-dir={profile}','about:blank']
    process=subprocess.Popen(command,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,
                             creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    port_file=Path(profile)/'DevToolsActivePort';deadline=time.monotonic()+15
    while not port_file.is_file() and process.poll() is None and time.monotonic()<deadline:time.sleep(.05)
    assert port_file.is_file(),'Chromium did not open its DevTools endpoint'
    port=int(port_file.read_text(encoding='utf-8').splitlines()[0]);target=None;deadline=time.monotonic()+15
    while target is None and time.monotonic()<deadline:
        try:
            with local_open(f'http://127.0.0.1:{port}/json/list',timeout=2) as response:targets=json.load(response)
            target=next((entry for entry in targets if entry.get('type')=='page'),None)
        except (OSError,urllib.error.URLError,json.JSONDecodeError):pass
        if target is None:time.sleep(.1)
    assert target,'Chromium did not expose an initial page target'
    cdp=DevTools(target['webSocketDebuggerUrl'])
    cdp.command('Page.enable');cdp.command('Runtime.enable');cdp.command('Log.enable')
    cdp.command('Emulation.setDeviceMetricsOverride',{'width':args.width,'height':args.height,'deviceScaleFactor':1,'mobile':args.width<600})
    cdp.command('Emulation.setEmulatedMedia',{'features':[{'name':'prefers-color-scheme','value':args.theme}]})
    cdp.command('Page.navigate',{'url':f'http://127.0.0.1:{server.server_port}/'})
    deadline=time.monotonic()+45;encoded_proof=None
    while time.monotonic()<deadline:
        encoded_proof=cdp.evaluate("document.documentElement.getAttribute('data-actuator-proof')")
        error=cdp.evaluate("document.documentElement.getAttribute('data-actuator-error')")
        if error:raise AssertionError(urllib.parse.unquote(error))
        if encoded_proof:break
        time.sleep(.05)
    assert encoded_proof,'View did not finish its SDK fixture suite'
    proof=json.loads(urllib.parse.unquote(encoded_proof))
    assert set(proof['cases'])==case_ids,proof['cases']
    assert proof['viewport']['width']==args.width and proof['viewport']['height']==args.height,proof['viewport']
    assert proof['viewport']['prefersDark']==(args.theme=='dark'),proof['viewport']
    for _ in range(3):cdp.evaluate('0');time.sleep(.05)
    page_errors=[event for event in cdp.events if event.get('method')=='Runtime.exceptionThrown'
                 or (event.get('method')=='Log.entryAdded' and event.get('params',{}).get('entry',{}).get('level')=='error')
                 or (event.get('method')=='Runtime.consoleAPICalled' and event.get('params',{}).get('type')=='error')]
    assert not page_errors,page_errors
    png=base64.b64decode(cdp.command('Page.captureScreenshot',{'format':'png','captureBeyondViewport':False})['data'])
    assert png[:8]==b'\x89PNG\r\n\x1a\n' and struct.unpack('!II',png[16:24])==(args.width,args.height)
    screenshot.write_bytes(png)
    proof['pageErrors']=page_errors
    proof.update(browser=Path(browser).name,viewSha256=hashlib.sha256(VIEW.read_bytes()).hexdigest(),
                 fixturesSha256=hashlib.sha256(FIXTURES.read_bytes()).hexdigest(),
                 limitations=['Injected SDK-shaped mock; no Studio Host, HTTP server, DNS, TLS, proxy, or confirmation was exercised',
                               'This proof validates View request/response handling only and is not Host conformance'])
    proof['screenshot']=screenshot.name
    proof['screenshotSha256']=hashlib.sha256(screenshot.read_bytes()).hexdigest()
finally:
    if cdp:
        try:cdp.command('Browser.close')
        except Exception:pass
        cdp.close()
    if process and process.poll() is None:
        try:process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            subprocess.run(['taskkill','/PID',str(process.pid),'/T','/F'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=False)
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
    server.shutdown();server.server_close()
    shutil.rmtree(profile,ignore_errors=True)
assert proof['pass']
args.output.parent.mkdir(parents=True,exist_ok=True)
args.output.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f"Actuator preview View passed: {proof['browser']}; {proof['sdkCalls']} SDK fixture calls; no direct network")
