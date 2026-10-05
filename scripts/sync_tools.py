"""Generate tools.md; --check verifies source coverage and generated docs."""
import argparse, json, re, sys, tomllib
from pathlib import Path
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
data = json.loads((root / 'docs/tools.json').read_text(encoding='utf-8'))
assert data['version'] == tomllib.loads((root/'Cargo.toml').read_text(encoding='utf-8'))['package']['version'], 'Catalog version mismatch'
items = data['tools']
assert len({t['id'] for t in items}) == len(items), 'Duplicate tool IDs'
assert all(t['status'] in ('implemented','planned','in-progress') for t in items)
version_pattern = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)')
for tool in items:
    version = tool.get('tool_version', '')
    assert version_pattern.fullmatch(version) and all(int(p) <= 4294967295 for p in version.split('.')), f"Invalid tool version: {tool['id']}"
    history = tool.get('version_history', [])
    assert history and history[-1]['version'] == version, f"Missing current tool history: {tool['id']}"
    versions = []
    for event in history:
        assert version_pattern.fullmatch(event['version']), f"Invalid historical version: {tool['id']}"
        assert re.fullmatch(r'\d{4}-\d{2}-\d{2}', event['date']) and event['changes'].strip()
        versions.append(tuple(map(int, event['version'].split('.'))))
    assert all(a < b for a, b in zip(versions, versions[1:])), f"Tool versions must increase: {tool['id']}"
source = (root/'src/tools.rs').read_text(encoding='utf-8')
ids = set(re.findall(r'Self::\w+ => "([a-z0-9-]+)"', source[source.index('fn id'):source.index('pub fn description')]))
registry = (root/'src/app/registry.rs').read_text(encoding='utf-8')
ids |= set(re.findall(r'"([a-z0-9-]+)" => Page::', registry))
ids.add('services')
framework = (root/'src/framework/mod.rs').read_text(encoding='utf-8')
ids |= set(re.findall(r'Self::\w+ => "([a-z0-9-]+)"', framework[framework.index('pub fn id'):framework.index('pub fn label')]))
implemented_ids = {t['id'] for t in items if t['status']=='implemented'}
visible_ids = {t['id'] for t in items if t['status'] in ('implemented', 'in-progress')}
assert implemented_ids <= ids, 'Implemented catalog entries lack source UI IDs'
assert ids <= visible_ids, 'Source UI IDs are missing or still marked planned'
for tool in items:
    if tool['id'] in ids:
        discovery = tool.get('discovery')
        assert discovery, f"Missing discovery metadata: {tool['id']}"
        assert all(isinstance(discovery.get(k), str) and discovery[k].strip() for k in ('label', 'summary', 'category'))
        assert isinstance(discovery.get('keywords'), str)
        assert isinstance(discovery.get('aliases'), list) and all(isinstance(a, str) and a.strip() for a in discovery['aliases'])
    else:
        assert not tool.get('discovery'), f"Unrouted discovery entry: {tool['id']}"

lines = ['# 工具清单与路线图', '', f"更新：{data['updated']} · 程序目录版本：v{data['version']}", '', '本文件由 `docs/tools.json` 生成。规划表示方向，不代表已经可用，也不承诺发布日期。', '', '每项工具独立版本与程序版本分开，首次基线不追溯历史发布。规划项的 0.0.0 不表示可用。版本记录见 JSON 的 version_history，维护规则见 [独立工具版本](tool-versions.md)，新需求见 [桌面效率扩展](desktop-product-roadmap.md)。', '', '## 已实现', '', '| 工具 | 工具版本 | 分类 | 当前范围 |', '| --- | --- | --- | --- |']
for t in items:
    if t['status']=='implemented': lines.append(f"| {t['name']} | {t['tool_version']} | {t['category']} | {t['scope']} |")
