//! Explicit, bounded UTF-8 file snapshots for text chat.
use anyhow::{Result, ensure};
use eframe::egui;
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const FILE_LIMIT: usize = 64 * 1024;
const TOTAL_LIMIT: usize = 128 * 1024;
#[derive(Serialize)]
struct Attachment {
    name: String,
    content: String,
}
#[derive(Default)]
pub struct State {
    files: Vec<Attachment>,
    path: String,
    message: String,
    #[cfg(feature = "ui-preview")]
    preview_scroll: bool,
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self) {
        self.files = vec![Attachment {
            name: "示例配置.json".into(),
            content: "{\n  \"name\": \"demo\",\n  \"enabled\": true\n}".into(),
        }];
        self.message = "示例附件快照 · 截图未读取用户文件或发送请求".into();
        self.preview_scroll = true;
    }
    pub fn clear(&mut self) {
        self.files.clear();
        self.path.clear();
        self.message.clear();
    }
    fn load(&mut self) -> Result<()> {
        let path = PathBuf::from(self.path.trim());
        self.load_path(&path)
    }
    fn load_path(&mut self, path: &Path) -> Result<()> {
        ensure!(self.files.len() < 4, "最多添加 4 个文本附件");
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        ensure!(
            matches!(
                extension.as_str(),
                "txt"
                    | "md"
                    | "json"
                    | "yaml"
                    | "yml"
                    | "csv"
                    | "tsv"
                    | "log"
                    | "py"
                    | "java"
                    | "xml"
                    | "toml"
                    | "properties"
                    | "sql"
                    | "js"
                    | "ts"
                    | "html"
                    | "css"
                    | "rs"
                    | "ini"
            ),
            "仅支持文本、代码及常用配置文件；不支持 PDF、Word 或图片"
        );
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("文件名无效"))?
            .to_owned();
        ensure!(
            !self.files.iter().any(|f| f.name == name),
            "已有同名附件，请先移除后重新读取"
        );
        ensure!(std::fs::metadata(path)?.is_file(), "请选择普通文件");
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((FILE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= FILE_LIMIT, "单个附件超过 64 KiB");
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("附件必须为 UTF-8 文本"))?
            .trim_start_matches('\u{feff}');
        ensure!(!text.trim().is_empty(), "附件内容为空");
        ensure!(
            !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
            "附件包含二进制控制字符"
        );
        ensure!(
            self.files.iter().map(|f| f.content.len()).sum::<usize>() + text.len() <= TOTAL_LIMIT,
            "附件正文总量超过 128 KiB"
        );
        self.files.push(Attachment {
            name,
            content: text.to_owned(),
        });
        self.path.clear();
        Ok(())
    }
    pub fn compose(&self, input: &str) -> Result<String> {
        if self.files.is_empty() {
            return Ok(input.to_owned());
        }
        ensure!(!input.trim().is_empty(), "请填写对附件的处理要求");
        let text = format!(
            "{input}\n\n以下为用户选择的文本附件（JSON 中仅含文件名及正文）：\n{}",
            serde_json::to_string_pretty(&self.files)?
        );
        ensure!(
            text.len() <= crate::conversation::MAX_BYTES,
            "输入与附件编码后超过 256 KiB，请减少附件或缩短输入"
        );
        Ok(text)
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, enabled: bool) {
        ui.add_enabled_ui(enabled, |ui| {
            let response = egui::CollapsingHeader::new(format!("文本附件 · {} / 4", self.files.len()))
                .id_salt("chat-attachments")
                .default_open(!self.files.is_empty())
                .show(ui, |ui| {
                ui.weak("发送时将文件名和完整正文交给当前模型。仅读取明确选择的文件，使用预览快照；不发送完整路径。单文件64 KiB，总计128 KiB。");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("UTF-8 文件完整路径").desired_width(ui.available_width() - 104.0));
                    #[cfg(windows)]
                    if ui.button("选择文件").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("文本与代码", &["txt", "md", "json", "yaml", "yml", "csv", "tsv", "log", "py", "java", "xml", "toml", "properties", "sql", "js", "ts", "html", "css", "rs", "ini"])
                            .pick_file()
                        {
                            self.message = match self.load_path(&path) {
                                Ok(()) => "附件已读取并预览，尚未发送；文件变化需移除后重新添加".into(),
                                Err(e) => e.to_string(),
                            };
                        }
                    }
                });
                if ui.button("读取附件并预览").clicked() {
                    self.message = match self.load() { Ok(()) => "附件已读取，尚未发送；文件变化需移除后重新添加".into(), Err(e) => e.to_string() };
                }
                let mut remove = None;
                for (index, file) in self.files.iter().enumerate() {
                    ui.horizontal(|ui| { ui.strong(format!("{} · {} 字节", file.name, file.content.len())); if ui.button("移除").clicked() { remove = Some(index); } });
                    egui::ScrollArea::vertical().id_salt(("attachment", index)).max_height(100.0).show(ui, |ui| { ui.monospace(&file.content); });
                }
                if let Some(index) = remove { self.files.remove(index); self.message = "附件已移除".into(); }
                if !self.message.is_empty() { ui.label(&self.message); }
            });
            #[cfg(feature = "ui-preview")]
            if self.preview_scroll {
                response.header_response.scroll_to_me(Some(egui::Align::TOP));
                self.preview_scroll = false;
            }
            #[cfg(not(feature = "ui-preview"))]
            let _ = response;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_are_bounded_literal_and_do_not_expose_paths() {
        let root = std::env::temp_dir().join(format!("zi-attachment-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("示例.json");
        std::fs::write(&path, "\u{feff}{\"key\":\"$input🙂\"}").unwrap();
        let mut state = State {
            path: path.to_str().unwrap().into(),
            ..Default::default()
        };
        state.load().unwrap();
        std::fs::write(&path, "changed").unwrap();
        let composed = state.compose("解释").unwrap();
        assert!(composed.contains("$input🙂"));
        assert!(!composed.contains("changed"));
        assert!(!composed.contains(root.to_str().unwrap()));
        let body =
            serde_json::json!({"model":"$model","messages":[{"role":"user","content":"$input"}]});
        let payload: serde_json::Value = serde_json::from_slice(
            &crate::plugins::request_body(&body, &composed, "example", None).unwrap(),
        )
        .unwrap();
        assert_eq!(payload["messages"][0]["content"], composed);
        assert!(state.compose("").is_err());
        assert!(
            state
                .compose(&"x".repeat(crate::conversation::MAX_BYTES))
                .is_err()
        );
        state.path = path.to_str().unwrap().into();
        assert!(state.load().is_err());
        state.clear();
        state.path = path.to_str().unwrap().into();
        for bytes in [vec![0, 1], vec![255], vec![b'x'; FILE_LIMIT + 1]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(state.load().is_err());
            assert!(state.files.is_empty());
        }
        state.files = (0..4)
            .map(|i| Attachment {
                name: i.to_string(),
                content: "text".into(),
            })
            .collect();
        assert!(state.load().is_err());
        state.files = vec![Attachment {
            name: "other".into(),
            content: "x".repeat(TOTAL_LIMIT),
        }];
        std::fs::write(&path, "a").unwrap();
        assert!(state.load().is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
