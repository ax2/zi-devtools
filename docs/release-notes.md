# Zi DevTools v0.18.0

本轮完成原有 14 项 Java / Django 规划的首版，提供独立诊断工作台。

- Java：JDK/构建环境、线程转储、Maven/Gradle 依赖、GC 日志、JFR 事件、Spring 配置、Actuator 只读检查。
- Python：解释器/Django 环境、迁移计划、SQL/N+1、URL/reverse、DRF OpenAPI、部署检查、Celery 日志。
- 新增 Java/Python 两个目录分类；全部工具支持收藏、最近/常用、Ctrl K 与托盘直达。Ctrl Enter 执行，后台结果保留原工具。
- 会话独立草稿、UTF-8 文件导入、可复制摘要/JSON、新文件报告保存；输入与报告不自动持久化。
- 固定 java/python/jfr 命令、后台进程、输出/时间限制与 Windows Job 清理；核心仍为 Rust，不要求预装这些运行时。未开放任意进程插件。
- 迁移依赖不由显示顺序伪造；Object.wait 释放的监视器不计为持有者；SQL/N+1 与死锁推测和原文明确证据分开。
- 清单共 50 项内置已实现、66 项规划；14 项状态和具体支持范围已同步。3 个可选插件包 / 6 项工具另计。

详细格式、导出脚本、权限和限制：[Java / Django 使用说明](java-django-tools.md)。不支持所有日志方言、虚拟线程 JSON、完整 JFR 调用树或复杂 OpenAPI/reverse 语义；不执行迁移、修改环境、连接 broker 或重放任务。

验证：69 项单元测试、4 项 Windows 生命周期集成测试；1 项子进程夹具由边界测试单独启动。fmt、全目标全特性 Clippy、60 个真实 UI 场景和内置/插件/诊断快捷导航。临时 Java 21 与 Django 5.1.6 项目验证真实线程死锁、GC/JFR、迁移/SQL/URL/check 输出，另验证空 venv；Actuator 本地 HTTP 夹具验证 GET/Bearer/503。公开发版流水线另验证 MSI 安装/卸载。

Windows x64：MSI、便携 EXE、SHA256SUMS.txt，无 ZIP。升级前请从托盘退出旧版本。MIT 开源。
