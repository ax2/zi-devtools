"""Exercise Agent Assist inside Chromium's sandboxed iframe and CSP."""
import argparse
import base64
import hashlib
import http.server
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

from cdp_client import DevTools

ROOT = Path(__file__).resolve().parents[1]
HARNESS_BUILDER = ROOT / 'plugins/agent-assist-preview/tests/build-sandbox-harness.cjs'
parser = argparse.ArgumentParser()
parser.add_argument('--width', type=int, default=1180)
parser.add_argument('--height', type=int, default=900)
parser.add_argument('--theme', choices=['light', 'dark'], default='light')
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--screenshot', type=Path)
args = parser.parse_args()

harness_path = Path(subprocess.check_output(
    ['node', str(HARNESS_BUILDER)], cwd=ROOT, text=True, encoding='utf-8').strip()).resolve(strict=True)
harness = harness_path.read_bytes()
assert b'iframe id="plugin-view"' in harness and b'sandbox="allow-scripts"' in harness

class Handler(http.server.BaseHTTPRequestHandler):
    hits = []
    body = harness
    def do_GET(self):
        self.hits.append(self.path)
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(self.body)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(self.body)
    def log_message(self, *_):
        pass

browser = shutil.which('msedge.exe') or shutil.which('chrome.exe')
for candidate in [Path('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'),
                  Path('C:/Program Files/Google/Chrome/Application/chrome.exe')]:
    if browser is None and candidate.is_file(): browser = str(candidate)
assert browser, 'Microsoft Edge or Google Chrome is required'
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
server.daemon_threads = True
thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
profile = tempfile.mkdtemp(prefix='zi-agent-assist-cdp-')
proxyless = urllib.request.build_opener(urllib.request.ProxyHandler({})).open
process = None; cdp = None

def wait_for(read, predicate, label, timeout=12):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = read()
        if predicate(value): return value
        time.sleep(.05)
    raise AssertionError(f'Timed out waiting for {label}; last value={value!r}')

def evaluate(expression, context_id=None):
    params = {'expression': expression, 'returnByValue': True, 'awaitPromise': True}
    if context_id is not None: params['contextId'] = context_id
    result = cdp.command('Runtime.evaluate', params)
    if result.get('exceptionDetails'): raise AssertionError(result['exceptionDetails'])
    return result.get('result', {}).get('value')

def ask(action, payload=None):
    payload = json.dumps(payload or {}, ensure_ascii=False)
    identifier = cdp.evaluate(f"window.__qaSend({json.dumps(action)}, {payload})")
    expression = f"window.__qaResults[{json.dumps(identifier)}]"
    return wait_for(lambda: cdp.evaluate(expression), lambda value: value is not None, f'QA action {action}')

