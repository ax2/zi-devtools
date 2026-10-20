# application.directory.v1 应用目录元数据预览

状态：Studio 接收侧预览 `=1.0.0-preview.1`。该 profile 只影响可信工具中心对**带主视图的插件应用**的发现，不执行插件、不增加侧栏图标或权限。提供方包需独立交付与核验；旧包继续归入“其他应用”。

## 清单约定

schemaVersion 2 清单必须在 `requiredProfiles` 中声明精确版本，方可给 `contributes.activities[]` 增加 `directory`：

```json
{
  "schemaVersion": 2,
  "requiredProfiles": { "application.directory.v1": "=1.0.0-preview.1" },
  "contributes": {
    "activities": [{
      "id": "example.service",
      "title": "Service diagnostics",
      "directory": {
        "categoryId": "system",
        "aliases": ["Spring health", "服务健康"],
        "description": "Read a confirmed service health endpoint"
      }
    }],
    "views": [{ "id": "example.main", "activity": "example.service", "location": "main", "entry": "views/main.html" }]
  }
}
```

`directory` 只允许声明在首个（主）activity 上；Studio 的应用入口以该 activity 代表整包。它与 `tool.directory.v1` 使用同一有界字段和 12 个任务分类：`text`、`data`、`web`、`code`、`files`、`media`、`system`、`ai`、`knowledge`、`automation`、`security`、`other`。`categoryId` 必填且必须属于该枚举；`aliases` 最多 16 项，每项非空、最多 128 UTF-8 字节，经 NFKC、小写、去首尾空白后不得重复；`description` 可选、非空且最多 512 UTF-8 字节；禁止控制字符与额外字段。Host 在安装和载入时校验。清单仍受 256 KiB 上限。

工具中心只读取已启用、当前客户端可用且有主视图插件的目录摘要；显示类别和用途，搜索别名，收藏/最近与 40 项分页沿用通用应用目录。打开才加载 View。声明不会把它变成可按需执行的工具；提供方要进入“单项工具”目录仍须声明可调用 capability 及相应输入、输出、权限与执行合同。

## ZiDevTools 提供方需求

Spring Actuator 0.1.1 candidate-02 目前没有此元数据；即使 Host 局部测试通过，它仍出现在“其他应用”，没有单项工具能力。提供方若交付下一版本，应在独立包中增加上述 activity 目录声明并更新 `requiredProfiles`、版本、包外收据，同时维持完整源工具范围差异表。不得修改已冻结 candidate-02；Studio 本轮也不修改 DevTools。

## 当前验证边界

Host 清单校验与前端目录投影有合成专项；真实提供方包、百项实际分类质量、原生截图和其他系统待验。类别/别名用于发现，不代表功能已经运行或已获权限。

Studio 自有合成 View 插件另经过隔离 Host 安装、轻量列表、启用/禁用和 View 源路由专项；列表元数据保留，禁用态路由拒绝。它验证接收侧生命周期，不构成 ZiDevTools 实包或原生界面验收。
