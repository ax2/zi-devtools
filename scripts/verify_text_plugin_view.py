"""Exercise the bundled text View through its public SDK shape and actual WASI module."""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import urllib.request

from cdp_client import DevTools

ROOT = Path(__file__).resolve().parents[1]
VIEW = ROOT / 'plugins/text/views/main.html'
parser = argparse.ArgumentParser()
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
wasm = args.wasm.resolve()
module_sha = hashlib.sha256(wasm.read_bytes()).hexdigest()
node = shutil.which('node')
assert node, 'Node is required only for local WASI View verification'


class Handler(http.server.BaseHTTPRequestHandler):
    calls = []
    html = ''

    def do_GET(self):
        body = self.html.encode('utf-8')
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        assert 0 < length <= 48 * 1024
        request = json.loads(self.rfile.read(length))
        Handler.calls.append(request['capabilityId'])
        encoded = json.dumps(request, ensure_ascii=False, separators=(',', ':')).encode()
        result = subprocess.run(
            [node, '--disable-warning=ExperimentalWarning', str(ROOT / 'scripts/run_text_wasi.mjs'), str(wasm)],
            input=encoded, capture_output=True, timeout=6, check=True)
        assert not result.stdout == b''
        body = result.stdout
        self.send_response(200)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


sdk = r'''<script>
// Browser automation must not stall on the View's explicit replace/clear confirms.
window.confirm=()=>true;
window.zicode={ready:async()=>{},context:{get:async()=>({})},setDirty:()=>{},capabilities:{invoke:async(capabilityId,input)=>{
(window.__calls||(window.__calls=[])).push(capabilityId);
const response=await fetch('/invoke',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({pluginId:'com.zicode.devtools.text',sceneId:'view-proof',capabilityId,commandId:null,input})});
return await response.json();}}};
</script>'''
source = VIEW.read_text(encoding='utf-8')
Handler.html = source.replace('<script>\n(() => {', sdk + '\n<script>\n(() => {', 1)
assert Handler.html != source
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
server.daemon_threads = True
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
edge = shutil.which('msedge.exe') or shutil.which('chrome.exe')
for candidate in [
    Path(os.environ.get('ProgramFiles(x86)', 'C:/Program Files (x86)')) / 'Microsoft/Edge/Application/msedge.exe',
    Path(os.environ.get('ProgramFiles', 'C:/Program Files')) / 'Google/Chrome/Application/chrome.exe',
]:
    if edge is None and candidate.is_file():
        edge = str(candidate)
