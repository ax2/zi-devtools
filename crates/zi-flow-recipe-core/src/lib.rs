//! Definitions and action contracts shared by the native executor and read-only recipe preview.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const STEPS: usize = 16;
pub const RECIPE_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Text,
    Table,
    File,
    Image,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Table => "表格",
            Self::File => "文件引用",
            Self::Image => "图片",
        }
    }
}

#[derive(Clone, Copy)]
pub enum NativeTool {
    Json,
    Base64,
    Url,
    HtmlEscape,
    TextEscape,
    Hex,
    Sha256,
}

#[derive(Clone, Copy)]
pub enum Executor {
    Native(NativeTool, usize),
    FileSha256,
    ImageInfo,
    ParseCsv(u8),
    ParseJson,
    ParseSchema,
    ExportCsv(u8),
    ExportJson,
    ExportSchema,
    Trim,
}
pub struct Action {
    pub id: &'static str,
    pub version: u32,
    pub label: &'static str,
    pub input: Kind,
    pub output: Kind,
    pub side_outputs: &'static [OutputPort],
    pub note: &'static str,
    pub effect: Effect,
    pub executor: Executor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPort {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: Kind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    Transform,
    ReadSelectedFile,
    ProcessInMemoryImage,
}
impl Effect {
    pub fn label(self) -> &'static str {
        match self {
            Self::Transform => "纯转换",
            Self::ReadSelectedFile => "读取所选文件",
            Self::ProcessInMemoryImage => "处理本次内存图片",
        }
    }
    pub fn details(self) -> &'static str {
        match self {
            Self::Transform => "只处理本流程已选材料；不读写文件或联网",
            Self::ReadSelectedFile => "运行后读取已选普通文件并计算摘要；不修改文件内容",
            Self::ProcessInMemoryImage => "只处理本次接力的内存像素；不读取路径、保存或联网",
        }
    }
}
macro_rules! native {
    ($id:literal,$label:literal,$kind:ident,$op:literal) => {
        Action {
            id: $id,
            version: 1,
            label: $label,
            input: Kind::Text,
            output: Kind::Text,
            side_outputs: &[],
            note: "原生文本操作",
            effect: Effect::Transform,
            executor: Executor::Native(NativeTool::$kind, $op),
        }
    };
}
macro_rules! table {
    ($id:literal,$label:literal,$input:ident,$output:ident,$executor:expr,$note:literal) => {
        Action {
            id: $id,
            version: 1,
            label: $label,
            input: Kind::$input,
            output: Kind::$output,
            side_outputs: &[],
            note: $note,
            effect: Effect::Transform,
            executor: $executor,
        }
    };
}
pub static ACTIONS: &[Action] = &[
    native!("json.pretty", "JSON · 格式化", Json, 0),
    native!("json.minify", "JSON · 压缩", Json, 1),
    native!("base64.encode", "Base64 · 编码", Base64, 0),
    native!("base64.decode", "Base64 · 解码", Base64, 1),
    native!("url.encode", "URL · 编码", Url, 0),
    native!("url.decode", "URL · 解码", Url, 1),
    native!("html.escape", "HTML · 转义", HtmlEscape, 0),
    native!("html.unescape", "HTML · 还原", HtmlEscape, 1),
    native!("text.escape", "文本 · 转义", TextEscape, 0),
    native!("text.unescape", "文本 · 还原", TextEscape, 1),
    native!("hex.encode", "十六进制 · 编码", Hex, 0),
    native!("hex.decode", "十六进制 · 解码", Hex, 1),
    native!("sha256.digest", "SHA-256 · 摘要", Sha256, 0),
    Action {
        id: "file.sha256",
        version: 1,
        label: "文件 · SHA-256 报告",
        input: Kind::File,
        output: Kind::Text,
        side_outputs: &[OutputPort {
            id: "source-file",
            label: "原始文件引用",
            kind: Kind::File,
        }],
        note: "点击运行后分块读取已选普通文件并复核身份；报告不含路径或文件内容，原文件引用作为独立输出保留",
        effect: Effect::ReadSelectedFile,
        executor: Executor::FileSha256,
    },
    Action {
        id: "image.info",
        version: 1,
        label: "图片 · 信息报告",
        input: Kind::Image,
        output: Kind::Text,
        side_outputs: &[],
        note: "从受限的内存像素生成不含路径的 JSON 信息报告；不解码文件、不保存或联网",
        effect: Effect::ProcessInMemoryImage,
        executor: Executor::ImageInfo,
    },
    table!(
        "table.parse_csv",
        "CSV → 表格",
        Text,
        Table,
        Executor::ParseCsv(b','),
        "单元格保留为文本，不推断数字；空表保留表头"
    ),
    table!(
        "table.parse_tsv",
        "TSV → 表格",
        Text,
        Table,
        Executor::ParseCsv(b'\t'),
        "单元格保留为文本，不推断数字"
    ),
    table!(
        "table.parse_json",
        "JSON对象数组 → 表格",
        Text,
        Table,
        Executor::ParseJson,
        "保留JSON值类型；列按现有数据工作台规则排序；空数组无法推断列"
    ),
    table!(
        "table.parse_schema",
        "列结构JSON → 表格",
        Text,
        Table,
        Executor::ParseSchema,
        "读取headers/rows结构，保留列顺序、空表与值类型"
    ),
    table!(
        "table.trim",
        "表格 · 文本去空白",
        Table,
        Table,
        Executor::Trim,
        "复用数据工作台列转换；只处理文本值，其他类型保留"
    ),
    table!(
        "table.export_csv",
        "表格 → CSV原值文本",
        Table,
        Text,
        Executor::ExportCsv(b','),
        "数字/布尔变为文本，null变为空单元格；原值导出，不添加公式保护"
    ),
    table!(
        "table.export_tsv",
        "表格 → TSV原值文本",
        Table,
        Text,
        Executor::ExportCsv(b'\t'),
        "数字/布尔变为文本，null变为空单元格；原值导出"
    ),
    table!(
        "table.export_json",
        "表格 → JSON对象数组",
        Table,
        Text,
        Executor::ExportJson,
        "保留值类型，列顺序不编码；空表请用列结构JSON导出"
    ),
    table!(
        "table.export_schema",
        "表格 → 列结构JSON",
        Table,
        Text,
        Executor::ExportSchema,
        "保留列顺序、值类型与空表结构"
    ),
];

