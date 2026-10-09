"""Independent fixed expectations for experimental named-field JSON behavior and hostile inputs."""
import json
import copy
version='1.1.0-proposal.2'
def raw(value):return json.dumps(value,ensure_ascii=False,separators=(',',':')).encode('utf-8')
def request(cap,input):return raw(dict(contractVersion=version,pluginId='com.zicode.devtools.compare',sceneId='coding',capabilityId='devtools.compare.'+cap,commandId=None,input=input))
def success(text):return raw(dict(contractVersion=version,ok=True,data=dict(text=text),error=None))
def failure(code,message):return raw(dict(contractVersion=version,ok=False,data=None,error=dict(code=code,message=message)))
def pretty(value):return json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)
def diff(equal,changes,unordered):return pretty(dict(equal=equal,changes=changes,arrayOrder='ignored (duplicates retained)' if unordered else 'by index'))
rows=[
 ('query',request('json.path',dict(text='{"项":[1,2]}',query='$.项[*]')),success(pretty([1,2]))),
 ('large-index',request('json.path',dict(text='[1]',query='$[4294967296]')),success('[]')),
 ('u64-max',request('json.path',dict(text='[1]',query='$[18446744073709551615]')),success('[]')),
 ('wide-integer',request('json.path',dict(text='[9007199254740993]',query='$[0]')),success(pretty([9007199254740993]))),
 ('ordered',request('json.diff.ordered',dict(left='[1,2]',right='[2,1]')),success(diff(False,[dict(path='/0',kind='changed',before=1,after=2),dict(path='/1',kind='changed',before=2,after=1)],False))),
 ('unordered',request('json.diff.unordered',dict(left='[1,2]',right='[2,1]')),success(diff(True,[],True))),
 ('duplicates',request('json.diff.unordered',dict(left='[1,1,2]',right='[1,2,2]')),success(diff(False,[dict(path='',kind='changed',before=[1,1,2],after=[1,2,2])],True))),
 ('missing-query',request('json.path',dict(text='{}')),failure('INVALID_INPUT','查询输入结构无效')),
 ('query-byte-over',request('json.path',dict(text='{}',query='中'*1366)),failure('INPUT_TOO_LARGE','查询字段超过 UTF-8 字节限制')),
 ('unsupported-draft-operation',request('regex.matches',dict(text='a',pattern='a')),failure('UNSUPPORTED_OPERATION','该实验适配未注册此能力')),
]
old=request('json.path',dict(text='{}',query='$')).replace(version.encode(),b'1.0.0-rc.1')
rows.append(('old-contract',old,failure('INVALID_INPUT','协议、插件或场景标识无效')))
rows.append(('invalid-utf8',b'\xff',failure('INVALID_ENCODING','请求不是有效 UTF-8')))
rows.append(('exact-text-bytes',request('json.path',dict(text='{}'+' '*8190,query='$')),success(pretty([{}]))))
rows.append(('text-byte-over',request('json.path',dict(text='{}'+' '*8191,query='$')),failure('INPUT_TOO_LARGE','查询字段超过 UTF-8 字节限制')))
rows.append(('exact-query-bytes',request('json.path',dict(text='{}',query='$.'+'a'*4094)),success('[]')))
rows.append(('exact-path-steps',request('json.path',dict(text='{}',query='$'+'.a'*128)),success('[]')))
rows.append(('path-steps-over',request('json.path',dict(text='{}',query='$'+'.a'*129)),failure('INVALID_INPUT','输入无法按该操作处理')))
rows.append(('request-byte-over',b' '*(49152+1),failure('INPUT_TOO_LARGE','请求超过 48 KiB')))
prefix='x'*1000
left=json.dumps({prefix:{f'key{i}':0 for i in range(100)}},separators=(',',':'))
right=json.dumps({prefix:{f'key{i}':1 for i in range(100)}},separators=(',',':'))
rows.append(('output-byte-over',request('json.diff.ordered',dict(left=left,right=right)),failure('INPUT_TOO_LARGE','结果超过 48 KiB')))


def add(name, body, expected): rows.append((name, body, expected))
seed=json.loads(request('json.path',dict(text='{}',query='$')))
structure_error=failure('INVALID_INPUT','请求结构无效')
identity_error=failure('INVALID_INPUT','协议、插件或场景标识无效')
path_input_error=failure('INVALID_INPUT','查询输入结构无效')
diff_input_error=failure('INVALID_INPUT','差异输入结构无效')
algorithm_error=failure('INVALID_INPUT','输入无法按该操作处理')
for key in seed:
 value=copy.deepcopy(seed);del value[key];add('outer-missing-'+key,raw(value),structure_error)
for key in ['contractVersion','pluginId','sceneId','capabilityId']:
 for tag,value in [('null',None),('number',4),('bool',True),('array',[])]:
  altered=copy.deepcopy(seed);altered[key]=value;add('outer-type-'+key+'-'+tag,raw(altered),structure_error)
for value in ['',True,4,[]]:
 altered=copy.deepcopy(seed);altered['commandId']=value;add('command-type-'+repr(value),raw(altered),structure_error)
