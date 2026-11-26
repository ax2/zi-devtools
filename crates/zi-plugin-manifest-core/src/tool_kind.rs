#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToolKind {
    #[default]
    Json,
    Base64,
    Url,
    Sha256,
    Timestamp,
    Uuid,
    Jwt,
    Regex,
    RegexReplace,
    Number,
    Qr,
    Color,
    TextStats,
    HtmlEscape,
    TextEscape,
    CaseConvert,
    Yaml,
    Hex,
    Lines,
    UrlInspect,
    Cidr,
    JsonPath,
    JsonDiff,
    DataQuality,
    Cron,
    Random,
    Unicode,
    JavaTrace,
    DjangoTrace,
    SqlFormat,
    JsonSchema,
}

impl ToolKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Base64 => "base64",
            Self::Url => "url",
            Self::Sha256 => "sha256",
            Self::Timestamp => "timestamp",
            Self::Uuid => "uuid",
            Self::Jwt => "jwt",
            Self::Regex => "regex",
            Self::RegexReplace => "regex-replace",
            Self::Number => "number",
            Self::Qr => "qr",
            Self::Color => "color",
            Self::TextStats => "text-stats",
            Self::HtmlEscape => "html-escape",
            Self::TextEscape => "text-escape",
            Self::CaseConvert => "case-convert",
            Self::Yaml => "yaml",
            Self::Hex => "hex",
            Self::Lines => "lines",
            Self::UrlInspect => "url-inspect",
            Self::Cidr => "cidr",
            Self::JsonPath => "json-path",
            Self::JsonDiff => "json-diff",
            Self::DataQuality => "data-schema",
            Self::Cron => "cron",
            Self::Random => "random",
            Self::Unicode => "unicode",
            Self::JavaTrace => "java-trace",
            Self::DjangoTrace => "django-trace",
            Self::SqlFormat => "sql-format",
            Self::JsonSchema => "json-schema",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Json => "格式化、压缩、校验与结构化对象摘要 JSON",
            Self::Base64 => "UTF-8 文本与 Base64 编解码",
            Self::Url => "URL 参数百分号编码与还原",
            Self::Sha256 => "计算文本的 SHA-256 摘要",
            Self::Timestamp => "Unix 时间戳与本地时间转换",
            Self::Uuid => "生成随机 UUID v4",
            Self::Jwt => "查看 Header 与 Payload；不验证签名",
            Self::Regex => "查找匹配与捕获组",
            Self::RegexReplace => "预览正则批量替换，支持捕获组引用",
            Self::Number => "二、八、十、十六进制互转",
            Self::Qr => "从文本生成二维码并保存 PNG",
            Self::Color => "HEX、RGB、HSL 颜色表示",
            Self::TextStats => "字符、字节、行数与词段统计",
            Self::HtmlEscape => "HTML 特殊字符转义与还原",
            Self::TextEscape => "JSON 字符串转义与还原",
            Self::CaseConvert => "camel、Pascal、snake 与 kebab 命名",
            Self::Yaml => "YAML 与 JSON 双向转换",
            Self::Hex => "UTF-8 文本与十六进制字节互转",
            Self::Lines => "稳定去重、排序、去空行与空白处理",
            Self::UrlInspect => "解析协议、主机、路径和重复查询参数",
            Self::Cidr => "IPv4 掩码、网络地址与可用范围",
            Self::JsonPath => "提取 JSONPath 子集匹配结果；支持字段、索引与通配符",
            Self::JsonDiff => "比较两个 JSON；对象键顺序无关，可选择数组策略",
            Self::DataQuality => "检查 JSON / CSV / TSV 的空值、重复行与类型分布",
            Self::Cron => "五字段 Cron；预览未来 5 年内最多 10 次运行时间",
            Self::Random => "操作系统安全随机字符串，或可复现的测试数据",
            Self::Unicode => "检查码点、UTF-8、常见不可见字符与 Unicode 规范化",
            Self::JavaTrace => "提取主异常链、Suppressed 分支和堆栈帧",
            Self::DjangoTrace => "整理 Python 异常链与 Django 常见异常排查提示",
            Self::SqlFormat => "按所选方言格式化 SQL 子句，不连接数据库或校验语义",
            Self::JsonSchema => {
                "按本机 JSON Schema 校验 JSON、定位字段错误；仅读取用户选择文件所在目录的本地引用"
            }
        }
    }
    pub fn sample(self) -> &'static str {
        match self {
            Self::JavaTrace => {
                "java.lang.RuntimeException: Request failed\n\tat demo.Service.run(Service.java:42)\nCaused by: java.lang.IllegalArgumentException: missing id\n\tat demo.Repository.get(Repository.java:18)\n\t... 1 more"
            }
            Self::DjangoTrace => {
                "Traceback (most recent call last):\n  File \"app/views.py\", line 12, in detail\n    reverse(\"missing\")\ndjango.urls.exceptions.NoReverseMatch: route not found"
            }
            Self::SqlFormat => {
                "select u.id,u.name,count(o.id) as order_count from users u left join orders o on o.user_id=u.id where u.active=true group by u.id,u.name order by order_count desc"
            }
            Self::JsonSchema => r#"{"name":"Zi DevTools","version":1,"local":true}"#,
            Self::Json => r#"{"name":"Zi DevTools","local":true,"tools":["JSON","CSV"]}"#,
            Self::Yaml => "name: Zi DevTools\nlocal: true\ntools: [JSON, CSV]",
            Self::Timestamp => "1780000000",
            Self::Number => "255",
            Self::Color => "#538DE8",
            Self::Cidr => "192.168.10.42/24",
            Self::JsonPath => r#"{"items":[{"name":"Rust"},{"name":"Python"}]}"#,
            Self::JsonDiff => r#"{"name":"Zi","tools":["JSON","CSV"],"version":14}"#,
            Self::DataQuality => {
                r#"[{"id":1,"name":"Zi"},{"id":2,"name":" "},{"id":1,"name":"Zi"},{"id":"3"}]"#
            }
            Self::Cron => "*/15 9-18 * * 1-5",
            Self::Random => r#"{"length":24,"count":5,"seed":42}"#,
            Self::Unicode => "Hello\u{200b} Ａ e\u{301}",

            Self::Qr | Self::UrlInspect => "https://example.com/search?q=hello&tag=rust&tag=tools",
            Self::Jwt => "eyJhbGciOiJub25lIn0.eyJzdWIiOiJkZW1vIn0.",
            Self::Regex => "build-2026 release-14 test-42",
            Self::RegexReplace => "build-2026 release-14 test-42",
            Self::Lines => "Rust\nPython\nRust\n\nTypeScript",
            Self::CaseConvert => "hello local developer tools",
            Self::HtmlEscape => "<p title=\"Zi\">Hello & welcome</p>",
            Self::TextEscape => "Hello\nZi DevTools\t本地工具",
            _ => "Hello, 字与码!",
        }
    }
    pub fn actions(self) -> &'static [&'static str] {
        match self {
            Self::Json => &["格式化", "压缩", "校验", "对象摘要"],
            Self::Base64 | Self::Url => &["编码", "解码"],
            Self::Sha256 => &["计算哈希"],
            Self::Timestamp => &["时间戳 → 本地时间", "本地时间 → 秒"],
            Self::Uuid => &["生成 UUID v4"],
            Self::Jwt => &["解析内容"],
            Self::Regex => &["查找匹配"],
            Self::RegexReplace => &["预览全部替换"],
            Self::Number => &["转换"],
            Self::Qr => &[],
            Self::Color => &["转换颜色"],
            Self::TextStats => &["统计文本"],
            Self::HtmlEscape => &["转义 HTML", "还原 HTML"],
            Self::TextEscape => &["转义文本", "还原文本"],
            Self::CaseConvert => &["camelCase", "PascalCase", "snake_case", "kebab-case"],
            Self::Yaml => &["YAML → JSON", "JSON → YAML"],
            Self::Hex => &["文本 → Hex", "Hex → 文本"],
            Self::Lines => &["去重", "升序", "降序", "去空行", "去首尾空白"],
            Self::UrlInspect => &["解析 URL"],
            Self::Cidr => &["计算子网"],
            Self::JsonPath => &["提取匹配结果"],
            Self::JsonDiff => &["按数组索引对比", "忽略数组顺序"],
            Self::DataQuality => &["检查 JSON", "检查 CSV", "检查 TSV"],
            Self::Cron => &["预览运行时间"],
            Self::Random => &["安全随机字符串", "种子测试数据（非安全）"],
            Self::Unicode => &["检查字符", "转换 NFC", "转换 NFKC"],
            Self::JavaTrace => &["整理异常链"],
            Self::DjangoTrace => &["整理 Traceback"],
            Self::SqlFormat => &["格式化"],
            Self::JsonSchema => &["校验 JSON"],
        }
    }
    pub const ALL: [Self; 31] = [
        Self::Json,
        Self::Base64,
        Self::Url,
        Self::Sha256,
        Self::Timestamp,
        Self::Uuid,
        Self::Jwt,
        Self::Regex,
        Self::RegexReplace,
        Self::Number,
        Self::Qr,
        Self::Color,
        Self::TextStats,
        Self::HtmlEscape,
        Self::TextEscape,
        Self::CaseConvert,
        Self::Yaml,
        Self::Hex,
        Self::Lines,
        Self::UrlInspect,
        Self::Cidr,
        Self::JsonPath,
        Self::JsonDiff,
        Self::DataQuality,
        Self::Cron,
        Self::Random,
        Self::Unicode,
        Self::JavaTrace,
        Self::DjangoTrace,
        Self::SqlFormat,
        Self::JsonSchema,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Base64 => "Base64",
            Self::Url => "URL 编码",
            Self::Sha256 => "SHA-256",
            Self::Timestamp => "时间戳",
            Self::Uuid => "UUID",
            Self::Jwt => "JWT 查看",
            Self::Regex => "正则测试",
            Self::RegexReplace => "正则替换",
            Self::Number => "进制转换",
            Self::Qr => "二维码",
            Self::Color => "颜色转换",
            Self::TextStats => "文本统计",
            Self::HtmlEscape => "HTML 转义",
            Self::TextEscape => "文本转义",
            Self::CaseConvert => "大小写转换",
            Self::Yaml => "YAML / JSON",
            Self::Hex => "UTF-8 / Hex",
            Self::Lines => "文本行整理",
            Self::UrlInspect => "URL 拆解",
            Self::Cidr => "IPv4 子网",
            Self::JsonPath => "JSON 路径查询",
            Self::JsonDiff => "JSON 结构对比",
            Self::DataQuality => "数据质量检查",
            Self::Cron => "Cron 预览",
            Self::Random => "随机数据生成",
            Self::Unicode => "Unicode 检查",
            Self::JavaTrace => "Java 异常链",
            Self::DjangoTrace => "Django / Python 堆栈",
            Self::SqlFormat => "SQL 格式化",
            Self::JsonSchema => "JSON Schema 校验",
        }
    }

    pub fn secondary_sample(self) -> &'static str {
        match self {
            Self::Regex => r"([a-z]+)-(\d+)",
            Self::RegexReplace => r"([a-z]+)-(\d+)",
            Self::JsonPath => "$.items[*].name",
            Self::JsonDiff => r#"{"name":"Zi","tools":["CSV","JSON"],"version":15}"#,
            Self::Cron => "2026-09-26T09:00:00+08:00",
            Self::SqlFormat => "postgresql",
            Self::JsonSchema => {
                r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","required":["name","version"],"properties":{"name":{"type":"string","minLength":1},"version":{"type":"integer","minimum":1},"local":{"type":"boolean"}},"additionalProperties":false}"#
            }
            _ => "",
        }
    }
    pub fn replacement_sample(self) -> &'static str {
        match self {
            Self::RegexReplace => "$1=$2",
            _ => "",
        }
    }
    pub fn option_label(self) -> Option<&'static str> {
        match self {
            Self::Regex => Some("正则表达式"),
            Self::RegexReplace => Some("正则表达式"),
            Self::JsonPath => Some("JSON 路径 · $、.字段、[索引]、[\"字段\"]、[*]"),
            Self::JsonDiff => Some("右侧 JSON · 数组无序模式保留重复次数；数组差异作为整体报告"),
            Self::Cron => Some("参考时间（RFC3339，含时区偏移）· 留空使用当前 UTC"),
            Self::JsonSchema => Some("JSON Schema · 粘贴文档，或明确选择本机文件"),
            _ => None,
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::JsonPath => "不支持递归、过滤器或切片。未匹配返回 []，所有匹配以 JSON 数组输出。",
            Self::JsonDiff => {
                "差异路径采用 JSON Pointer。无序模式忽略各层数组顺序、保留重复次数，数组差异整体报告。"
            }
            Self::DataQuality => {
                "JSON 需为对象数组；CSV/TSV 首行为列名。缺失与 null 合计，空白字符串单列；CSV 不推断类型。"
            }
            Self::Cron => {
                "分 时 日 月 周；支持数字、*、列表、范围与步长。日和周均不以 * 开头时按 OR；否则按 AND。固定偏移，不模拟夏令时。"
            }
            Self::Random => {
                "输入 JSON 参数 length（1–256）、count（1–100）、alphabet（可选）、seed（仅测试模式）。安全模式使用系统随机源；不保证各类字符必选。"
            }
            Self::Unicode => {
                "按码点而非字形检查，最多 10000 码点。NFC 合并等价字符；NFKC 会折叠兼容字符，可能改变语义。"
            }
            Self::JavaTrace => {
                "粘贴标准 printStackTrace 文本，去除时间戳前缀。省略帧不还原；Suppressed 不替代主异常链。最后可见异常不等于已确认根因。"
            }
            Self::DjangoTrace => {
                "支持标准文本 Traceback 与异常链；不读取源码或运行 Python。暂不支持 HTML 调试页、ExceptionGroup 树。提示仅供排查。"
            }
            Self::SqlFormat => {
                "仅整理常见 SQL 关键字、子句和缩进，不解析或验证语义；字符串、引用标识符和注释内容保持原样。方言用于识别各自的引用语法。"
            }
            Self::JsonSchema => {
                "草稿自动识别；根Schema≤64KiB、实例≤1MiB；本机引用最多32个/累计1MiB。选择文件后只读其目录内引用；拒绝网络、越界路径和$id语义歧义。"
            }
            Self::RegexReplace => {
                "预览匹配与替换结果，不会修改输入。支持 $1、${name} 和 $$；最多1000条匹配、输入与输出各2MiB、替换模板64KiB。"
            }
            _ => "",
        }
    }
    pub fn is_encoding(self) -> bool {
        matches!(
            self,
            Self::Json
                | Self::Base64
                | Self::Url
                | Self::Sha256
                | Self::Jwt
                | Self::HtmlEscape
                | Self::TextEscape
                | Self::Yaml
                | Self::JsonPath
                | Self::JsonDiff
                | Self::JsonSchema
                | Self::Unicode
                | Self::Hex
        )
    }
}