impl Action {
    pub fn typed(&self) -> bool {
        !matches!(self.executor, Executor::Native(..))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: String,
    pub version: u32,
}
impl Step {
    pub fn registered(&self) -> Result<&'static Action> {
        ACTIONS
            .iter()
            .find(|a| a.id == self.action && a.version == self.version)
            .ok_or_else(|| anyhow::anyhow!("未知或不兼容的操作：{} v{}", self.action, self.version))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u32,
    pub steps: Vec<Step>,
}
impl Default for Definition {
    fn default() -> Self {
        Self {
            version: 1,
            steps: vec![],
        }
    }
}
impl Definition {
    pub fn output_kind(&self) -> Result<Kind> {
        self.validate()?;
        match self.steps.last() {
            Some(step) => Ok(step.registered()?.output),
            None => self.start_kind(),
        }
    }
    pub fn start_kind(&self) -> Result<Kind> {
        if self.version >= 3
            && let Some(first) = self.steps.first()
        {
            let action = first.registered()?;
            if action.input == Kind::File {
                ensure!(
                    action.id == "file.sha256",
                    "文件引用只能从文件 SHA-256 操作开始"
                );
                return Ok(Kind::File);
            }
            if action.input == Kind::Image {
                ensure!(
                    self.version >= 4 && action.id == "image.info",
                    "图片材料只能从图片信息操作开始"
                );
                return Ok(Kind::Image);
            }
        }
        Ok(Kind::Text)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(matches!(self.version, 1..=4), "不支持此流程版本");
        ensure!(self.steps.len() <= STEPS, "文本流程最多16步");
        let mut kind = self.start_kind()?;
        for (index, step) in self.steps.iter().enumerate() {
            let action = step.registered()?;
            let mut ports = std::collections::BTreeSet::from(["result"]);
            for port in action.side_outputs {
                ensure!(
                    !port.id.trim().is_empty()
                        && !port.label.trim().is_empty()
                        && ports.insert(port.id),
                    "第{}步 {} 输出端口定义无效或重复",
                    index + 1,
                    action.label
                );
            }
            ensure!(
                self.version >= 2 || !action.typed(),
                "第{}步需要流程格式2或3",
                index + 1
            );
            ensure!(
                action.input == kind,
                "第{}步 {}需要{}，上一步输出为{}",
                index + 1,
                action.label,
                action.input.label(),
                kind.label()
            );
            kind = action.output;
        }
        Ok(())
    }
}

pub fn read_recipe(bytes: &[u8]) -> Result<Definition> {
    ensure!(bytes.len() <= RECIPE_LIMIT, "流程配方最多64 KiB");
    let definition: Definition = serde_json::from_slice(bytes)?;
    definition.validate()?;
    Ok(definition)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    recipe_version: u32,
    input: Kind,
    output: Kind,
    steps: Vec<PreviewStep>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewStep {
    action: &'static str,
    version: u32,
    label: &'static str,
    input: Kind,
    output: Kind,
    effect: Effect,
    side_outputs: &'static [OutputPort],
}

pub fn preview(bytes: &[u8]) -> Result<Preview> {
    let definition = read_recipe(bytes)?;
    let input = definition.start_kind()?;
    let mut output = input;
    let mut steps = Vec::with_capacity(definition.steps.len());
    for step in &definition.steps {
        let action = step.registered()?;
        output = action.output;
        steps.push(PreviewStep {
            action: action.id,
            version: action.version,
            label: action.label,
            input: action.input,
            output: action.output,
            effect: action.effect,
            side_outputs: action.side_outputs,
        });
    }
    Ok(Preview {
        recipe_version: definition.version,
        input,
        output,
        steps,
    })
}
