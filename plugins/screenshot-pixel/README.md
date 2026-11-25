# PNG 截图取色

在 Studio 工具中心搜索「PNG 截图取色」，选择并确认一份本机静态 PNG。输入要读取的 `x` 和 `y` 整数坐标。左上角是 `(0, 0)`，坐标对应文件里的原始像素。缩放界面不影响读取位置。

执行后可复制原图尺寸、对应 RGBA 通道、`#RRGGBB` 和 `#RRGGBBAA`。结果保留像素原始 RGB 通道和独立 Alpha，即使像素完全透明也保留其颜色。灰度、索引颜色、RGB 和 RGBA PNG 按其原始 8 位通道语义读取。16 位通道和动画 PNG 会明确提示不支持。

支持每张最多 256 KiB、每边 512 像素、合计 16384 个像素的局部截图。仅申请 `resource.read`。本工具读取已存在的文件。屏幕采集、放大镜画面、裁剪与系统剪贴板属于其他功能。

## 构建与验证

使用 `cargo build --offline --locked -p zi-screenshot-pixel-wasi` 构建原生对照，再以 `--profile plugin --target wasm32-wasip1` 构建独立模块。用 `scripts/verify_screenshot_pixel_plugin.py` 通过真实 Windows Worker 对照边界、像素及原件，`scripts/package_screenshot_pixel_plugin.py` 负责冻结通过的候选和包外收据。
