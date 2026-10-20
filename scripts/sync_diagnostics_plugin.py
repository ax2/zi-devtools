"""Generate independent provider metadata and inline View samples from local sources."""
import argparse
import ast
import copy
import json
from pathlib import Path
import re
from tool_directory import validate_directory

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
core=(root/'crates/zi-diagnostics-core/src/lib.rs').read_text(encoding='utf-8')
registry=re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*scope:\s*"([^"]+)"',core)
tree=ast.parse((root/'scripts/diagnostics_vectors.py').read_text(encoding='utf-8'))
samples=next(ast.literal_eval(node.value) for node in tree.body if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='samples' for t in node.targets))
samples={row[0]:row for row in reversed(samples)}
assert set(samples)=={row[0] for row in registry}
tool_catalog=json.loads((root/'docs/tools.json').read_text(encoding='utf-8'))
tools={t['id']:t for t in tool_catalog['tools']}
secondary={'spring.config':'右侧配置（JSON / YAML）','django.migrations':'SQL 输出（可选，仅分析）','django.urls':'查询参数（可选 JSON）','django.openapi':'右侧 OpenAPI JSON'}
actions=[dict(id=action,title=tools[source]['name'],group='Java' if action.startswith(('java.','spring.')) else 'Celery' if action.startswith('celery.') else 'Django',scope=scope,text=samples[action][1],secondary=samples[action][2],secondaryLabel=secondary.get(action)) for action,source,scope in registry]
view=root/'plugins/diagnostics/views/main.html'
old=view.read_text(encoding='utf-8');new=re.sub(r'const actions = .*?; // diagnostic-actions-end',lambda _: 'const actions = '+json.dumps(actions,ensure_ascii=False,separators=(',',':')).replace('<','\\u003c')+'; // diagnostic-actions-end',old,flags=re.S)
assert old!=new or 'java.threads' in old
manifest=json.loads((root/'contracts/studio-devtools/v1/plugin.example.json').read_text(encoding='utf-8'))
manifest.update(schemaVersion=2,id='com.zicode.devtools.diagnostics',name='ZiDevTools Diagnostics',displayName='ZiDevTools 诊断报告',description='Imported Java/Django/Celery reports with tool/runtime provenance; diagnostics schema v2 remains a provider proposal.',version='0.1.4')
manifest['activationEvents']=['onView:devtools.diagnostics.main']+['onCapability:devtools.diagnostics.'+a['id'] for a in actions]
output_schema=json.loads((root/'contracts/diagnostics/v1/result.schema.json').read_text(encoding='utf-8'))
output_schema['$id']='https://devtools.zicode.com/contracts/diagnostics/v2/result.schema.json'
output_schema['title']='Diagnostics v2 compute result envelope; report text is validated separately'
secondary_actions={'spring.config','django.migrations','django.urls','django.openapi'}
operation_schemas={}
for action in actions:
    suffix=action['id']
    props={'text':{'type':'string','maxLength':8192,'description':'Runtime additionally limits UTF-8 bytes to 8192.'}}
    if suffix in secondary_actions:
        props['secondary']={'type':'string','maxLength':8192,'default':'','description':'Optional second report; runtime additionally limits UTF-8 bytes to 8192.'}
    operation_schemas[suffix]=dict(
        **{'$schema':'https://json-schema.org/draft/2020-12/schema',
           '$id':'https://devtools.zicode.com/contracts/diagnostics/v2/operations/'+suffix+'.input.schema.json'},
        title=action['title']+' input',type='object',additionalProperties=False,
        required=['text'],properties=props)
manifest['contributes']={'activities':[dict(id='devtools.diagnostics',title='诊断报告',order=851)],'views':[dict(id='devtools.diagnostics.main',activity='devtools.diagnostics',location='main',title='诊断报告',entry='views/main.html')],
    'capabilities':[dict(id='devtools.diagnostics.'+a['id'],title=a['title'],inputSchema=operation_schemas[a['id']],outputSchema=output_schema,maxResultBytes=49152) for a in actions],
    'piTools':[dict(name='devtools_diagnostics_'+a['id'].replace('.','_'),title=a['title'],description=a['scope'],capability='devtools.diagnostics.'+a['id'],inputSchema=operation_schemas[a['id']]) for a in actions]}
