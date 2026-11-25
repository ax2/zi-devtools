use crate::{
    tools::{ToolKind, run_tool},
    workbench::{DataFormat, Dataset, DesktopDataset},
};
use anyhow::{Context, Result, bail, ensure};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(crate) use zi_flow_recipe_core::Kind;
use zi_flow_recipe_core::{Action, Executor, NativeTool, OutputPort};

#[derive(Clone, Debug)]
pub enum Material {
    Text(String),
    Table(Dataset),
    File(crate::material_files::FileMaterial),
    Image(crate::image_tools::ImageMaterial),
}
impl PartialEq for Material {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Table(a), Self::Table(b)) => a == b,
            (Self::File(a), Self::File(b)) => a == b,
            (Self::Image(a), Self::Image(b)) => {
                Arc::ptr_eq(&a.image, &b.image) && a.origins == b.origins && a.kind == b.kind
            }
            _ => false,
        }
    }
}
impl Material {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Text(_) => Kind::Text,
            Self::Table(_) => Kind::Table,
            Self::File(_) => Kind::File,
            Self::Image(_) => Kind::Image,
        }
    }
    pub fn text(&self) -> Result<&str> {
        match self {
            Self::Text(text) => Ok(text),
            Self::File(_) => bail!("此结果为文件引用；请选择明确的文件接收工具"),
            Self::Image(_) => bail!("此结果为图片；请先运行图片信息操作"),
            _ => bail!("此结果为表格，需先选择文本导出操作"),
        }
    }
    pub fn render(&self) -> Result<String> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Table(data) => Ok(serde_json::to_string(data)?),
            Self::File(_) => bail!("文件引用不序列化为路径或文件内容"),
            Self::Image(_) => bail!("图片像素不序列化为文本；请先运行图片信息操作"),
        }
    }
    pub fn bytes(&self) -> Result<usize> {
        match self {
            Self::Text(text) => Ok(text.len()),
            Self::Table(data) => Ok(serde_json::to_vec(data)?.len()),
            Self::File(data) => usize::try_from(data.bytes()).context("文件大小无法表示"),
            Self::Image(data) => Ok(data.image.as_bytes().len()),
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
            Self::File(data) => format!(
                "文件引用 · {} · {}字节；内容尚未读取",
                data.display_name(),
                data.bytes()
            ),
            Self::Image(data) => format!(
                "图片 · {} × {} · {}像素 · 来源链{}步；运行 image.info 后生成报告",
                data.image.width(),
                data.image.height(),
                u64::from(data.image.width()) * u64::from(data.image.height()),
                data.origins.len()
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
            Self::File(_) => return Ok(self.summary() + "；预览不读取文件内容"),
            Self::Image(_) => return Ok(self.summary() + "；不展开像素数据"),
        };
        Ok(prefix(&text, 8192))
    }
}
pub(super) fn prefix(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
pub(super) trait ExecuteAction {
    fn execute(&self, input: &Material, cancel: &AtomicBool) -> Result<Material>;
    fn side_material(&self, port: &OutputPort, input: &Material) -> Result<Material>;
}

impl From<NativeTool> for ToolKind {
    fn from(tool: NativeTool) -> Self {
        match tool {
            NativeTool::Json => Self::Json,
            NativeTool::Base64 => Self::Base64,
            NativeTool::Url => Self::Url,
            NativeTool::HtmlEscape => Self::HtmlEscape,
            NativeTool::TextEscape => Self::TextEscape,
            NativeTool::Hex => Self::Hex,
            NativeTool::Sha256 => Self::Sha256,
        }
    }
}

impl ExecuteAction for Action {
    fn execute(&self, input: &Material, cancel: &AtomicBool) -> Result<Material> {
        ensure!(
            input.kind() == self.input,
            "{}需要{}输入，当前为{}",
            self.label,
            self.input.label(),
            input.kind().label()
        );
        match self.executor {
            Executor::FileSha256 => {
                let Material::File(file) = input else {
                    bail!("需要文件引用")
                };
                let digest = file.sha256(usize::MAX, cancel)?;
                let generated_by = crate::tool_provenance::current_for_tool("text-flow")?;
                Ok(Material::Text(serde_json::to_string_pretty(
                    &serde_json::json!({
                        "schemaVersion": 1,
                        "material": "file-sha256-report",
                        "generatedBy": generated_by,
                        "action": {"id": self.id, "version": self.version},
                        "fileName": digest.source.display_name(),
                        "bytes": digest.source.bytes(),
                        "sha256": digest.sha256
                    }),
                )?))
            }
            Executor::ImageInfo => {
                let Material::Image(image) = input else {
                    bail!("需要图片材料")
                };
                Ok(Material::Text(crate::image_tools::text_flow_image_report(
                    image, cancel,
                )?))
            }
            Executor::Native(tool, operation) => Ok(Material::Text(run_tool(
                tool.into(),
                operation,
                input.text()?,
                "",
                None,
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
    fn side_material(&self, port: &OutputPort, input: &Material) -> Result<Material> {
        match (self.executor, port.id, input) {
            (Executor::FileSha256, "source-file", Material::File(file)) => {
                Ok(Material::File(file.clone()))
            }
            _ => anyhow::bail!("{} 未实现输出端口 {}", self.id, port.id),
        }
    }
}