altered=dict(seed,hidden=True);add('outer-extra',raw(altered),structure_error)
for key in ['sceneId','capabilityId','input']:
 body=raw(seed);literal=raw(seed[key]).decode('utf-8');needle='"'+key+'":'+literal
 add('outer-duplicate-'+key,body.replace(needle.encode(),(needle+','+needle).encode()),structure_error)
for name,value in [('empty',''),('ascii-exact','a'*128),('ascii-over','a'*129),('utf8-exact','中'*42+'aa'),('utf8-over','中'*43)]:
 altered=dict(seed,sceneId=value);add('scene-'+name,raw(altered),success(pretty([{}])) if value and len(value.encode())<=128 else identity_error)
for cap,fields,message in [('json.path',['text','query'],path_input_error),('json.diff.ordered',['left','right'],diff_input_error),('json.diff.unordered',['left','right'],diff_input_error)]:
 valid_fields={key:('{}' if key!='query' else '$') for key in fields}
 for key in fields:
  altered=valid_fields.copy();del altered[key];add(cap+'-missing-'+key,request(cap,altered),message)
  for tag,value in [('null',None),('number',1),('array',[]),('object',{}),('bool',False)]:
   altered=dict(valid_fields);altered[key]=value;add(cap+'-type-'+key+'-'+tag,request(cap,altered),message)
  body=request(cap,valid_fields);needle='"'+key+'":'+raw(valid_fields[key]).decode()
  add(cap+'-duplicate-'+key,body.replace(needle.encode(),(needle+','+needle).encode()),message)
 altered=dict(valid_fields,hidden='ignored?');add(cap+'-extra',request(cap,altered),message)
for path,values in [('$',[None]),('$["a.b"]',[3]),('$["a\\\"b"]',[7]),('$[*]',[1,2]),('$.项[*].名',['甲','乙']),('$[01]',[2]),('$.missing',[])]:
 text={'$':'null','$["a.b"]':'{"a.b":3}','$["a\\\"b"]':'{"a\\\"b":7}','$[*]':'{"b":2,"a":1}','$.项[*].名':'{"项":[{"名":"甲"},{"名":"乙"}]}','$[01]':'[1,2]','$.missing':'{}'}[path]
 add('path-semantic-'+str(len(rows)),request('json.path',dict(text=text,query=path)),success(pretty(values)))
for path in ['', 'a', '$..x', '$[-1]', '$[?(@.x)]', '$[1:2]', '$[', '$["x"', '$.a b', '$[18446744073709551616]']:
 add('path-invalid-'+str(len(rows)),request('json.path',dict(text='{}',query=path)),algorithm_error)
for text in ['', '{', '[NaN]', '{"x":Infinity}']:
 add('json-invalid-'+str(len(rows)),request('json.path',dict(text=text,query='$')),algorithm_error)
add('inner-duplicate-json-key',request('json.path',dict(text='{"x":1,"x":2}',query='$.x')),success(pretty([2])))
add('unicode-query-exact-bytes',request('json.path',dict(text='{}',query='$.'+'中'*1364+'aa')),success('[]'))
add('query-surrogate',raw(seed).replace(b'"query":"$"',b'"query":"\\ud800"'),path_input_error)
add('scene-surrogate',raw(seed).replace(b'"sceneId":"coding"',b'"sceneId":"\\ud800"'),structure_error)
exact=request('json.path',dict(text='{}',query='$'));add('request-exact-bytes',exact+b' '*(49152-len(exact)),success(pretty([{}])))
for cap,unordered in [('json.diff.ordered',False),('json.diff.unordered',True)]:
 add(cap+'-null-equal',request(cap,dict(left='null',right='null')),success(diff(True,[],unordered)))
 add(cap+'-root-type-change',request(cap,dict(left='null',right='{"x":1}')),success(diff(False,[dict(path='',kind='changed',before=None,after={'x':1})],unordered)))
 add(cap+'-pointer-add-remove',request(cap,dict(left='{"a/b~":1}',right='{"z":2}')),success(diff(False,[dict(path='/a~1b~0',kind='removed',before=1),dict(path='/z',kind='added',after=2)],unordered)))
 for side in ['left','right']:
  values=dict(left='null',right='null');values[side]='null'+' '*8188
  add(cap+'-'+side+'-exact-bytes',request(cap,values),success(diff(True,[],unordered)))
  values[side]+=' ';add(cap+'-'+side+'-byte-over',request(cap,values),failure('INPUT_TOO_LARGE','差异字段超过 UTF-8 字节限制'))
add('nested-unordered-equality',request('json.diff.unordered',dict(left='{"x":[{"a":[1,2]},{"b":1}]}',right='{"x":[{"b":1},{"a":[2,1]}]}')),success(diff(True,[],True)))
prefix='x'*6000
left=json.dumps({prefix:{f'key{i}':0 for i in range(100)}},separators=(',',':'))
right=json.dumps({prefix:{f'key{i}':1 for i in range(100)}},separators=(',',':'))
assert len(left.encode())<=8192 and len(right.encode())<=8192
add('diff-path-amplification',request('json.diff.ordered',dict(left=left,right=right)),failure('INPUT_TOO_LARGE','结果超过 48 KiB'))
assert len({name for name,_,_ in rows})==len(rows)
