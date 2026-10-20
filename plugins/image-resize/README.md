# ZiDevTools 图片缩放插件候选 0.2.0

提供 `devtools.image.resize`：对用户明确授权的一张 PNG、JPEG 或 WebP 图片缩放，生成一项派生图片资源。独立版 image-workflow 0.12.0 仍包含更多功能；本候选不代表完整工具迁移。

## 使用参数

- 最大宽度：整数 1–4096，默认 1600。
- 输出格式：PNG / JPEG / WebP，默认 WebP。PNG 和当前 WebP 编码均为无损输出。
- JPEG 质量：整数 35–95，默认 82，仅 JPEG 生效。
- 工具目录增加 media 分类、中英文别名和明确的 PNG/JPEG/WebP 资源过滤。

Host 输入契约使用其支持的 integer、minimum、maximum。0.1.2 中间候选曾使用 pattern/not：虽然标准 JSON Schema 与 profile 静态预检通过，但 Host 安装声明校验不支持这两个关键字，故不可作为当前交付。0.2.0 改用整数参数并重建 WASI 适配器；底层兼容旧字符串请求仅用于迁移测试，新清单不接受字符串输入。

## 执行与边界

只使用 resource.read 和资源输出公开协议；不访问用户文件路径、环境变量或网络，不自动保存或覆盖源文件。输入每边不超过4096、总像素不超过16 MP，输入和输出分别不超过32 MiB。使用 resource.wasi.image.resize.v1@1.0.0-preview.1，预算为256 MiB guest memory、30B fuel、Host全生命周期10秒。并发、权限、取消和资源 lease 由 Host 控制。

本轮23项直接WASI执行覆盖九种格式组合、透明度、九项新旧参数输出一致、四项参数拒绝和一项坏图零输出。小样本不证明16MP性能、10秒生命周期、并发、取消或真实Studio安装。成功输出遵循输出schema；图片失败由公开图片profile的Host错误归一化路径处理。

## 构建与交付

使用 scripts/dev.ps1 -UseUserCache dt-image-plugin --offline 构建，再运行 scripts/verify_image_resize_revision.py 和 scripts/package_image_resize_revision.py。候选包含新WASM、输入/输出schema、来源摘要、运行证据及历史基线收据。历史基线证据不继承为新模块验收。

当前是未签名提供方目录；尚未安装、发布或获得Host端到端验收。单图缩放以外的批处理、裁剪、OCR、EXIF清理、文件保存等未包含在该capability中。