assert edge, 'Install Microsoft Edge or Google Chrome for browser View verification'
local_open = urllib.request.build_opener(urllib.request.ProxyHandler({})).open
results = []
try:
    for theme, width, height in [('light', 1180, 900), ('dark', 390, 844)]:
        Handler.calls = []
        with tempfile.TemporaryDirectory(prefix='zitext-browser-profile-') as profile:
            process = subprocess.Popen(
                [edge, '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
                 '--disable-extensions', '--disable-background-networking', '--no-proxy-server',
                 f'--user-data-dir={profile}', '--remote-debugging-port=0', '--window-size=1280,1000', 'about:blank'],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
            cdp = None
            try:
                port_file = Path(profile) / 'DevToolsActivePort'
                deadline = time.monotonic() + 15
                while not port_file.is_file() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(.05)
                assert port_file.is_file(), 'Chromium did not open DevTools'
                port = int(port_file.read_text(encoding='utf-8').splitlines()[0])
                with local_open(f'http://127.0.0.1:{port}/json/list', timeout=3) as response:
                    target = next(row for row in json.load(response) if row.get('type') == 'page')
                cdp = DevTools(target['webSocketDebuggerUrl'])
                # The first WASI-backed interaction can take longer on cold browser
                # startup; keep the CDP transport alive beyond the UI's 6s invoke guard.
                cdp.socket.settimeout(30)
                cdp.command('Page.enable')
                cdp.command('Runtime.enable')
                cdp.command('Log.enable')
                cdp.command('Emulation.setDeviceMetricsOverride',
                            {'width': width, 'height': height, 'deviceScaleFactor': 1, 'mobile': width < 650})
                cdp.command('Emulation.setEmulatedMedia', {'features': [{'name': 'prefers-color-scheme', 'value': theme}]})
                cdp.command('Page.navigate', {'url': f'http://127.0.0.1:{server.server_port}/'})
                deadline = time.monotonic() + 25
                while time.monotonic() < deadline and not cdp.evaluate("!!document.querySelector('#run')&&!document.querySelector('#run').disabled"):
                    time.sleep(.05)
                assert cdp.evaluate("!!document.querySelector('#run')&&!document.querySelector('#run').disabled")
                def run_operation(operation, value, expected):
                    operation_json = json.dumps(operation, ensure_ascii=False)
                    value_json = json.dumps(value, ensure_ascii=False)
                    cdp.evaluate(f"""(()=>{{const s=document.querySelector('#operation');s.value={operation_json};s.dispatchEvent(new Event('change',{{bubbles:true}}));const i=document.querySelector('#input');i.value={value_json};i.dispatchEvent(new Event('input',{{bubbles:true}}));document.querySelector('#run').click()}})()""")
                    deadline = time.monotonic() + 10
                    while time.monotonic() < deadline and cdp.evaluate("document.querySelector('#status').textContent!=='处理完成。'"):
                        time.sleep(.025)
                    assert cdp.evaluate("document.querySelector('#status').textContent") == '处理完成。', operation
                    actual = cdp.evaluate("document.querySelector('#output').textContent")
                    assert actual == expected, (operation, actual, expected)
                    deadline = time.monotonic() + 5
                    while time.monotonic() < deadline and cdp.evaluate("document.querySelector('#run').disabled"):
                        time.sleep(.01)
                    assert not cdp.evaluate("document.querySelector('#run').disabled"), operation

                examples = [
                    ('json.validate', '{"name":"Zi"}', 'JSON 有效（插件兼容规则）'),
                    ('json.format', '{"name":"Zi","ok":true}', '{\n  "name": "Zi",\n  "ok": true\n}'),
                    ('json.minify', '{ "name" : "Zi", "ok" : true }', '{"name":"Zi","ok":true}'),
                    ('sql.format', 'select id,name from users where active=true', 'SELECT id, name\nFROM users\nWHERE active = TRUE'),
                    ('base64.encode', 'Hello, 世界', base64.b64encode('Hello, 世界'.encode('utf-8')).decode('ascii')),
                    ('base64.decode', base64.b64encode('Hello, 世界'.encode('utf-8')).decode('ascii'), 'Hello, 世界'),
                    ('sha256', 'abc', 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'),
                ]
                for operation, value, expected in examples:
                    run_operation(operation, value, expected)
                cdp.evaluate("""(()=>{const s=document.querySelector('#operation');s.value='json.validate';s.dispatchEvent(new Event('change',{bubbles:true}));const i=document.querySelector('#input');i.value='{"x":1,"x":2}';i.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('#run').click()})()""")
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline and not cdp.evaluate("document.querySelector('#status').textContent.startsWith('INVALID_INPUT')"):
                    time.sleep(.025)
                assert cdp.evaluate("document.querySelector('#status').textContent.startsWith('INVALID_INPUT')")
                assert cdp.evaluate("document.querySelector('#output').textContent==='未生成结果'")
                calls = ['devtools.text.' + operation for operation, _, _ in examples]
                calls = calls[:1] + calls[1:] + ['devtools.text.json.validate']
                assert Handler.calls == calls
                assert cdp.evaluate('window.__calls') == calls
                errors = [event for event in cdp.events
                          if event.get('method') == 'Runtime.exceptionThrown'
                          or (event.get('method') == 'Log.entryAdded'
                              and event.get('params', {}).get('entry', {}).get('level') == 'error')]
                assert not errors, errors
                image = cdp.command('Page.captureScreenshot', {'format': 'png', 'captureBeyondViewport': False})
                screenshot_bytes = base64.b64decode(image['data'])
                args.output.parent.mkdir(parents=True, exist_ok=True)
                screenshot = args.output.parent / f'text-view-{theme}.png'
                screenshot.write_bytes(screenshot_bytes)
                results.append({'theme': theme, 'width': width, 'height': height, 'devicePixelRatio': 1,
                                'pass': True, 'actualWasi': True, 'actions': ['json.validate.valid', 'json.format', 'json.minify', 'sql.format', 'base64.encode', 'base64.decode', 'sha256', 'json.validate.invalid'],
                                'calls': len(calls), 'pageErrors': errors, 'screenshot': screenshot.name,
                                'screenshotSha256': hashlib.sha256(screenshot_bytes).hexdigest()})
            finally:
                if cdp:
                    try:
                        cdp.command('Browser.close')
                    except Exception:
                        pass
                    cdp.close()
                if process.poll() is None:
                    process.wait(timeout=5)
    proof = {'viewSha256': hashlib.sha256(VIEW.read_bytes()).hexdigest(), 'wasmSha256': module_sha,
             'offlinePass': True, 'results': results,
             'limitations': ['local SDK-shaped bridge executes the actual WASI module through Node',
                             'not Studio Host installation, authorization, Pi, worker isolation, or lifecycle acceptance']}
    args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f"PASS text plugin View: {len(results)} themes, all 7 operations plus rejected JSON validation")
finally:
    server.shutdown()
    server.server_close()
    thread.join(timeout=2)
