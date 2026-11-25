//! Text combination used by the native clipboard workbench and the file adapter.
use std::fmt::Write as _;

#[derive(Clone, Copy)]
pub enum TextFormat {
    Paragraphs,
    Numbered,
    Csv,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeError {
    CsvWrite,
    CsvEncoding,
    TooLarge,
}

impl ComposeError {
    pub fn native_message(self) -> &'static str {
        match self {
            Self::CsvWrite => "CSV组合失败",
            Self::CsvEncoding => "CSV编码失败",
            Self::TooLarge => "组合结果超过1 MiB，请减少选择",
        }
    }
}

pub fn combine_text(
    parts: &[&str],
    format: TextFormat,
    maximum: usize,
) -> Result<String, ComposeError> {
    let value = match format {
        TextFormat::Paragraphs => parts.join("\n\n"),
        TextFormat::Numbered => {
            let mut output = String::new();
            for (index, text) in parts.iter().enumerate() {
                if index != 0 {
                    output.push('\n');
                }
                let _ = write!(output, "{}. {text}", index + 1);
            }
            output
        }
        TextFormat::Csv => {
            let mut writer = csv::Writer::from_writer(Vec::new());
            writer
                .write_record(["text"])
                .map_err(|_| ComposeError::CsvWrite)?;
            for text in parts {
                if text.trim_start().starts_with(['=', '+', '-', '@'])
                    || text.starts_with(['\t', '\r'])
                {
                    writer
                        .write_record([format!("'{text}")])
                        .map_err(|_| ComposeError::CsvWrite)?;
                } else {
                    writer
                        .write_record([text])
                        .map_err(|_| ComposeError::CsvWrite)?;
                }
            }
            String::from_utf8(writer.into_inner().map_err(|_| ComposeError::CsvWrite)?)
                .map_err(|_| ComposeError::CsvEncoding)?
        }
        TextFormat::Markdown => {
            let mut output = String::new();
            for (index, text) in parts.iter().enumerate() {
                if index != 0 {
                    output.push('\n');
                }
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                let mut lines = normalized.split('\n');
                output.push_str("- ");
                output.push_str(lines.next().unwrap_or_default());
                for line in lines {
                    output.push_str("\n  ");
                    output.push_str(line);
                }
            }
            output
        }
    };
    if value.len() > maximum {
        return Err(ComposeError::TooLarge);
    }
    Ok(value)
}
