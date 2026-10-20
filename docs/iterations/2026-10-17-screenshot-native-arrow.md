# 截图工作台原生箭头标注（2026-10-17）

## 目标

让教程作者直接在截图预览上标出要讲解的位置，不必先把截图接力到图片编辑器；保留透明选区和原始捕获，箭头方向由用户拖动决定。

## 实现

- 新增“箭头”工具：从箭尾拖向箭尖，结果绘制为红色主体、白色描边；至少拖动6个输出像素才会提交。
- 箭头绘制以输出图像坐标计算，预览阶段实时显示方向；实际箭头通过后台渲染写入预览。
- 绘制只更改可见像素颜色，不改写每像素alpha；透明轮廓外保持透明，羽化边缘保留原有透明度。
- 箭头与黑色遮挡、像素化共用最多100项的撤销/重做历史，并随选区快照恢复；重算从原始捕获、当前轮廓和羽化状态重新生成。
- 撤销/重做控件改名为通用“标注”；工具版本升至0.11.0，程序开发版本仍为`0.82.0-dev.105`。

## 验证

- 箭头像素测试覆盖方向保留、短箭头拒绝、RGB写入与原alpha不变；异步历史测试覆盖箭头渲染、撤销、重做及精确像素结果。截图模块定向测试21项通过。
- `cargo test --offline --locked --features ui-preview`：1002项通过、27项忽略（需要特定Windows桌面/编码器、模型、网络或专用CI环境）；已执行测试全部通过。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过；目录检查为91项已实现、63项规划/开发中。
- `cargo build --offline --locked --release`：成功，用时8分16秒；生成`target/release/ZiDevTools.exe`（40,564,736字节）。SHA-256：`FEDFA95663F8AE089C5EEF5DEEB018AD8EDFB1C778807E8314EDF5BF7934B976`。
- 1600×1125深浅主题合成截图：[浅色](../../screenshots/screenshot-redaction-20261017/screenshot-lasso-light.png)、[深色](../../screenshots/screenshot-redaction-20261017/screenshot-lasso-dark.png)。用于检查箭头控件和结果预览，不代表真实桌面或物理交互验收；预览区域因固定视口只显示一部分。

## 遗留与边界

- 原生文字、模糊/渐变遮挡和置顶截图仍未实现。
- 真实多屏、混合DPI和物理鼠标跨应用验收仍待进行；合成输入测试不替代该验收。
- 截图仍需用户主动保存或复制；本次不自动创建临时文件、不更改程序版本、不发布。
