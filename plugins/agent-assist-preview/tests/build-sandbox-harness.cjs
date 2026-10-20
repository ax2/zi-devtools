const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const pluginRoot = path.join(__dirname, '..');
const viewPath = path.join(pluginRoot, 'views', 'main.html');
const viewHtml = fs.readFileSync(viewPath, 'utf8');
if (/<script\s+src=/i.test(viewHtml)) throw new Error('Plugin View contains an external script');

const channel = 'agent-assist-preview-test-channel';
const csp = "default-src 'none'; img-src data: blob:; media-src data: blob:; font-src data:; " +
  "style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; " +
  "frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'";

function sandboxBridge(channel) {
  const namespace = 'zicode-plugin-view';
  const pending = new Map();
  let sequence = 0;
  let resolveContext;
  const context = new Promise((resolve) => { resolveContext = resolve; });
  function send(message) { parent.postMessage({ namespace, channel, ...message }, '*'); }
  function request(method, params) {
    const requestId = `view-${++sequence}`;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        pending.delete(requestId);
        reject(new Error('Plugin view request timed out'));
      }, 5000);
      pending.set(requestId, { resolve, reject, timeout });
      send({ type: 'request', requestId, method, params });
    });
  }
  window.addEventListener('message', (event) => {
    if (event.source !== parent) return;
    const message = event.data;
    if (message?.namespace !== namespace || message.channel !== channel) return;
    if (message.type === 'context') { resolveContext(message.context); return; }
    if (message.type === 'qa') {
      const report = (result) => send({ type: 'qa-result', id: message.id, result });
      try {
        if (message.action === 'state') {
          report({ ready: document.querySelector('#model-status')?.textContent || '',
            modelStatusKind: document.querySelector('#model-status')?.dataset.kind || '',
            generation: document.querySelector('#generation-status')?.textContent || '',
            generationKind: document.querySelector('#generation-status')?.dataset.kind || '',
            modelCount: document.querySelector('#model-select')?.options.length || 0,
            selectedModel: document.querySelector('#model-select')?.value || '',
            generateDisabled: document.querySelector('#generate')?.disabled ?? true,
            formDisabled: document.querySelector('#form-fields')?.disabled ?? true,
            resultHidden: document.querySelector('#result-card')?.hidden ?? true,
            result: document.querySelector('#result-text')?.textContent || '',
            resultContainsElement: Boolean(document.querySelector('#result-text img')),
            title: document.title,
            heading: document.querySelector('h1')?.textContent,
            overflow: document.documentElement.scrollWidth > window.innerWidth,
            innerWidth: window.innerWidth,
            stale: !document.querySelector('#result-state').hidden,
            resultState: document.querySelector('#result-state').textContent,
            goal: document.querySelector('#goal').value,
            materialCount: document.querySelectorAll('[data-material]').length,
            selection: window.getSelection().toString(),
            focus: document.activeElement?.id,
            clearDisabled: document.querySelector('#clear-result').disabled,
            frameElementIsOpaque: window.frameElement === null,
            csp: document.querySelector('meta[http-equiv="Content-Security-Policy"]')?.content || '' });
        } else if (message.action === 'fill') {
          const goal = document.querySelector('#goal'); goal.value = message.payload.goal;
          goal.dispatchEvent(new Event('input', { bubbles: true }));
          document.querySelector('#add-material').click();
          const card = document.querySelector('.material');
          card.querySelector('input').value = message.payload.name;
          const text = card.querySelector('textarea'); text.value = message.payload.text;
          text.dispatchEvent(new Event('input', { bubbles: true }));
          report({ bytes: document.querySelector('#materials-bytes').textContent });
        } else if (message.action === 'edit') {
          const element = document.querySelector(message.payload.selector);
          element.value = message.payload.value;
          element.dispatchEvent(new Event(message.payload.event || 'input', { bubbles: true }));
          report({changed: true});
        } else if (message.action === 'click') {
          document.querySelector(message.payload.selector).click(); report({clicked: true});
        } else if (message.action === 'top') {
          window.scrollTo(0,0); report({scrolled: true});
        } else if (message.action === 'submit') {
          document.querySelector('#generate').click();
          report({ generateDisabled: document.querySelector('#generate').disabled,
            selected: document.querySelector('#model-select').value });
        } else if (message.action === 'refresh') {
          document.querySelector('#refresh-models').click(); report({ requested: true });
        } else if (message.action === 'network') {
          fetch(`${location.origin}/blocked-probe`).then(() => report({ blocked: false }),
            () => report({ blocked: true }));
        } else report({ error: 'unknown_qa_action' });
      } catch (error) { report({ error: String(error?.message || error) }); }
      return;
    }
    if (message.type !== 'response') return;
    const operation = pending.get(message.requestId);
    if (!operation) return;
    pending.delete(message.requestId);
    clearTimeout(operation.timeout);
    if (message.error) operation.reject(new Error(message.error));
    else operation.resolve(message.result);
  });
  window.zicode = Object.freeze({
    ready: () => context,
    services: Object.freeze({
      models: Object.freeze({
        list: (cursor = null) => request('listServiceModels', { cursor }),
        generate: (input) => request('generateModel', input),
      }),
    }),
  });
  send({ type: 'ready' });
}

