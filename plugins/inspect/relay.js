// Inlined inside the self-contained View by sync_inspect_plugin.py.
let pendingRelay = null, relaySource = null;
const relayDefaults = {'yaml.to_json':'yaml.from_json','yaml.from_json':'yaml.to_json','unicode.nfc':'unicode.nfkc','unicode.nfkc':'unicode.nfc'};
function closeRelay(){pendingRelay=null;$('relay-preview').hidden=true}
function relayExcerpt(value){let result='',count=0;for(const char of value){if(count++===4096)return result+'\n（仅展示前 4096 个字符；目标草稿尚未改变）';result+=char}return result}
function renderRelay(state){
  const selected=$('relay-target').value;
  const changed=relaySource!==active;
  if(changed){closeRelay();relaySource=active}
  const choices=actions.filter(a=>a.id!==active);
  $('relay-target').replaceChildren();
  const groups=new Map();
  for(const action of choices){
    if(!groups.has(action.group)){const group=document.createElement('optgroup');group.label=action.group;groups.set(action.group,group);$('relay-target').append(group)}
    const option=document.createElement('option');option.value=action.id;option.textContent=action.title;groups.get(action.group).append(option);
  }
  const preferred=changed?relayDefaults[active]:selected;
  if(choices.some(a=>a.id===preferred))$('relay-target').value=preferred;
  const available=state.result!==null&&bytes(state.result)<=8192&&!busy;
  $('relay-send').disabled=!available;
  $('relay-target').disabled=busy;
  if(pendingRelay && (busy||state.revision!==pendingRelay.sourceRevision||state.result!==pendingRelay.text||states.get(pendingRelay.targetId).revision!==pendingRelay.targetRevision))closeRelay();
}
$('relay-target').addEventListener('change',()=>{closeRelay();render()});
$('relay-send').addEventListener('click',()=>{
  const source=states.get(active),targetId=$('relay-target').value,target=states.get(targetId);
  if(!target||targetId===active||source.result===null||bytes(source.result)>8192||busy)return;
  pendingRelay={sourceId:active,sourceRevision:source.revision,targetId,targetRevision:target.revision,text:source.result};
  const title=actions.find(a=>a.id===targetId).title;
  $('relay-title').textContent=`将结果发送到「${title}」的输入`;
  $('relay-summary').textContent=`${bytes(source.result)} 字节${source.old?' · 来源为旧结果':''}。${target.input.length?'确认后替换目标现有输入；目标旧结果会保留。':'目标输入为空。'}仅填入文本，不自动执行。`;
  $('relay-source').textContent=source.result===''?'（空文本）':source.result;
  $('relay-before').textContent=target.input===''?'（空输入）':relayExcerpt(target.input);
  $('relay-preview').hidden=false;$('relay-confirm').focus();
});
$('relay-cancel').addEventListener('click',()=>{closeRelay();status('已取消接力，所有输入和结果保持不变。');$('relay-send').focus()});
$('relay-confirm').addEventListener('click',()=>{
  const preview=pendingRelay;
  if(!preview||busy)return;
  const source=states.get(preview.sourceId),target=states.get(preview.targetId);
  if(active!==preview.sourceId||source.revision!==preview.sourceRevision||source.result!==preview.text||target.revision!==preview.targetRevision||bytes(preview.text)>8192){closeRelay();status('内容已变化，请重新预览接力。',true);return}
  closeRelay();$('search').value='';active=preview.targetId;options();change(preview.text);
  status('已填入目标输入，来源和目标旧结果已保留。请检查后主动处理文本。');$('input').focus();
});
document.addEventListener('keydown',event=>{if(event.key==='Escape'&&pendingRelay){event.preventDefault();$('relay-cancel').click()}});
