# Zi DevTools v0.17.0

本轮完善现有工具体验，增加 Java 与 Django 离线诊断入口。

- 新增 Java 异常链：异常类型、cause/suppressed 关系、帧位置和省略帧数，正确区分主链与 suppressed 分支。
- 新增 Django / Python Traceback：异常链、文件/行号/函数、最后可见帧、截断标记及常见 Django 异常规则提示。
- 目录每页最多 18 项；分类、搜索与视图变化后回到首页；收藏、最近、常用和 Ctrl K 保持统一入口。
- 插件任务离开页面后继续收取完成结果并提示，保留各工具结果；模型必填检查、GET 参数简化、运行中锁定输入、结构化 HTTP 错误摘要与复制反馈。
- 插件清单目录与管理页避免每帧复制所有请求体；搜索匹配分数只计算一次。
- 本机入口发现增加 Java/javac/Maven/Gradle/django-admin；不执行环境命令、不导入项目。
- 新增 14 项 Java/Django 规划与专项路线文档；现有 36 项内置能力、80 项规划，另有 3 个示例插件包 / 6 项可选工具。

边界：两种诊断只解析支持的标准文本，不执行代码，不证明真实根因。Java 不还原省略帧；Python 不支持 HTML 调试页及 ExceptionGroup 树。模型请求仍不支持 SSE、手动取消或自主 Agent；MCP、RAG、进程插件尚在规划。

验证：56 项单元测试、4 项 Windows 生命周期集成测试、fmt、全目标全特性 Clippy；32 张真实应用 UI 截图和内置/插件 Ctrl K 键盘导航。安装器与发布结果见对应 Actions。

Windows x64：MSI、便携 EXE、SHA256SUMS.txt，无 ZIP。升级前请从托盘退出旧版本。MIT 开源。
