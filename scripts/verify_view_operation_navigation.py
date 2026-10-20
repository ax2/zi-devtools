"""Render both provider Views in opaque sandbox frames; mock only the public SDK."""
import argparse
import hashlib
import html
import json
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
BOOTSTRAP = r"""
window.__calls={list:0,generate:0,http:0,executed:0};window.__pending=null;
window.__openOperation=null;
const gated=(kind,request)=>{window.__calls[kind]++;return new Promise((resolve,reject)=>{
  window.__pending={kind,request,resolve,reject};
})};
window.__confirm=approved=>{
  const p=window.__pending;if(!p)throw Error('No pending confirmation');window.__pending=null;
  if(!approved){p.reject(Error('user_declined'));return;}
  window.__calls.executed++;
  p.resolve(p.kind==='generate'?{text:'Synthetic retained answer'}:{schemaVersion:1,operation:'http.get',
    result:{endpoint:p.request.endpoint,status:200,contentType:'application/json',byteLength:15,data:{status:'UP'}}});
};
window.zicode={ready:async()=>({}),context:{get:async()=>({})},
 navigation:{onOpenViewOperation:handler=>{window.__openOperation=handler;
   return {dispose:()=>{window.__openOperation=null}}}},
 services:{models:{list:async()=>{window.__calls.list++;return {models:[{modelRef:'fixture-model',displayName:'本机测试模型'}],nextCursor:null}},
   generate:request=>gated('generate',request)},http:{get:request=>gated('http',request)}}};
"""
STATE = """() => ({calls:{...window.__calls},
  values:[...document.querySelectorAll('input,textarea,select')].map(e=>[e.id,e.value]),
  result:document.querySelector('#result-text')?.textContent,
  summary:document.querySelector('#summary')?.textContent,
  summaryHidden:document.querySelector('#summary')?.hidden})"""


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    rows = []
    with sync_playwright() as p:
        browser = p.chromium.launch(channel='msedge', headless=True)
        for name in ['agent-assist-preview', 'spring-actuator']:
            source = ROOT/'plugins'/name
            manifest = json.loads((source/'plugin.json').read_bytes())
            agent = name == 'agent-assist-preview'
            for theme, width, height in [('light', 1180, 900), ('dark', 390, 844)]:
                context = browser.new_context(viewport={'width':width,'height':height}, color_scheme=theme)
                page = context.new_page()
                errors = []
                page.on('pageerror', lambda e: errors.append(str(e)))
                page.on('console', lambda m: errors.append(m.text) if m.type in ['error','warning'] else None)
                csp = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'"
                view = (source/'views/main.html').read_text(encoding='utf-8')
                view = view.replace('<head>', '<head><meta http-equiv="Content-Security-Policy" content="'+csp+'"><script>'+BOOTSTRAP+'</script>', 1)
                wrapper = '<!doctype html><meta charset="utf-8"><title>Provider navigation fixture</title><style>html,body{margin:0;height:100%}iframe{border:0;width:100%;height:100%}</style><iframe sandbox="allow-scripts" srcdoc="'+html.escape(view, quote=True)+'"></iframe>'
                url = 'https://provider-fixture.invalid/'+name
                page.route(url, lambda route: route.fulfill(status=200, content_type='text/html', body=wrapper))
                page.goto(url)
                frame = page.frames[1]
                frame.wait_for_function('typeof window.__openOperation === "function"')
                frame.wait_for_load_state('domcontentloaded')
                assert page.url == url
                expected_title = 'ZiDevTools 模型辅助' if agent else 'Spring Actuator 只读诊断'
                frame.wait_for_function('(title)=>document.title===title', arg=expected_title)
                assert frame.evaluate('document.title') == expected_title
                assert frame.locator('h1').inner_text()
                assert frame.evaluate('window.frameElement === null'), 'opaque sandbox frame'
                assert frame.evaluate('window.__calls.executed') == 0
                if agent:
                    # A new View can locate generation even before any model is loaded.
                    assert frame.evaluate("window.__openOperation({operationId:'devtools.agent-assist.generate'})") == 'opened'
                    assert frame.evaluate('document.activeElement.id') == 'limits-heading'
                    assert frame.evaluate('window.__calls.list') == 0
                    frame.locator('#refresh-models').click()
                    frame.locator('#goal').fill('保留我的目标，不要自动发送')
                    frame.locator('#add-material').click()
                    frame.locator('.material textarea').fill('需要保留的本地草稿')
                    run = frame.locator('#generate')
                else:
                    # Empty summary is a navigable panel, not an implicit HTTP request.
                    assert frame.evaluate("window.__openOperation({operationId:'devtools.spring-actuator.summary'})") == 'opened'
                    assert frame.evaluate('document.activeElement.id') == 'result-title'
                    frame.locator('#origin').fill('https://service.example.test')
                    run = frame.locator('#run')
                # Explicit action is still gated; declining produces no execution.
                run.click()
                frame.wait_for_function('window.__pending !== null')
                for op in manifest['contributes']['viewOperations']:
                    before = frame.evaluate(STATE)
                    assert frame.evaluate('(id)=>window.__openOperation({operationId:id})', op['id']) == 'unsupported'
                    assert frame.evaluate(STATE) == before, op['id']+' while busy'
                assert frame.evaluate('window.__calls.executed') == 0
                frame.evaluate('window.__confirm(false)')
                run.wait_for(state='visible')
                frame.wait_for_function('!document.getElementById('+json.dumps('generate' if agent else 'run')+').disabled')
                assert frame.evaluate('window.__calls.executed') == 0
                run.click()
                frame.wait_for_function('window.__pending !== null')
                frame.evaluate('window.__confirm(true)')
                frame.wait_for_function('!document.getElementById('+json.dumps('generate' if agent else 'run')+').disabled')
                assert frame.evaluate('window.__calls.executed') == 1
                for op in manifest['contributes']['viewOperations']:
                    before = frame.evaluate(STATE)
                    status = frame.evaluate('(id)=>window.__openOperation({operationId:id})', op['id'])
                    assert status == 'opened', op['id']
                    target = ('refresh-models' if op['id'].endswith('.models') else 'generate') if agent else ('result-title' if op['id'].endswith('.summary') else 'endpoint')
                    assert frame.evaluate('document.activeElement.id') == target
                    after = frame.evaluate(STATE)
                    if not agent and target == 'endpoint':
                        before['values'] = [[key,op['id'].split('.')[-1] if key == 'endpoint' else value] for key,value in before['values']]
                    assert after == before, op['id']+' must preserve draft/results and request counts'
                    box = frame.locator('#'+target).bounding_box()
                    assert box and box['y'] >= 0 and box['y']+box['height'] <= height, box
                    assert not frame.evaluate('document.documentElement.scrollWidth > innerWidth'), 'horizontal overflow'
                    screenshot = args.output/(name+'-'+theme+'-'+op['id'].split('.')[-1]+'.png')
                    page.screenshot(path=str(screenshot))
                    rows.append({'operationId':op['id'],'theme':theme,'viewport':[width,height],
                                 'status':status,'focus':target,'busyStatus':'unsupported','noAutomaticRequest':True,
                                 'preservesDraftAndResult':True,'screenshot':screenshot.name,
                                 'viewSha256':digest(source/'views/main.html'),'manifestSha256':digest(source/'plugin.json')})
                for unknown in ['unknown.operation','toString','__proto__',None]:
                    before = frame.evaluate(STATE)
                    focused = frame.evaluate('document.activeElement.id')
                    assert frame.evaluate('(id)=>window.__openOperation({operationId:id})',unknown) == 'not_found'
                    assert frame.evaluate(STATE) == before
                    assert frame.evaluate('document.activeElement.id') == focused
                frame.evaluate("window.dispatchEvent(new Event('pagehide'))")
                assert frame.evaluate('window.__openOperation === null'), 'navigation registration disposed'
                assert not errors, errors
                context.close()
        browser.close()
    proof = {'status':'passed','browser':'Playwright / installed Edge, Browser plugin not available',
             'profile':'view.operation.directory.v1@=1.0.0-preview.1','cases':rows,
             'limitations':['Synthetic public SDK in opaque sandbox; no real Studio installation or transport acceptance',
                            'Approval/decline is simulated; no real model or HTTP request',
                            'Host revision, workspace, disabled-plugin and timeout isolation remain Host-owned acceptance'],
             'signatureVerified':False,'runtimeAccepted':False,'fullToolAcceptance':False}
    (args.output/'proof.json').write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'status':'passed','renderedOperationCases':len(rows),'proof':str(args.output/'proof.json')},ensure_ascii=False))


if __name__ == '__main__':
    main()
