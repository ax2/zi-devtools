"""Generate independent provider metadata and inline View samples from local sources."""
import argparse
import ast
import json
from pathlib import Path
import re

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
core=(root/'crates/zi-diagnostics-core/src/lib.rs').read_text(encoding='utf-8')
registry=re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*scope:\s*"([^"]+)"',core)
tree=ast.parse((root/'scripts/diagnostics_vectors.py').read_text(encoding='utf-8'))
samples=next(ast.literal_eval(node.value) for node in tree.body if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='samples' for t in node.targets))
samples={row[0]:row for row in samples}
assert set(samples)=={row[0] for row in registry}
tools={t['id']:t for t in json.loads((root/'docs/tools.json').read_text(encoding='utf-8'))['tools']}
secondary={'spring.config':'右侧配置（JSON / YAML）','django.migrations':'SQL 输出（可选，仅分析）','django.urls':'查询参数（可选 JSON）','django.openapi':'右侧 OpenAPI JSON'}
actions=[dict(id=action,title=tools[source]['name'],group='Java' if action.startswith(('java.','spring.')) else 'Celery' if action.startswith('celery.') else 'Django',scope=scope,text=samples[action][1],secondary=samples[action][2],secondaryLabel=secondary.get(action)) for action,source,scope in registry]
view=root/'plugins/diagnostics/views/main.html'
old=view.read_text(encoding='utf-8');new=re.sub(r'const actions = .*?; // diagnostic-actions-end',lambda _: 'const actions = '+json.dumps(actions,ensure_ascii=False,separators=(',',':')).replace('<','\\u003c')+'; // diagnostic-actions-end',old,flags=re.S)
assert old!=new or 'java.threads' in old
schema=json.loads((root/'contracts/diagnostics/v1/input.schema.json').read_text(encoding='utf-8'))
manifest=json.loads((root/'contracts/studio-devtools/v1/plugin.example.json').read_text(encoding='utf-8'))
manifest.update(id='com.zicode.devtools.diagnostics',name='ZiDevTools Diagnostics',displayName='ZiDevTools 诊断报告',description='Imported Java/Django/Celery reports; provider diagnostics schema proposal, not Host-accepted.',version='0.1.1')
manifest['activationEvents']=['onView:devtools.diagnostics.main']+['onCapability:devtools.diagnostics.'+a['id'] for a in actions]
manifest['contributes']={'activities':[dict(id='devtools.diagnostics',title='诊断报告',order=851)],'views':[dict(id='devtools.diagnostics.main',activity='devtools.diagnostics',location='main',title='诊断报告',entry='views/main.html')],
    'capabilities':[dict(id='devtools.diagnostics.'+a['id'],title=a['title'],inputSchema=schema) for a in actions],
    'piTools':[dict(name='devtools_diagnostics_'+a['id'].replace('.','_'),title=a['title'],description=a['scope'],capability='devtools.diagnostics.'+a['id'],inputSchema=schema) for a in actions]}
manifest['permissions']['optional'][0]['reason']='显式授权后允许当前场景的 AI 分析已提供的诊断报告'
catalog=dict(contract='zicode.devtools-plugin/1.0.0-rc.1',packageId=manifest['id'],packageVersion='0.1.1',source=dict(revision='REPLACE_WITH_PROVIDER_SOURCE_REVISION',dirty=True),tools=[dict(sourceToolId=source,toolId='devtools.diagnostics.'+action,capabilityId='devtools.diagnostics.'+action,version=tools[source]['tool_version'],status='candidate',profile='diagnostics.compute.wasi.v1.proposal',title=tools[source]['name'],scope=scope) for action,source,scope in registry])
outputs={view:new,root/'plugins/diagnostics/plugin.json':json.dumps(manifest,ensure_ascii=False,indent=2)+'\n',root/'plugins/diagnostics/catalog.json':json.dumps(catalog,ensure_ascii=False,indent=2)+'\n'}
for path,text in outputs.items():
    if args.check:assert path.read_text(encoding='utf-8')==text,str(path)
    else:path.write_text(text,encoding='utf-8')
print('Diagnostics provider metadata/View: 11 registered actions; profile proposal, not Host-accepted')
