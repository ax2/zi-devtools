use crate::{
    tools::{ToolKind, run_tool},
    workbench::{DataFormat, Dataset},
};
use anyhow::{Result, bail, ensure};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Table,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Table => "表格",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Material {
    Text(String),
    Table(Dataset),
}
impl Material {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Text(_) => Kind::Text,
            Self::Table(_) => Kind::Table,
        }
    }
    pub fn text(&self) -> Result<&str> {
        match self {
            Self::Text(text) => Ok(text),
            _ => bail!("此结果为表格，需先选择文本导出操作"),
        }
    }
    pub fn render(&self) -> Result<String> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Table(data) => Ok(serde_json::to_string(data)?),
        }
    }
    pub fn bytes(&self) -> Result<usize> {
        match self {
            Self::Text(text) => Ok(text.len()),
            Self::Table(data) => Ok(serde_json::to_vec(data)?.len()),
        }
    }
    pub fn summary(&self) -> String {
        match self {
            Self::Text(text) => format!("文本 · {}字节", text.len()),
            Self::Table(data) => format!(
                "表格 · {}行 / {}列；按值类型接力，列顺序保留",
                data.rows.len(),
                data.headers.len()
            ),
        }
    }
    pub fn preview(&self) -> Result<String> {
        let text = match self {
            Self::Text(text) => return Ok(prefix(text, 8192)),
            Self::Table(data) => {
                let mut lines = vec![
                    data.headers
                        .iter()
                        .take(6)
                        .map(|h| serde_json::to_string(&prefix(h, 256)).unwrap())
                        .collect::<Vec<_>>()
                        .join(" | "),
                ];
                lines.extend(data.rows.iter().take(3).map(|row| {
                    row.iter()
                        .take(6)
                        .map(|value| prefix(&value.to_string(), 1024))
                        .collect::<Vec<_>>()
                        .join(" | ")
                }));
                lines.join("\n")
            }
        };
        Ok(prefix(&text, 8192))
    }
}
fn prefix(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
#[derive(Clone, Copy)]
pub(super) enum Executor {
    Native(ToolKind, usize),
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
    pub note: &'static str,
    pub(super) executor: Executor,
}
macro_rules! native {
    ($id:literal,$label:literal,$kind:ident,$op:literal) => {
        Action {
            id: $id,
            version: 1,
            label: $label,
            input: Kind::Text,
            output: Kind::Text,
            note: "原生文本操作",
            executor: Executor::Native(ToolKind::$kind, $op),
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
            note: $note,
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
    pub(super) fn typed(&self) -> bool {
        !matches!(self.executor, Executor::Native(..))
    }
    pub(super) fn execute(&self, input: &Material, cancel: &AtomicBool) -> Result<Material> {
        ensure!(
            input.kind() == self.input,
            "{}需要{}输入，当前为{}",
            self.label,
            self.input.label(),
            input.kind().label()
        );
        match self.executor {
            Executor::Native(tool, operation) => Ok(Material::Text(run_tool(
                tool,
                operation,
                input.text()?,
                "",
                10,
            )?)),
            Executor::ParseCsv(delimiter) => Ok(Material::Table(Dataset::parse(
                input.text()?,
                DataFormat::Csv,
                delimiter,
            )?)),
            Executor::ParseJson => Ok(Material::Table(Dataset::parse(
                input.text()?,
                DataFormat::Json,
                b',',
            )?)),
            Executor::ParseSchema => {
                let data: Dataset = serde_json::from_str(input.text()?)?;
                Ok(Material::Table(Dataset::from_parts(
                    data.headers,
                    data.rows,
                )?))
            }
            _ => {
                let Material::Table(data) = input else {
                    bail!("需要表格材料")
                };
                match self.executor {
                    Executor::Trim => Ok(Material::Table(data.trim_text_columns(cancel)?)),
                    Executor::ExportCsv(delimiter) => Ok(Material::Text(data.export(
                        &(0..data.rows.len()).collect::<Vec<_>>(),
                        DataFormat::Csv,
                        delimiter,
                    )?)),
                    Executor::ExportJson => {
                        ensure!(
                            !data.rows.is_empty(),
                            "JSON空数组不编码列结构，请用列结构JSON导出"
                        );
                        Ok(Material::Text(data.export(
                            &(0..data.rows.len()).collect::<Vec<_>>(),
                            DataFormat::Json,
                            b',',
                        )?))
                    }
                    Executor::ExportSchema => Ok(Material::Text(serde_json::to_string(data)?)),
                    _ => unreachable!(),
                }
            }
        }
    }
}
