# 插件结果声明校验（预览）

2026-10-09：Host 新增通用 `output.schema.v1 / 1.0.0-rc.1` 预览。
这不是诊断专属 profile，也不表示既有 DevTools 候选已完成接入。
Windows 阶段运行版 `release/plugin-contracts-2026-10-09-02bd9552-dirty`
已包含门禁并完成自有夹具的原生 SDK→Host→WASI→显示及拒绝恢复验证。
旧 Host 不因此获得支持，其他系统及诊断正式 profile 仍待验收。

## 声明与协商

```json
{
  "schemaVersion": 2,
  "engines": {"zicode": ">=0.1.0 <1.0.0", "pluginApi": "^1.0.0"},
  "requiredProfiles": {"output.schema.v1": "=1.0.0-rc.1"},
  "contributes": {"capabilities": [{
    "id": "report",
    "inputSchema": {"type": "object"},
    "outputSchema": {"type": "object", "required": ["text"],
      "additionalProperties": false, "properties": {"text": {"type": "string"}}},
    "maxResultBytes": 49152
  }]}
}
```

这是字段片段，完整插件还需 id、版本、发布者及实际运行入口等。
`outputSchema` 和 `maxResultBytes` 可独立选用；声明 profile 必须至少有一个能力
实际声明其中一项。两个字段均要求 schemaVersion 2 和显式 profile 要求。
schemaVersion 1 携带新字段会拒绝。稳定版 `^1.0.0` 不匹配此预览版。
该结果契约不强制要求 WASI；实际执行入口仍由既有运行时规则决定。

## 支持的严格子集

- 类型：object、array、string、integer、number、boolean、null，以及这些类型的数组联合。
- const、非空 enum、properties、required、boolean additionalProperties、单 schema items。
- minLength/maxLength（Unicode 字符数量）、minimum/maximum、minItems/maxItems。
- oneOf：恰好一个分支匹配，1–8 个分支。
- 元数据：$schema、$id、title、description、default、examples；不执行这些注释。

未知关键词及错误字段类型在安装时拒绝。未支持 `$ref`、anyOf、allOf、not、
pattern、format、条件和 uniqueItems；不得依赖这些功能。
Schema 序列化上限 32 KiB，全部 JSON 节点最多 512，深度最多 16；
运行时共用校验访问预算 65536，避免 oneOf 放大处理成本。
数值上下限按现有 f64 实现比较，不承诺任意精度；const/enum 保留既有整数精度比较。

## 执行、缓存和错误

先执行既有身份、权限、输入和运行时检查，再校验 JSON 结果。
新结果通过全局 1 MiB 及能力字节预算、结果 schema 后才可缓存并返回给调用方。
`maxResultBytes` 是序列化 JSON 的 UTF-8 字节数，允许 1–1048576。
缓存命中同样按当前能力声明检查；结果不匹配或超限会清除该调用缓存。
恢复合法声明后，相同调用 ID 可重新执行。生产包声明变化必须提升插件版本；
测试中同版本重装仅用于证明当前缓存结果不会绕过门禁。

- 结构不匹配或校验预算耗尽：固定 `worker_protocol_error`。
- 超过能力字节预算：固定 `result_too_large`。
- 错误不包含结果值、提交字段名或底层堆栈；审计沿用摘要及固定错误码。

这是解析后 JSON 的结构检查，不解析诊断 `data.text` 内的报告字符串，
不恢复解析器已丢弃的重复键，不证明业务数据正确或所有工具已可用。
结果校验也不是操作授权：工具之间传递、保存数据和写入操作继续使用既有确认及权限机制。

## DevTools 接口要求与验收边界

提供方可在后续独立包中选择以上通用契约；不要修改既有候选的字段冒充正式交付。
需要完整版本化包、输入输出 schema 快照、正反例和全预算边界证据。
11 项诊断候选仍有两项 fuel 失败及 URL 余量问题，不能通过结果门禁代替算法修复。
专属诊断 profile、报告内部语义、二进制结果和图片算法仍按各自接口要求推进。

自动化结果和真实 Worker 记录见技术博客 `iterations/plugin-output-contract-2026-10-09`。
早期源码 Host 调用旧 EXE Worker 的验证与后续 Windows 原生验证分别记录。
新 EXE 为 e2e85f8fc120c083c833fe7fc3af895ae3e0ba90cd6cdcd5973dd4b31d3ba70d，
同目录前端资产补齐结果超限的公开错误码；新 Pi 目录完整流程仍待验收。