try:
    command = [browser, '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
               '--disable-extensions', '--disable-background-networking', '--no-proxy-server',
               '--remote-debugging-port=0', f'--user-data-dir={profile}', 'about:blank']
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                               creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    port_file = Path(profile) / 'DevToolsActivePort'; deadline = time.monotonic() + 15
    while not port_file.is_file() and process.poll() is None and time.monotonic() < deadline: time.sleep(.05)
    assert port_file.is_file(), 'Chromium did not expose a DevTools endpoint'
    port = int(port_file.read_text(encoding='utf-8').splitlines()[0]); target = None; deadline = time.monotonic() + 15
    while target is None and time.monotonic() < deadline:
        try:
            with proxyless(f'http://127.0.0.1:{port}/json/list', timeout=2) as response: rows = json.load(response)
            target = next((row for row in rows if row.get('type') == 'page'), None)
        except (OSError, urllib.error.URLError, json.JSONDecodeError): pass
        if target is None: time.sleep(.1)
    assert target, 'Chromium page target unavailable'
    cdp = DevTools(target['webSocketDebuggerUrl'])
    cdp.command('Page.enable'); cdp.command('Runtime.enable'); cdp.command('Log.enable')
    cdp.command('Emulation.setDeviceMetricsOverride', {'width': args.width, 'height': args.height,
              'deviceScaleFactor': 1, 'mobile': args.width < 600})
    cdp.command('Emulation.setEmulatedMedia', {'features': [{'name': 'prefers-color-scheme', 'value': args.theme}]})
    cdp.command('Page.navigate', {'url': f'http://127.0.0.1:{server.server_port}/'})
    wait_for(lambda: cdp.evaluate("document.readyState"), lambda state: state == 'complete', 'harness document')
    assert cdp.evaluate("document.getElementById('plugin-view')?.srcdoc?.length || 0") > 1000, 'Harness did not populate srcdoc'
    wait_for(lambda: cdp.evaluate("document.getElementById('fixture-status').textContent"),
             lambda value: value == 'View ready handshake received', 'sandbox handshake')
    ready_state = wait_for(lambda: ask('state'), lambda state: state['modelStatusKind'] == 'success', 'View ready state')
    assert ready_state['title'] == 'ZiDevTools 模型辅助' and ready_state['heading'] == '模型辅助任务'
    assert not ready_state['overflow']
    assert 0 < ready_state['innerWidth'] <= args.width, 'Fixture must not silently use a desktop layout viewport on mobile'
    assert ready_state['frameElementIsOpaque'] is True, 'Sandbox frameElement access should be denied'
    assert "connect-src 'none'" in ready_state['csp']
    network_result = ask('network')
    assert network_result == {'blocked': True}, network_result
    assert not [path for path in Handler.hits if path not in ('/', '/favicon.ico')], f'CSP allowed an external request: {Handler.hits}'
    ask('refresh')
    model_state = wait_for(lambda: ask('state'), lambda state: state['modelCount'] == 2 and not state['formDisabled'], 'model catalog')
    ask('edit', {'selector': '#model-select', 'value': '1', 'event': 'change'})
    form_result = ask('fill', {'goal': '请核对这段用户提供的资料。', 'name': '本机手动资料',
                               'text': 'untrusted fixture text'})
    assert form_result == {'bytes': '22'}
    submit_ack = ask('submit')
    assert submit_ack['generateDisabled'] and submit_ack['selected'] == '1', submit_ack
    assert ask('state')['formDisabled']
    observed = wait_for(lambda: (cdp.evaluate("document.querySelector('#pending').textContent"), ask('state')),
                        lambda value: bool(value[0]) or value[1]['generationKind'] == 'error',
                        'Host confirmation preview or validation error')
    pending, submit_state = observed
    assert pending, f'Generation did not reach Host; View status={submit_state!r}'
    request = json.loads(pending)
    assert set(request) == {'modelRef', 'goal', 'materials', 'maxOutputTokens', 'deadlineMs'}
    assert request['goal'] == '请核对这段用户提供的资料。' and request['modelRef'] == 'fixture-model-ref-b'
    assert request['materials'] == [{'sourceId': request['materials'][0]['sourceId'], 'displayName': '本机手动资料',
                                     'sourceKind': 'user_text', 'trust': 'untrusted_user_material', 'text': 'untrusted fixture text'}]
    assert cdp.evaluate("document.querySelector('#approve').hidden") is False
    cdp.evaluate("document.querySelector('#reject').click()")
    rejected_state = wait_for(lambda: ask('state'), lambda state: state['generationKind'] == 'error', 'request rejection')
    assert rejected_state['resultHidden'] is True
    assert rejected_state['selectedModel'] == '1'
    ask('submit')
    wait_for(lambda: cdp.evaluate("document.querySelector('#approve').hidden"), lambda hidden: hidden is False,
             'second confirmation')
    cdp.evaluate("document.querySelector('#approve').click()")
    wait_for(lambda: ask('state')['resultHidden'], lambda hidden: hidden is False,
             'approved local fixture response')
    result_text = ask('state')['result']
    assert '本机模拟结果' in result_text
    assert '<img ' in result_text and ask('state')['resultContainsElement'] is False
    assert ask('state')['selectedModel'] == '1' and not ask('state')['stale']
    ask('edit', {'selector': '#goal', 'value': '修改后的任务目标'})
    assert ask('state')['stale']
    ask('submit')
    wait_for(lambda: cdp.evaluate("document.querySelector('#approve').hidden"), lambda hidden: hidden is False, 'third confirmation')
    assert ask('state')['clearDisabled'] and '上次回答' in ask('state')['resultState']
    cdp.evaluate("document.querySelector('#reject').click()")
    wait_for(lambda: ask('state')['formDisabled'], lambda value: not value, 'rejection unlock')
    assert ask('state')['result'] == result_text and ask('state')['stale']
    ask('click', {'selector': '#show-result'})
    assert ask('state')['focus'] == 'result-text'
    ask('click', {'selector': '#select-result'})
    assert ask('state')['selection'] == result_text
    # Keep the actual sandbox, but show its viewport without the fixture operator toolbar.
    cdp.evaluate("document.querySelector('.bar').hidden=true; document.querySelector('main').style.cssText='margin:0;padding:0;max-width:none'; document.querySelector('iframe').style.cssText='width:100%;height:100vh;border:0;border-radius:0'; window.scrollTo(0,0)")
    ask('click', {'selector': '#show-result'})
    screenshot = args.screenshot or args.output.with_suffix('.png')
    screenshot.parent.mkdir(parents=True, exist_ok=True)
    result_screenshot = screenshot.with_stem(screenshot.stem + '-result')
    result_png = base64.b64decode(cdp.command('Page.captureScreenshot', {'format': 'png', 'captureBeyondViewport': False})['data'])
    result_screenshot.write_bytes(result_png)
    before_clear = ask('state')
    ask('click', {'selector': '#clear-result'})
    cleared = ask('state')
    assert cleared['resultHidden'] and cleared['result'] == '' and not cleared['stale']
    assert cleared['goal'] == before_clear['goal'] and cleared['materialCount'] == before_clear['materialCount'] == 1
    assert cleared['focus'] == 'goal' and not cleared['overflow']
    ask('top')
    assert not [path for path in Handler.hits if path not in ('/', '/favicon.ico')], f'Unexpected iframe/browser network request: {Handler.hits}'
    errors = [event for event in cdp.events if event.get('method') == 'Runtime.exceptionThrown']
    assert not errors, errors
    screenshot = args.screenshot or args.output.with_suffix('.png')
    screenshot.parent.mkdir(parents=True, exist_ok=True)
    png = base64.b64decode(cdp.command('Page.captureScreenshot', {'format': 'png', 'captureBeyondViewport': False})['data'])
    assert png[:8] == b'\x89PNG\r\n\x1a\n'
    screenshot.write_bytes(png)
    proof = {
        'pass': True, 'browser': Path(browser).name, 'viewSha256': hashlib.sha256((ROOT/'plugins/agent-assist-preview/views/main.html').read_bytes()).hexdigest(),
        'harnessSha256': hashlib.sha256(harness).hexdigest(), 'viewport': {'width': args.width, 'height': args.height,
        'devicePixelRatio': 1, 'prefersDark': args.theme == 'dark'}, 'sandbox': 'allow-scripts; opaque origin; frameElement inaccessible',
        'initialFrameWidth': ready_state['innerWidth'],
        'csp': "default-src 'none'; script/style inline; connect-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'",
        'modelsReadByUserAction': 1, 'generationRequests': 3,
        'resultInteractions': {'modelSelectionPreserved': True, 'inputsLockedWhilePending': True,
            'staleInputWarning': True, 'rejectionPreservedPreviousAnswer': True,
            'resultJumpFocus': True, 'selectionMatchesAnswer': True, 'clearPreservesDraft': True,
            'noHorizontalOverflow': True},
        'rejectionKeptResultHidden': True, 'approvalReturnedLocalFixture': True, 'userMaterialMarkedUntrusted': True,
        'outerPageRequests': Handler.hits, 'runtimeExceptions': errors,
        'screenshot': screenshot.name, 'screenshotSha256': hashlib.sha256(png).hexdigest(),
        'resultScreenshot': result_screenshot.name, 'resultScreenshotSha256': hashlib.sha256(result_png).hexdigest(),
        'limitations': ['Mock Studio Host; no real model provider or network generation',
                        'Does not verify Studio native permission UI, profile negotiation, or plugin installation']}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(f"Agent Assist sandbox View passed: {proof['browser']}, {args.width}x{args.height} {args.theme}; no external page requests")
finally:
    if cdp:
        try: cdp.command('Browser.close')
        except Exception: pass
        cdp.close()
    if process and process.poll() is None:
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, check=False)
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
    server.shutdown(); server.server_close()
    shutil.rmtree(profile, ignore_errors=True)
    if (harness_path.parent == Path(tempfile.gettempdir()) and
            harness_path.name == 'zi-agent-assist-preview-sandbox-harness.html'):
        harness_path.unlink(missing_ok=True)
assert proof['pass']