lines += ['', '## 规划中', '', 'P1：优先推进共用基础与完整任务闭环；P2：扩展场景；P3：需专项依赖与权限设计。详见 [平台架构与阶段路线](platform-roadmap.md)。', '', '| 优先级 | 工具 | 工具版本 | 验收范围 | 状态 |', '| --- | --- | --- | --- | --- |']
for t in items:
    if t['status']!='implemented': lines.append(f"| {t['priority']} | {t['name']} | {t['tool_version']} | {t['scope']} | {'进行中' if t['status']=='in-progress' else '规划中'} |")
lines += ['', '## 持续同步规则', '', '1. 唯一清单源是 `docs/tools.json`。新想法先加入 planned；开始开发改为 in-progress。', '2. 实现、边界用例与可用入口均验证后，才改为 implemented；同时更新范围、版本和日期。', '3. 执行 `python scripts/sync_tools.py`，将 JSON、生成文档、README 和代码一起提交。', '4. CI 用 `python scripts/sync_tools.py --check` 验证文档未过期，且已实现 ID 与 Rust 入口一致。', '5. 官网读取同一仓库的原始清单并保留本地快照；加载失败会明确显示快照日期，发版时同步快照。', '6. 状态转换后更新阶段日志、测试证据与发布说明；未通过测试的功能不能列为已实现。', '']
output = '\n'.join(lines)
target = root/'docs/tools.md'
if args.check:
    if not target.exists() or target.read_text(encoding='utf-8') != output: sys.exit('Run python scripts/sync_tools.py and commit docs/tools.md')
else: target.write_text(output, encoding='utf-8')
print(f"Catalog OK: {len(implemented_ids)} implemented, {len(items)-len(implemented_ids)} planned/in progress")

readme = root/'README.md'
text = readme.read_text(encoding='utf-8')
small = sum(t['status']=='implemented' and t['category']=='小工具' for t in items)
workbenches = sum(t['status']=='implemented' and t['category']=='工作台' for t in items)
planned = sum(t['status']!='implemented' for t in items)
summary = f"<!-- tools-summary:start -->\n当前包含 **{small} 个小工具、{workbenches} 个开发工作台和本地服务管理**，另有 **{planned} 项规划 / 开发中能力**。完整列表见 [工具清单](docs/tools.md)。\n\n优先推进：" + '、'.join(t['name'] for t in [x for x in items if x['priority']=='P1' and x['status']!='implemented'][:8]) + '等；完整优先级见清单。规划不代表已实现。\n<!-- tools-summary:end -->'
updated, count = re.subn(r'<!-- tools-summary:start -->.*?<!-- tools-summary:end -->', lambda _: summary, text, flags=re.S)
assert count == 1, 'README needs one tools-summary block'
if args.check:
    assert text == updated, 'README tool summary is stale; regenerate it'
else:
    readme.write_text(updated, encoding='utf-8')

roadmap = root/'docs/platform-roadmap.md'
roadmap_text = roadmap.read_text(encoding='utf-8')
roadmap_summary = (f'<!-- catalog-summary:start -->\n更新：{data["updated"]} · 工具目录版本：v{data["version"]}。'
                   f'工具唯一状态源为 [tools.json](tools.json)，当前 {len(items)} 项目录条目：'
                   f'{len(implemented_ids)} 项已实现、{len(items)-len(implemented_ids)} 项规划或开发中；'
                   '另有可选示例插件包。连接器需要用户已有服务和模型，不能把插件数量当作已安装模型数量。\n'
                   '<!-- catalog-summary:end -->')
roadmap_updated, roadmap_count = re.subn(r'<!-- catalog-summary:start -->.*?<!-- catalog-summary:end -->',
                                          lambda _: roadmap_summary, roadmap_text, flags=re.S)
assert roadmap_count == 1, 'platform-roadmap needs one catalog-summary block'
if args.check:
    assert roadmap_text == roadmap_updated, 'Platform roadmap catalog summary is stale; regenerate it'
else:
    roadmap.write_text(roadmap_updated, encoding='utf-8')