function buildSandboxDocument(html, channel) {
  const prefix = [
    `<meta http-equiv="Content-Security-Policy" content="${csp}">`,
    '<meta name="referrer" content="no-referrer">',
    '<meta name="viewport" content="width=device-width,initial-scale=1">',
    `<script>(${sandboxBridge.toString()})(${JSON.stringify(channel)});</script>`,
  ].join('');
  return html.replace(/<head(?:\s[^>]*)?>/i, (match) => `${match}${prefix}`);
}

const sandboxHtml = buildSandboxDocument(viewHtml, channel);
const viewBase64 = Buffer.from(sandboxHtml, 'utf8').toString('base64');
const harness = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Local plugin View sandbox harness</title>
<style>body{font:14px/1.5 system-ui,sans-serif;margin:0;background:#eef2f7;color:#182230}main{max-width:1100px;margin:24px auto;padding:0 18px}.bar{padding:12px 14px;background:white;border:1px solid #ccd5e0;border-radius:10px;margin-bottom:12px}#pending{display:none;white-space:pre-wrap;overflow-wrap:anywhere;padding:10px;background:#f5f7fa;border-radius:8px}button{padding:8px 12px;margin-right:8px}iframe{display:block;width:100%;height:980px;border:1px solid #ccd5e0;border-radius:12px;background:white}</style></head><body><main>
<section class="bar"><strong>模拟 Studio Host · 本机夹具</strong><div>请求只在本机内存中处理，不访问网络。列表读取和生成调用均由测试人员触发；生成前先展示完整 JSON。</div>
<pre id="pending" data-testid="pending-request"></pre><button id="approve" hidden>批准本机模拟请求</button><button id="reject" hidden>拒绝请求</button><div id="fixture-status" role="status">等待 View 就绪</div></section>
<iframe id="plugin-view" title="Agent 模型辅助测试 View" sandbox="allow-scripts"></iframe></main>
<script>
const channel=${JSON.stringify(channel)};const namespace='zicode-plugin-view';const frame=document.getElementById('plugin-view');
const pending=document.getElementById('pending');const approve=document.getElementById('approve');const reject=document.getElementById('reject');const status=document.getElementById('fixture-status');let activeRequest=null;
function respond(requestId,result,error){frame.contentWindow.postMessage({namespace,channel,type:'response',requestId,result,error},'*')}
window.addEventListener('message',(event)=>{if(event.source!==frame.contentWindow)return;const data=event.data;if(data?.namespace!==namespace||data.channel!==channel)return;
if(data.type==='ready'){status.textContent='View ready handshake received';frame.contentWindow.postMessage({namespace,channel,type:'context',context:{sceneId:'fixture-scene'}},'*');return}
if(data.type==='qa-result'){window.__qaResults[data.id]=data.result;return}
if(data.type!=='request')return;
if(data.method==='listServiceModels'){status.textContent='Model catalog requested by View';respond(data.requestId,{models:[{modelRef:'fixture-model-ref',displayName:'本机模拟模型 A',providerKind:'openai-compatible',supportsCancellation:false},{modelRef:'fixture-model-ref-b',displayName:'本机模拟模型 B',providerKind:'openai-compatible',supportsCancellation:false}],nextCursor:null});return}
if(data.method==='generateModel'){activeRequest=data;pending.textContent=JSON.stringify(data.params,null,2);pending.style.display='block';approve.hidden=false;reject.hidden=false;status.textContent='Waiting for explicit local fixture approval';return}
respond(data.requestId,null,'Unsupported fixture request');});
approve.addEventListener('click',()=>{if(!activeRequest)return;const request=activeRequest;activeRequest=null;approve.hidden=true;reject.hidden=true;status.textContent='Approved: responding locally without network';respond(request.requestId,{text:'本机模拟结果：Studio srcdoc + CSP 下，模型生成调用已提交并收到响应。 <img src=x onerror=alert(1)>'});});
reject.addEventListener('click',()=>{if(!activeRequest)return;const request=activeRequest;activeRequest=null;approve.hidden=true;reject.hidden=true;status.textContent='Rejected by fixture operator';respond(request.requestId,null,'Request rejected by local fixture');});
window.__qaResults={};let qaSequence=0;window.__qaSend=(action,payload={})=>{const id='qa-'+(++qaSequence);frame.contentWindow.postMessage({namespace,channel,type:'qa',id,action,payload},'*');return id};
const binary=atob('${viewBase64}');const bytes=Uint8Array.from(binary,character=>character.charCodeAt(0));frame.srcdoc=new TextDecoder('utf-8',{fatal:true}).decode(bytes);
</script></body></html>`;

const outputPath = path.join(os.tmpdir(), 'zi-agent-assist-preview-sandbox-harness.html');
fs.writeFileSync(outputPath, harness, 'utf8');
process.stdout.write(outputPath);
