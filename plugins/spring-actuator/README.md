# Spring Actuator View candidate 0.1.3

## 0.1.3 页面操作定位

增加view.operation.directory.v1@=1.0.0-preview.1与页面操作目录，Host可将搜索条目导航到对应按钮、端点选择或摘要。定位不执行模型/HTTP调用、不新增权限或capability；请求忙碌时返回unsupported，未知操作返回not_found，卸载页面时注销处理器。模型尚未读取时定位生成区标题；Spring切换端点保留Origin和旧摘要，摘要仍显示原端点。

新构建入口为scripts/package_view_operation_revision.py；需要与当前View/manifest哈希一致的实际浏览器证据，独立生成0.1.3候选及包外收据。下方0.1.2与0.1.1记录仅为历史来源。真实Studio安装、跨工作区/版本切换及原生完整流程仍待验收，不能将提供方测试等同Host接受。

## 0.1.2 应用发现

主activity声明application.directory.v1@=1.0.0-preview.1，增加类别、中英文别名和用途。启用后由Host工具中心提供整应用搜索、收藏和最近使用。此修订保持View、权限、请求确认与其他执行内容的字节不变；不会将页面操作注册为Agent工具。

0.1.2历史构建入口为scripts/package_application_directory_revision.py，沿用已冻结0.1.1候选的运行内容，输出新的0.1.2候选及包外收据。旧打包脚本和下文运行验收记录对应历史版本。六项单操作定位仅在proposals/studio-view-operation-directory/2026-10-18中提出，当时manifest未声明该profile；真实Host安装和目录UI验收仍待完成。


这是 `spring-actuator` 独立工具的**部分 Studio 插件候选**，不是完整功能移植，也未获 Studio 接受。来源工具仍按独立版 `1.1.1` 跟踪；`docs/tools.json` 是来源工具目录。插件使用 Studio `service.http.get.v1@=1.0.0-preview.1` 草案，只实现固定 `health`、`info`、`metrics` 端点、用户逐次确认后的 GET 和 View 通用摘要。插件 manifest 的可选权限为 `service.http.read`。`health` 的 HTTP 503 JSON 保留为诊断数据。

View 只调用 `window.zicode.services.http.get({origin, endpoint}, {signal})`。CSP 禁止连接，代码没有 fetch/XHR/WebSocket 回退。View 不接触 Host token、grant、DNS 地址、任意路径、请求头、cookie、认证信息或响应原文；只显示端点、HTTP 状态、字节数、顶层字段数和 health 状态。Host 的完整响应通过可信 Studio 父页面/工作台结果接力处理。用户拒绝得到 `status: "declined"`，并且不发网络请求。点击取消后 View 等待 `service.http.get` 的 commit Promise 最终状态，不会提前把任务标成结束。

## 来源范围逐项映射

| 来源工具 `spring-actuator` 1.1.1 范围 | 插件 0.1.1 映射 | 状态 |
| --- | --- | --- |
| 显式 origin + `health` GET | `devtools.spring-actuator.health` | HTTP-only 候选 |
| `info` GET | `devtools.spring-actuator.info` | HTTP-only 候选 |
| `metrics` GET | `devtools.spring-actuator.metrics` | HTTP-only 候选 |
| health 503 JSON | health operation 的业务诊断数据 | 候选 View 已覆盖；不是网络失败 |
| 不跟随重定向、不请求 env/configprops | 固定 profile endpoint + Host 安全策略 | Host conformance 待验证 |
| 异步状态进入任务中心 | `devtools.spring-actuator.background-task` | 未交付；provider proposal 已按 Studio 任务生命周期草案列出路由，Studio 尚未实现/发布 |
| View 只展示通用摘要 | `devtools.spring-actuator.summary` | View 候选；完整响应由 Host/工作台接力 |
| 详细报告留在工作台 | Host `resultRef`/结果动作 | 由可信 Studio 父层负责；View 看不到 `resultRef`；真实接力待端到端验收 |
| 导出 JSON，含工具版本与便携环境指纹 | `devtools.spring-actuator.report-export` | 未交付；等 Studio 结果/资源接口、预览与用户确认 |

本插件不写 `super_agent_tasks`，不包装模型调用为任务，不创建本地路径或文件。`schemas/task-api-proposal/` 和 `schemas/result-resource-proposal/` 是提供方拟议合同，不在 manifest `requiredProfiles` 中，也没有已实现的 View API。完整来源工具接受状态保持 false。

## 测试

使用 Node.js 对真实 `views/main.html` 的脚本做无网络 DOM harness 测试：

```powershell
node plugins/spring-actuator/scripts/test-view.cjs
```

fixtures 覆盖 health 200/503、info/metrics 200、拒绝零请求、profile 不匹配、权限拒绝、DNS/TLS/HTTP/超时/取消错误显示、origin 路径/凭据/查询拒绝和取消等待 commit 最终态。测试仅模拟 Host API，不连服务、不验证真实 Studio Pi→Host→View、OS 根证书或公网 DNS。

另运行提供方合同检查：

```powershell
python -m pip install --target target\plugin-verification-deps -r plugins\spring-actuator\scripts\requirements-verification.txt
$env:PYTHONPATH = (Resolve-Path target\plugin-verification-deps).Path
$env:PYTHONDONTWRITEBYTECODE = "1"
python plugins/spring-actuator/scripts/verify-contract.py
```

验证器使用 `jsonschema==4.26.0` 的 Draft 2020-12 实现及 format checker 检查所有候选 schema 和正反向向量；另外执行捕获事实一致性、任务游标、终态顺序、确认 ID 绑定/过期/重放及副作用数量等语义向量。`__pycache__` 与 `.pyc` 不属于候选内容。正向结果只证明提供方 fixtures 通过，不能证明 Studio Host SDK conformance 或真实端到端接受。

## 包签名状态

包版本固定为 `0.1.1`；变更必须递增版本，禁止以相同插件 ID/版本重发不同字节。正式 `.zicode-plugin` 按 Studio ZIP 容器格式要求 `package.json` 的 Ed25519 签名载荷；提供方没有被授权的发布者私钥时，只可交付此 unsigned 源候选及文件摘要，不能伪造签名、伪造 key ID 或将 ZIP 改扩展名冒充可安装包。工作树 SHA 与来源目录 SHA 必须记录在包外收据中，`packageSha256` 在无签名正式包前保持 `null`。
