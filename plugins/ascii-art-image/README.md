# 图片转 ASCII 字符画

使用本次明确授权的单张 PNG、JPEG 或 WebP 原图。候选接受最多 10 MiB、每边 2 万像素和总计 2000 万像素，输出宽度 8–120 字符，高度最多 120 行。透明部分按共享算法使用白色背景。输出是一份保留空格和换行的纯文本资源。

插件只申请 `resource.read`。它使用 `resource.wasi.image.ascii.v1` 的预览协商、`resource.wasi.input.draft2` 及 `resource.wasi.output.draft1`。另存由 Studio 的资源界面处理。

## 本地构建和验收

仅此插件的 WASI 构建使用图片 Worker 已有的 256 MiB 模块上限，保留 2 MiB 栈。构建进程传入以下 flags，并复用已有 Cargo 目标。

```powershell
$previousRustFlags = $env:RUSTFLAGS
try {
    $env:RUSTFLAGS = '-C link-arg=--max-memory=268435456 -C link-arg=-zstack-size=2097152'
    cargo build --offline --locked --profile plugin --target wasm32-wasip1 -p zi-ascii-art-image-wasi
} finally {
    $env:RUSTFLAGS = $previousRustFlags
}
```

原生对照用 `cargo build --offline --locked --profile plugin -p zi-ascii-art-image-wasi`。以 `scripts/verify_ascii_art_image_plugin.py` 核对合成三格式原图、透明度、宽度及资源边界。该验证可以使用已有 Studio 图片 Worker 预算进行提供方对照，安装和 Host 协商由 Studio 单独验证。
