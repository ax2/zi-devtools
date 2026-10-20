# 截图工作台原生黑色遮挡（2026-10-17）

## 目标

让用户在截图工作台的透明 PNG 预览内直接遮住敏感区域，不必为了简单打码跳转到图片编辑器；遮挡结果必须能预览、撤销和重做，并且保留未保存的原始捕获。

## 实现

- 在 PNG 预览上拖动矩形，直接生成不透明黑色遮挡；坐标以输出图像比例记录，单次遮挡至少覆盖 2×2 输出像素，最多保留 100 项。
- 增加独立的遮挡撤销/重做。对图像修改时，始终从原始捕获、当前选区与羽化半径重新渲染，避免对已经处理的像素重复叠加。
- 选区历史快照同时保留对应遮挡；新捕获或重新选择会清空当前遮挡。
- 后台重算期间保留最后一次有效预览并禁用可能冲突的操作；重算失败时恢复到最后一次已渲染的遮挡状态。
- 文档与工具目录将“截图与透明套索”升至工具版本 0.9.0；桌面程序开发版本维持 `0.82.0-dev.105`。

## 验证

- 算法测试覆盖遮挡区域不透明、区域外像素不变、基于原图重算以及撤销/重做。
- egui 输入事件测试覆盖在 PNG 预览上拖动、遮挡目标像素和区域外像素保持原值。
- `cargo test --offline --locked --features ui-preview`：全量测试通过；另以 `cargo test --offline --locked --features ui-preview --lib image_tools::screenshot::tests:: -- --nocapture` 复核截图模块，17 项通过。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过；目录检查为 91 项已实现、63 项规划/开发中。
- `cargo build --offline --locked --release` 通过，用时约 11 分 03 秒。产物 `target/release/ZiDevTools.exe` 为 40,545,792 字节，SHA-256：`2B91E4FD23404309C639800ACD88B22B58BEAD1A91D8098F202C53F932D1A35D`。未更改桌面程序版本，未发布。
- 合成 UI 画面：[截图工作台浅色夹具](../../screenshots/screenshot-redaction-20261017/screenshot-lasso-light.png)。这是固定合成画面，不代表真实桌面截图；该夹具窗口的纵向视口没有完整展示预览区域，作为当前界面布局检查证据，不作为完整操作流程或真实桌面验收。

## 遗留与边界

- 截图页原生箭头、文字、像素化、模糊/渐变遮挡、置顶截图仍未实现；现有图片编辑器接力能力继续保留。
- 真实主屏截图、多屏、混合 DPI 和物理鼠标跨应用验收仍需专项验证。合成 UI 测试不代替这些验收。
- 输出仍由用户主动保存或复制；本次不自动创建临时文件，不改桌面程序版本，也不发布版本。
