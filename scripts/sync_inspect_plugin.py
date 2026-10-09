"""Generate format/inspection metadata from its complete Rust action registry."""
import argparse
import copy
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
source = (ROOT / 'crates/zi-inspect-core/src/lib.rs').read_text(encoding='utf-8')
matches = re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*title:\s*"([^"]+)"\s*,\s*group:\s*"([^"]+)"', source)
actions = [dict(id=id_, sourceToolId=tool, group=group, title=title) for id_, tool, title, group in matches]
assert len(actions) == 7 and len({a['id'] for a in actions}) == 7
versions = {t['id']: t['tool_version'] for t in json.loads((ROOT / 'docs/tools.json').read_text(encoding='utf-8'))['tools']}
manifest = json.loads((ROOT / 'contracts/studio-devtools/v1/plugin.example.json').read_text(encoding='utf-8'))
manifest.update(id='com.zicode.devtools.inspect', name='ZiDevTools Inspect', displayName='ZiDevTools 格式与检查',
                description='Complete bounded standalone format conversion and inspection operations; no Host resources.', version='0.1.0')
manifest['activationEvents'] = ['onView:devtools.inspect.main'] + ['onCapability:devtools.inspect.' + a['id'] for a in actions]
schema = dict(type='object', properties=dict(text=dict(type='string', maxLength=8192)), required=['text'], additionalProperties=False)
manifest['contributes'] = dict(
    activities=[dict(id='devtools.inspect', title='格式与检查', order=852)],
    views=[dict(id='devtools.inspect.main', activity='devtools.inspect', location='main', title='格式与检查', entry='views/main.html')],
    capabilities=[dict(id='devtools.inspect.' + a['id'], title=a['title'], inputSchema=copy.deepcopy(schema)) for a in actions],
    piTools=[dict(name='devtools_inspect_' + a['id'].replace('.', '_'), title=a['title'],
                  description=f"Run bounded local {a['title']} on supplied UTF-8 text, at most 8192 bytes. " +
                  ('JWT inspection never verifies signature, expiry or trust. ' if a['sourceToolId'] == 'jwt' else
                   'NFKC may change text meaning; character attention flags are not a full security audit. ' if a['sourceToolId'] == 'unicode' else '') + 'No file or network access.',
                  capability='devtools.inspect.' + a['id'], inputSchema=copy.deepcopy(schema)) for a in actions])
catalog = dict(contract='zicode.devtools-plugin/1.0.0-rc.1', packageId=manifest['id'], packageVersion=manifest['version'],
               source=dict(revision='REPLACE_WITH_PROVIDER_SOURCE_REVISION', dirty=True),
               tools=[dict(sourceToolId=a['sourceToolId'], toolId='devtools.inspect.' + a['id'], capabilityId='devtools.inspect.' + a['id'],
                           version=versions[a['sourceToolId']], status='candidate', profile='compute.wasi.v1', title=a['title']) for a in actions])
request = json.loads((ROOT / 'contracts/studio-devtools/v1/request.schema.json').read_text(encoding='utf-8'))
assert request['properties']['pluginId'] == {'const': 'com.zicode.devtools.text'}
request['properties']['pluginId']['const'] = manifest['id']
request['$comment'] = 'Derived from frozen rc.1 pilot request schema; only pluginId.const is specialized to this manifest ID.'
samples = dict(yaml='name: zi\nitems: [true, null, 中]\nwide: 9007199254740993\n', cidr='192.168.10.42/24',
               jwt='eyJhbGciOiJub25lIn0.eyJzdWIiOiJ6aSIsImlhdCI6MH0.', unicode='e\u0301 Ａ①🙂\u200b')
hints = dict(yaml='重复键使用最后一个值；保留大整数的文本表示，每次转换一个 YAML 文档。',
             cidr='IPv4 前缀 0–32；/31 为点对点两地址，/32 为单主机。不会连接或扫描网络。',
             jwt='只解码 Header 和 Payload，必须为 JSON 对象；不验证签名、有效期或可信度。请勿把输出当作身份凭证。',
             unicode='按码点而非字形列出 UTF-8 字节和常见不可见字符；NFC/NFKC 需主动选择，NFKC 可能改变字符语义。')
html = (ROOT / 'plugins/transforms/views/main.html').read_text(encoding='utf-8')
html = html.replace('ZiDevTools 文本转换', 'ZiDevTools 格式与检查').replace('<h1>文本转换</h1>', '<h1>格式与检查</h1>')
html = html.replace('编码、整理、检查和进制转换，在本地完成。', '转换配置格式，检查网络范围、JWT 内容和 Unicode 字符。')
html = html.replace('23 个操作', '7 个操作').replace('devtools.transforms.', 'devtools.inspect.')
html = html.replace('不会读取文件、访问网络、运行命令或发送给模型。', '本页不会主动读取文件、访问网络或运行命令；可选 AI 使用需要另行授权。')
host_messages = dict(worker_fuel_exhausted='处理超出计算预算，请缩小输入；重复提交相同内容通常无效。',
                     worker_memory_trap='处理超出内存预算，请缩小输入。', worker_stack_exhausted='嵌套超出处理预算，请减少输入层级。',
                     worker_timeout='处理超过宿主时间预算，请缩小输入。', worker_cancelled='处理已取消。',
                     plugin_disabled='插件已停用，请检查插件设置。', plugin_view_busy='宿主正在处理其他调用，请稍后再试。',
                     local_wait_timeout='页面等待超时不代表宿主任务已取消，请检查任务状态后再操作。')
html = html.replace('const messages=', 'const hostMessages=' + json.dumps(host_messages, ensure_ascii=False) + ';const messages=', 1)
html = html.replace("new Error('timeout')", "Object.assign(new Error('wait'),{code:'local_wait_timeout'})")
old = "catch{if(active===id&&state.revision===stamp)status('宿主拒绝、响应异常或等待超时，请检查授权与插件状态。输入和已有结果已保留。',true)}"
assert old in html
html = html.replace(old, "catch(error){if(active===id&&state.revision===stamp)status((Object.hasOwn(hostMessages,error?.code)?hostMessages[error.code]:'宿主拒绝、响应异常或等待超时，请检查授权与插件状态。')+' 输入和已有结果已保留。',true)}")
html, count = re.subn(r'const actions=\[.*?\], samples=\{.*?\}, \$=id', lambda _: 'const actions=' + json.dumps(actions, ensure_ascii=False) + ', samples=' + json.dumps(samples, ensure_ascii=False) + ', $=id', html, count=1)
assert count == 1
html, count = re.subn(r'const hints=\{.*?\};', lambda _: 'const hints=' + json.dumps(hints, ensure_ascii=False) + ';', html, count=1)
assert count == 1
html, count = re.subn(r"if\(id==='hex.decode'\).*?change\(value\)", lambda _: "if(id==='yaml.from_json')value=JSON.stringify({name:'zi',items:[true,null,'中'],wide:'9007199254740993'},null,2);change(value)", html, count=1)
assert count == 1
outputs = {'plugin.json': manifest, 'catalog.json': catalog, 'request.schema.json': request}
for name, value in outputs.items():
    content = json.dumps(value, ensure_ascii=False, indent=2) + '\n'
    path = ROOT / 'plugins/inspect' / name
    if args.check:
        assert path.read_text(encoding='utf-8') == content, f'Run sync_inspect_plugin.py: {name}'
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding='utf-8')
path = ROOT / 'plugins/inspect/views/main.html'
if args.check:
    assert path.read_text(encoding='utf-8') == html, 'Regenerate inspect View'
else:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(html, encoding='utf-8')
print('Inspect registry: 4 tools / 7 operations; metadata, grouped View and optional Pi declarations synchronized')
