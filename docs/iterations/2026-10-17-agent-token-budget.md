# Agent 规划与执行累计 token 预算

日期：2026-10-17\
工具：Agent 任务工作台 `1.9.0`\
程序版本：`0.82.0-dev.105`（未更改、未发布）

## 背景

Agent 工作台已有 12,000 tokens 总预算，但旧逻辑从批准执行阶段才开始累计；计划生成的 prompt 与回复不计入，而且缺失用量字段会被当作 0。这使预算显示和 MCP 调用前的限制不完整。

## 修改

- 计划生成响应要求同时提供非负整数 `prompt_eval_count` 与 `eval_count`，并将两者相加记为规划用量；规划阶段若已达到总预算则不生成可执行计划。
- 执行阶段从规划用量继续累计，每次模型响应在处理工具调用或最终回答前核验用量。累计超过 12,000 tokens 时停止；触发上限的响应若包含工具调用，不会执行对应 MCP 操作。
- 任一阶段缺少有效用量字段时立即报错，避免把未知用量错误地显示为 0 或继续 MCP 调用。
- 审批卡片显示规划阶段已报告用量和包含规划/执行的总上限；最终结果显示累计值。
- Ollama 聊天响应字段依据 [官方 API 文档](https://docs.ollama.com/api/chat)；该报告是 token 计数，不是货币成本估算或前置保证模型计算不会超过上限。

## 验证

- `cargo test --offline --locked --test agent_local -- --nocapture`：8 项通过、1 项忽略。新增预算边界用例确认规划用量计入总额、越限时不执行 MCP 工具；缺失统计字段时在计划阶段停止。
- 常规本机模拟服务测试确认规划、工具调用、最终回答的 prompt 与生成计数累计为 `90`，并随 Outcome 导出。
- 全量 `cargo test --offline --locked --features ui-preview --quiet`：984 通过、27 忽略、0 失败；所有可运行集成测试通过。`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、工具目录及 Studio 插件映射同步检查通过。
- `cargo build --release --offline --locked`：通过，Release 构建耗时 6 分 23 秒。
- 实际 eframe 深色审批界面截图已检查（1600×1475）；新增用量文字在批准按钮上方清晰显示，无需滚动即可读取。截图暂存在 `target/agent-token-budget-preview-bc9f6cd07c0c476492ffb3a9754b6efc/agent-plan-dark.png`。

## 限制

Ollama 在响应完成后返回用量，因此预算是累计响应核验：计划或某次模型响应本身可能已经消耗较多计算，系统只能在收到该响应后识别并阻止后续步骤。Ollama 报告 token 量不等于任何货币成本；未来接入计费提供方时仍需独立费率、预估和账单对账模型。Agent 写操作、断点恢复与持久执行仍未实现。
