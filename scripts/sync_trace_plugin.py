"""Generate complete trace operations; development metadata is not delivery."""
import argparse
import copy
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
source = (ROOT / 'crates/zi-trace-core/src/lib.rs').read_text(encoding='utf-8')
matches = re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*title:\s*"([^"]+)"\s*,\s*group:\s*"([^"]+)"', source)
actions = [dict(id=id_, sourceToolId=tool, title=title, group=group) for id_, tool, title, group in matches]
assert len(actions) == 2 and {a['id'] for a in actions} == {'java.trace', 'django.trace'}
versions = {t['id']: t['tool_version'] for t in json.loads((ROOT / 'docs/tools.json').read_text(encoding='utf-8'))['tools']}
manifest = json.loads((ROOT / 'contracts/studio-devtools/v1/plugin.example.json').read_text(encoding='utf-8'))
manifest.update(id='com.zicode.devtools.trace', name='ZiDevTools Trace', displayName='ZiDevTools 堆栈分析',
                description='Pasted Java and Python trace analysis; development build, not Host acceptance.', version='0.1.0')
manifest['activationEvents'] = ['onView:devtools.trace.main'] + ['onCapability:devtools.trace.' + a['id'] for a in actions]
schema = dict(type='object', properties=dict(text=dict(type='string', maxLength=8192)), required=['text'], additionalProperties=False)
manifest['contributes'] = dict(
    activities=[dict(id='devtools.trace', title='堆栈分析', order=853)],
    views=[dict(id='devtools.trace.main', activity='devtools.trace', location='main', title='堆栈分析', entry='views/main.html')],
    capabilities=[dict(id='devtools.trace.' + a['id'], title=a['title'], inputSchema=copy.deepcopy(schema)) for a in actions],
    piTools=[dict(name='devtools_trace_' + a['id'].replace('.', '_'), title=a['title'],
                  description='Analyze pasted trace locally, at most 8192 UTF-8 bytes. Paths remain text; no file/process/network access. Clues are not confirmed causes.',
                  capabilityId='devtools.trace.' + a['id'], inputSchema=copy.deepcopy(schema)) for a in actions])
catalog = dict(contract='zicode.devtools-plugin/1.0.0-rc.1', packageId=manifest['id'], packageVersion=manifest['version'],
               source=dict(revision='REPLACE_WITH_PROVIDER_SOURCE_REVISION', dirty=True),
               tools=[dict(sourceToolId=a['sourceToolId'], toolId='devtools.trace.' + a['id'], capabilityId='devtools.trace.' + a['id'],
                           version=versions[a['sourceToolId']], status='candidate', profile='compute.wasi.v1', title=a['title']) for a in actions])
request = json.loads((ROOT / 'contracts/studio-devtools/v1/request.schema.json').read_text(encoding='utf-8'))
assert request['properties']['pluginId'] == {'const': 'com.zicode.devtools.text'}
request['properties']['pluginId']['const'] = manifest['id']
request['$comment'] = 'Derived from frozen rc.1 pilot request schema; only pluginId.const is specialized to this manifest ID.'
samples = {'java-trace': 'java.lang.RuntimeException: outer\n\tat demo.Main.run(Main.java:9)\n\tSuppressed: java.io.IOException: close\nCaused by: java.lang.NullPointerException: main\n\tat demo.Db.read(Db.java:4)',
           'django-trace': 'Traceback (most recent call last):\n  File "app/views.py", line 15, in detail\ndjango.urls.exceptions.NoReverseMatch: missing URL'}
hints = {'java-trace': '粘贴标准 printStackTrace 文本；保留主异常和 Suppressed 分支，不还原省略帧，不将可见链终点当作已确认根因。',
         'django-trace': '粘贴文本 Traceback；保留异常因果链、截断标记和 Django 排查提示。不读取路径、不运行项目；不支持 HTML 调试页或 ExceptionGroup 树。'}
html = (ROOT / 'plugins/transforms/views/main.html').read_text(encoding='utf-8')
html = html.replace('ZiDevTools 文本转换', 'ZiDevTools 堆栈分析').replace('<h1>文本转换</h1>', '<h1>堆栈分析</h1>')
html = html.replace('编码、整理、检查和进制转换，在本地完成。', '整理 Java 和 Python 异常链，保留原始帧位置与排查线索。')
html = html.replace('23 个操作 · 开发候选', '2 个操作 · 开发中').replace('devtools.transforms.', 'devtools.trace.')
html = html.replace('作为下一步输入</button>', '替换当前输入</button>')
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
html, count = re.subn(r"if\(id==='hex.decode'\).*?change\(value\)", 'change(value)', html, count=1)
assert count == 1
outputs = {'plugin.json': manifest, 'catalog.json': catalog, 'request.schema.json': request}
outputs = {name: json.dumps(value, ensure_ascii=False, indent=2) + '\n' for name, value in outputs.items()}
outputs['views/main.html'] = html
for name, content in outputs.items():
    path = ROOT / 'plugins/trace' / name
    if args.check:
        assert path.read_text(encoding='utf-8') == content, 'Run scripts/sync_trace_plugin.py: ' + name
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding='utf-8')
print('Trace development registry: 2 complete operations; no delivery or Host acceptance claim')
