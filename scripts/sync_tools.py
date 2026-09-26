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
source = (root/'src/tools.rs').read_text(encoding='utf-8')
ids = set(re.findall(r'Self::\w+ => "([a-z0-9-]+)"', source[source.index('fn id'):source.index('pub fn description')]))
app = (root/'src/app.rs').read_text(encoding='utf-8')
ids |= set(re.findall(r'id: "([a-z0-9-]+)"', app[app.index('fn catalog()'):app.index('pub struct DevToolsApp')]))
ids.add('services')
assert ids == {t['id'] for t in items if t['status']=='implemented'}, 'Source/catalog implemented IDs differ'
lines = ['# 工具清单与路线图', '', f"更新：{data['updated']} · 已实现版本：v{data['version']}", '', '本文件由 `docs/tools.json` 生成。规划表示方向，不代表已经可用，也不承诺发布日期。', '', '## 已实现', '', '| 工具 | 分类 | 当前范围 |', '| --- | --- | --- |']
for t in items:
    if t['status']=='implemented': lines.append(f"| {t['name']} | {t['category']} | {t['scope']} |")
lines += ['', '## 规划中', '', 'P1：优先补齐本地数据处理闭环；P2：扩展开发场景；P3：依赖专项设计与风险评估。', '', '| 优先级 | 工具 | 验收范围 | 状态 |', '| --- | --- | --- | --- |']
for t in items:
    if t['status']!='implemented': lines.append(f"| {t['priority']} | {t['name']} | {t['scope']} | {'进行中' if t['status']=='in-progress' else '规划中'} |")
lines += ['', '## 持续同步规则', '', '1. 唯一清单源是 `docs/tools.json`。新想法先加入 planned；开始开发改为 in-progress。', '2. 实现、边界用例与可用入口均验证后，才改为 implemented；同时更新范围、版本和日期。', '3. 执行 `python scripts/sync_tools.py`，将 JSON、生成文档、README 和代码一起提交。', '4. CI 用 `python scripts/sync_tools.py --check` 验证文档未过期，且已实现 ID 与 Rust 入口一致。', '5. 官网读取同一仓库的原始清单并保留本地快照；加载失败会明确显示快照日期，发版时同步快照。', '6. 状态转换后更新阶段日志、测试证据与发布说明；未通过测试的功能不能列为已实现。', '']
output = '\n'.join(lines)
target = root/'docs/tools.md'
if args.check:
    if not target.exists() or target.read_text(encoding='utf-8') != output: sys.exit('Run python scripts/sync_tools.py and commit docs/tools.md')
else: target.write_text(output, encoding='utf-8')
print(f"Catalog OK: {len(ids)} implemented, {len(items)-len(ids)} planned/in progress")
