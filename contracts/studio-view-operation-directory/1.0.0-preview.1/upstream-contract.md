# view.operation.directory.v1：View 操作目录预览

状态：Studio 已实现 `=1.0.0-preview.1` 的 Host 清单接收、目录查询、Studio View 定位消息和 SDK 回执。真实提供方包与原生完整流程尚未验收；提供方目前不要在发行包中声明支持此 profile，也不要把这些条目报为可调用能力。

## 包内清单

```json
{
  "schemaVersion": 2,
  "requiredProfiles": { "view.operation.directory.v1": "=1.0.0-preview.1" },
  "contributes": {
    "activities": [{ "id": "example.app", "title": "Service diagnostics" }],
    "views": [{ "id": "example.main", "activity": "example.app", "location": "main", "entry": "views/main.html" }],
    "viewOperations": [{
      "id": "example.health",
      "title": "Health",
      "viewId": "example.main",
      "sourceToolId": "example-service",
      "directory": {
        "categoryId": "system",
        "aliases": ["服务健康"],
        "description": "Open the health panel; the user confirms any network request separately."
      }
    }],
    "capabilities": [], "commands": [], "piTools": []
  }
}
```

`viewOperations` 必须有 1–128 项，所有 ID 为包内唯一的有界 ASCII 标识，并且不能与该包 capability ID 重复。每项只能有 `id`、`title`、`viewId`、`directory` 与可选的 `sourceToolId`。`viewId` 必须引用同包声明的主 View；标题最多 128 字节且不含控制字符。目录类别、别名、描述使用 [application.directory.v1](application-directory-v1.md) 的有界规则。输入 schema、输出 schema、运行时路径及任意其他字段都不允许出现在 View 操作上。

Host 在安装和载入时校验精确 profile 版本与结构；缺少 profile、空数组、重复 ID、未知主 View、无效分类、重复别名或执行字段均被拒绝。轻量插件列表只投影有界目录字段，仍单独保留 `contributes.capabilities`；View 操作不会成为 Pi 工具、Worker 调用或手动执行表单。`sourceToolId` 仅供来源对账，不能替代运行时权限身份。

## 后续目录与定位合同

Host 同一分页目录在请求 `includeViewOperations=true` 时返回 `kind=view-operation`、独立 `view.operation:` 访问 ID、`operationId` 和 `viewId`；不返回 `capabilityId`。搜索、类别、来源、收藏、最近的服务端过滤已可用，资源分享筛选始终排除。旧客户端不传该开关，保持原有可执行工具目录。当前工具中心及命令面板在有独立定位入口时请求这些条目，标记为“在应用中打开”，不显示执行表单。

用户选择后，Studio 再次核对 Host 目录修订、插件版本、场景、工作区、启用状态、View ID；若操作指向第二个主 View，先切换到该 View。定位消息仅含 `operationId` 和 `requestId`，不携带用户输入或结果。沙箱 View 调用 `window.zicode.navigation.onOpenViewOperation(({ operationId }) => ...)` 注册处理器，只返回 `opened`、`not_found` 或 `unsupported`。Studio 仅接受匹配当前帧与请求的 `openViewOperationResult`，确认后才记录最近使用；切换目标、停用和超时清除待定位请求。定位成功不代表网络、模型或文件写入已经执行。

当前 `tool.open.v1` 只接受真实 capability ID，不能被 View 操作复用。Studio 的独立定位协议已具备隔离一致性用例；真实提供方包仍需按该合同独立发布并提供包外收据，Studio 将进行新包及原生流程验收。Agent Assist 的模型请求与 Spring HTTP GET 仍使用各自逐次确认门禁。若提供方要让 Pi 调用某项操作，必须另外声明真实 capability 及其输入、输出、权限和执行合同。

## 验证边界

Host 正反例覆盖版本、缺失/重复操作、未知 View、未知执行字段、分类别名和来源 ID；隔离注册表用例核对清单安装、默认禁用及轻量列表中 View 操作与 capability 的分离。Studio 前端与沙箱专项覆盖目录分流、Host 修订复核、帧回执、第二主 View、停用与超时；真实提供方包和原生 View 操作定位尚未验收。九份现有 ZiDevTools 候选均未声明此 profile；本合同不改变其冻结字节或 DevTools 完整接受 **0/154**。