manifest['permissions']['optional'][0]['reason']='显式授权后允许当前场景的 AI 分析已提供的诊断报告'
manifest['requiredProfiles']={
    'compute.wasi.v1':'=1.0.0-rc.1',
    'output.schema.v1':'=1.0.0-rc.1',
    'tool.directory.v1':'=1.0.0-preview.1',
}
sources_by_action={action:source for action,source,_ in registry}
actions_by_id={action['id']:action for action in actions}
for capability in manifest['contributes']['capabilities']:
    suffix=capability['id'].removeprefix('devtools.diagnostics.')
    discovery=tools[sources_by_action[suffix]]['discovery']
    directory=dict(categoryId='code',aliases=discovery['aliases'],
                   description='仅分析用户提供的报告文本。'+actions_by_id[suffix]['scope'])
    validate_directory(directory)
    capability['directory']=directory
catalog=dict(contract='zicode.devtools-plugin/1.0.0-rc.1',packageId=manifest['id'],packageVersion='0.1.2',source=dict(revision='REPLACE_WITH_PROVIDER_SOURCE_REVISION',dirty=True),tools=[dict(sourceToolId=source,toolId='devtools.diagnostics.'+action,capabilityId='devtools.diagnostics.'+action,version=tools[source]['tool_version'],status='candidate',profile='compute.wasi.v1@=1.0.0-rc.1',diagnosticsProfile='diagnostics.compute.wasi.v1.proposal',title=tools[source]['name'],scope=scope) for action,source,scope in registry])
catalog['packageVersion']='0.1.4'
catalog['diagnosticsProtocol']='2.0.0-rc.1'
catalog['profile']='diagnostics.compute.wasi.v1.proposal'
catalog['hostComputeContract']='zicode.devtools-plugin/1.0.0-rc.1'
catalog['requiredProfiles']=manifest['requiredProfiles']
catalog['diagnosticsSchemaVersion']='2.0.0-rc.1'
catalog['diagnosticsReportSchema']='contracts/diagnostics/v2/report.schema.json'
for item in catalog['tools']:
    item['profile']='compute.wasi.v1@=1.0.0-rc.1'
    item['diagnosticsProfile']=catalog['profile']
    item['schemaVersion']=catalog['diagnosticsProtocol']
    item['inputSchema']='contracts/diagnostics/v2/operations/'+item['capabilityId'].removeprefix('devtools.diagnostics.')+'.input.schema.json'
    item['outputSchema']='contracts/diagnostics/v2/result.schema.json'
    item['maxResultBytes']=49152
    item['directoryProfile']='tool.directory.v1@=1.0.0-preview.1'
    item['fixtures']='fixtures/budget/'+item['capabilityId'].removeprefix('devtools.diagnostics.')
