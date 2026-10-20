# URL 文本资源插件 0.1.1：接入声明与失败输出（2026-10-18）

## 可复现的接入阻塞

只读核对 Studio 的2026-10-17接收记录与当前profile校验源码，旧URL资源候选声明了directory和outputSchema，却没有要求对应的tool.directory.v1、output.schema.v1，因此在安装预检阶段被拒绝。旧输出schema还只允许ok=true，但WASM实际会返回ok=false及稳定错误码；这会把合法业务失败当作输出契约错误。

本轮新候选补齐两个profile，保留resource.input.types.v1与resource.wasi.input.draft2。目录增加中英文别名和资源接收用途描述。输出schema是两个互斥的封闭分支：原成功分支保持完全一致，失败分支只允许ok=false和包含code/message的error；7个错误码固定枚举，message长度有界，拒绝成功/失败字段混合、未知错误码、缺少字段与额外字段。

## 实现与验证

复用0.1.0 candidate-02的225023字节WASM，SHA-256为ff233c72dcec2199aee7b559d6be9c1405e9d92ed72baaeb20b9468344dfe607，没有重新编译Rust。依赖许可证和NOTICE也从已经逐文件核对的基线继承；新收据记录runtimeReusedFrom并复制原收据。运行路由、输入schema、MIME、权限和结果大小上限不得在复用运行模块时改变，打包器逐项核对。

增强实际Wasmtime运行器，启用10M fuel、5秒执行期限和64MiB guest memory；明确不包含模块编译时间。13项执行全部通过：

- 原有3项成功、4项失败。
- 8192字节全部百分号膨胀及UTF-8精确字节边界。
- 错误插件标识、无资源描述、Host报告大小不符、Host读取失败。
- 错误请求/描述/MIME/空资源/超限输入在读取资源之前拒绝，计数断言通过。
- 5个成功结果和覆盖全部7种错误码的8个失败结果均通过新输出schema。
- 本次冻结包重放观察到最高1950473 fuel、最长3.854ms执行；不是生产Host性能承诺。

候选内附可独立运行的verification/verify_url_resource_plugin.py，自动使用包内WASM、schema和fixtures，不依赖当前源码目录；输出证明须位于冻结包外。已在冻结新包上完成13项重放。schema正反向与提供方冲突回归加入脚本测试，全量60项通过。Studio公开只读package、catalog verifier均verified=true，profile preflight issues=[]。

## 同名能力与工具互通

Studio接口请求要求保留已有精确能力ID devtools.transforms.url.encode。因此资源版与transforms普通文字版共用该ID，不能把两个提供方的声明计成两个不同能力，也不能假定它们可以同时启用。

迁移目录新增明确的资源输入操作、包版本、路径和收据。目录生成器新增capabilityProviderConflicts，列出同一能力的两个包及“隔离验收选择一个提供方；共同启用未验证”的边界，并有回归测试。没有擅自重命名Studio已请求的能力，没有改动Studio源码或发送其他会话消息。

## 交付

- 插件：release/plugin-candidates/url-text-resource-0.1.1-candidate-01，95文件、687946字节；同名-verification.json为外部收据。
- 包索引SHA-256：7a2fc4de330654ce4a0a910ea262373f6bffad0719e8574da673edecf60cf353。
- 目录：release/plugin-candidates/devtools-catalog-2026-10-18-9c17b7b91e84f1c7，14文件、2407456字节。
- 目录索引SHA-256：5a56fd019df64abb8d5a43df7d197ee30ea7e98e51e7d9a5419657684ff57a32。
- 当前9个候选包，54项capability声明对应53个不同能力ID，另有6项View操作；覆盖33项来源工具，121项尚未有交付操作映射。
- 源tools.json内容与独立URL工具1.0.2不变；迁移表SHA-256 b66b06b81792c60abe077cef4240ad8e038f2764df73990993d9545c80337179。
- 所有候选仍unsigned，Host接受仍为0/154。没有发布、安装、启用JSON Object、修改应用版本或中断已有Cargo进程。

## 复核命令与边界

设置PYTHONPATH使用现有Wasmtime与jsonschema依赖后，执行 scripts/verify_url_resource_plugin.py --wasm release/plugin-candidates/url-text-resource-0.1.0-candidate-02/runtime/tools.wasm --output target/plugin-url-resource-011/verification.json，再执行 scripts/package_url_resource_plugin.py 与 scripts/verify_url_resource_package.py。冻结包复核使用包内 verification/verify_url_resource_plugin.py --output <包外证明路径>。

本迭代归档verification.json、portable-replay.json、package/catalog/profile三份只读校验结果。源码与目录同步、diff检查通过。未进行真实OCR→授权→URL接收的Studio UI链路、原生授权对话框、撤权/取消/过期授权、跨平台和签名安装验收。图片缩放候选只做了定位，没有在本轮修改或宣称完成。
