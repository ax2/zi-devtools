# 剪贴片段组合

在 Studio 工具中心搜索「剪贴片段组合」，选择并确认一份本机 UTF-8 JSON。可以手动准备字符串数组，也可以将 ZiDevTools「超级剪贴板」已选条目复制为 JSON 数组或有序 JSON 记录，再另存为文件。

```json
["第一段\n续行", "第二段", "第三段"]
```

选择段落 TXT、编号 TXT、单列 CSV 或 Markdown 清单，执行后可预览全部正文并确认保存。组合顺序与文件顺序相同，重复正文也会保留。有序记录仅组合 `text`，其 `order` 必须从 1 开始并跟随数组顺序。Markdown 保留原生多行缩进规则，CSV 继续在可能被表格解释为公式的单元格前添加单引号。

每次一份最多 32 KiB 的 JSON，1–64 段非空文本，完整输出最多 48 KiB。仅申请 `resource.read`。文件输入与手动保存均使用 Studio 的确认流程。独立版系统剪贴板采集、实时历史、富文本与模板仍保留原有范围。

## 构建与验证

复用 Cargo 缓存运行 `cargo build --offline --locked -p zi-clipboard-compose-wasi`，再以 `--profile plugin --target wasm32-wasip1` 构建同一模块。用 `scripts/verify_clipboard_compose_plugin.py` 经真实 Windows Worker 和原生适配器比对四种输出的所有字节与边界，`scripts/package_clipboard_compose_plugin.py` 生成通过的候选和包外收据。