manifest['version']='0.1.4'
catalog_rs='// Generated from docs/tools.json by scripts/sync_diagnostics_plugin.py.\n'
catalog_rs+='pub const PROGRAM_VERSION: &str = '+json.dumps(tool_catalog['version'])+';\n'
catalog_rs+='pub const SOURCE_TOOL_VERSIONS: &[(&str, &str)] = &[\n'
catalog_rs+=''.join('    ('+json.dumps(source)+', '+json.dumps(tools[source]['tool_version'])+'),\n' for _,source,_ in registry)
catalog_rs+='];\n'
catalog_rs_path=root/'crates/zi-diagnostics-core/src/catalog.rs'
outputs={view:new,root/'plugins/diagnostics/plugin.json':json.dumps(manifest,ensure_ascii=False,indent=2)+'\n',root/'plugins/diagnostics/catalog.json':json.dumps(catalog,ensure_ascii=False,indent=2)+'\n',catalog_rs_path:catalog_rs}
outputs[root/'contracts/diagnostics/v2/result.schema.json']=json.dumps(output_schema,ensure_ascii=False,indent=2)+'\n'
request_schema={
    '$schema':'https://json-schema.org/draft/2020-12/schema',
    '$id':'https://devtools.zicode.com/contracts/diagnostics/v2/request.schema.json',
    'title':'Diagnostics v2 command request; Host compute envelope remains v1',
    'oneOf':[
        {'type':'object','additionalProperties':False,
         'required':['pluginId','sceneId','capabilityId','commandId','input'],
         'properties':{'pluginId':{'const':'com.zicode.devtools.diagnostics'},
                       'sceneId':{'type':'string','minLength':1,'maxLength':128,'description':'Runtime additionally limits UTF-8 bytes to 128.'},
                       'capabilityId':{'const':'devtools.diagnostics.'+a['id']},'commandId':{'type':'null'},
                       'input':{'$ref':'operations/'+a['id']+'.input.schema.json'}}}
        for a in actions]
}
report_schema={
    '$schema':'https://json-schema.org/draft/2020-12/schema',
    '$id':'https://devtools.zicode.com/contracts/diagnostics/v2/report.schema.json',
    'title':'Diagnostics v2 report with tool and portable runtime provenance',
    'type':'object','additionalProperties':False,
    'required':['diagnosticsProtocol','generatedBy','data'],
    'properties':{
        'diagnosticsProtocol':{'const':'2.0.0-rc.1'},
        'generatedBy':{'$ref':'generated-by.schema.json'},
        'data':{'type':'object','description':'Operation-specific analysis report; see the selected capability scope.'}
    }
}
generated_by_schema={
    '$schema':'https://json-schema.org/draft/2020-12/schema',
    '$id':'https://devtools.zicode.com/contracts/diagnostics/v2/generated-by.schema.json',
    'title':'Portable report provenance', 'type':'object','additionalProperties':False,
    'required':['toolId','toolVersion','runtimeFingerprint'],
    'properties':{
        'toolId':{'type':'string','minLength':1,'maxLength':128},
        'toolVersion':{'type':'string','pattern':'^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$'},
        'runtimeFingerprint':{'type':'object','additionalProperties':False,
            'required':['schema','appVersion','os','architecture','buildProfile','sha256'],
            'properties':{'schema':{'const':1},'appVersion':{'type':'string','maxLength':64},
                'os':{'type':'string','maxLength':32},'architecture':{'type':'string','maxLength':32},
                'buildProfile':{'enum':['debug','plugin']},'sha256':{'type':'string','pattern':'^[a-f0-9]{64}$'}}}
    }
}
outputs[root/'contracts/diagnostics/v2/request.schema.json']=json.dumps(request_schema,ensure_ascii=False,indent=2)+'\n'
outputs[root/'contracts/diagnostics/v2/report.schema.json']=json.dumps(report_schema,ensure_ascii=False,indent=2)+'\n'
outputs[root/'contracts/diagnostics/v2/generated-by.schema.json']=json.dumps(generated_by_schema,ensure_ascii=False,indent=2)+'\n'
for suffix,operation_schema in operation_schemas.items():
    outputs[root/'contracts/diagnostics/v2/operations'/f'{suffix}.input.schema.json']=json.dumps(operation_schema,ensure_ascii=False,indent=2)+'\n'
    operation_report=copy.deepcopy(report_schema)
    operation_report['properties']['generatedBy']['$ref']='../generated-by.schema.json'
    operation_report['$id']='https://devtools.zicode.com/contracts/diagnostics/v2/operations/'+suffix+'.report.schema.json'
    operation_report['title']=actions[[a['id'] for a in actions].index(suffix)]['title']+' report'
    outputs[root/'contracts/diagnostics/v2/operations'/f'{suffix}.report.schema.json']=json.dumps(operation_report,ensure_ascii=False,indent=2)+'\n'
for path,text in outputs.items():
    if args.check:assert path.read_text(encoding='utf-8')==text,str(path)
    elif not path.exists() or path.read_text(encoding='utf-8') != text:
        path.parent.mkdir(parents=True,exist_ok=True)
        path.write_text(text,encoding='utf-8')
print('Diagnostics provider metadata/View: 11 registered actions; profile proposal, not Host-accepted')
