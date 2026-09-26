//! Explicit, bounded file intake. Dropping a file never executes it.
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Csv,
    Tsv,
    JsonData,
    Json,
    Java,
    Python,
    Threads,
    Gc,
    Sql,
    Celery,
    Files,
}
impl Target {
    pub const ALL: [Self; 11] = [
        Self::Csv,
        Self::Tsv,
        Self::JsonData,
        Self::Json,
        Self::Java,
        Self::Python,
        Self::Threads,
        Self::Gc,
        Self::Sql,
        Self::Celery,
        Self::Files,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV 数据表",
            Self::Tsv => "TSV 数据表",
            Self::JsonData => "JSON 对象数组",
            Self::Json => "JSON 格式化",
            Self::Java => "Java 异常链",
            Self::Python => "Python / Django 异常",
            Self::Threads => "JVM 线程转储",
            Self::Gc => "GC 日志",
            Self::Sql => "Django SQL 分析",
            Self::Celery => "Celery 日志",
            Self::Files => "文件校验",
        }
    }
}
pub fn recommend(paths: &[PathBuf]) -> Target {
    if paths.len() != 1 {
        return Target::Files;
    }
    match paths[0]
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "csv" => Target::Csv,
        "tsv" => Target::Tsv,
        "json" => Target::Json,
        "log" | "txt" | "dump" => Target::Java,
        _ => Target::Files,
    }
}
pub struct Imported {
    pub target: Target,
    pub paths: Vec<PathBuf>,
    pub text: String,
}
pub fn read(paths: Vec<PathBuf>, target: Target) -> Result<Imported, String> {
    if paths.is_empty() || paths.len() > 64 {
        return Err("请选择 1–64 个文件".into());
    }
    if target != Target::Files && paths.len() != 1 {
        return Err("文本工具每次导入一个文件；多个文件请选择文件校验".into());
    }
    for path in &paths {
        if !path.is_file() {
            return Err(format!(
                "不是可读取的普通文件：{}；目录不会递归导入",
                path.display()
            ));
        }
    }
    let text = if target == Target::Files {
        String::new()
    } else {
        read_utf8(&paths[0])?
    };
    Ok(Imported {
        target,
        paths,
        text,
    })
}
fn read_utf8(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("文本超过 2 MiB，请先缩小范围；文件校验不受此文本限制".into());
    }
    String::from_utf8(bytes)
        .map(|s| s.trim_start_matches('\u{feff}').to_owned())
        .map_err(|_| "文件不是 UTF-8 文本；请选择文件校验或先转换编码".into())
}
pub struct State {
    pub paths: Vec<PathBuf>,
    pub manual: String,
    pub target: Target,
    pub message: String,
    receiver: Option<mpsc::Receiver<Result<Imported, String>>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            paths: vec![],
            manual: String::new(),
            target: Target::Files,
            message: String::new(),
            receiver: None,
        }
    }
}
impl State {
    pub fn accept(&mut self, paths: Vec<PathBuf>) {
        if self.receiver.is_some() {
            self.message = "正在读取文件，请等待当前导入完成".into();
            return;
        }
        if paths.is_empty() || paths.len() > 64 {
            self.message = "一次可接收 1–64 个文件；未修改当前列表".into();
            return;
        }
        let mut unique = Vec::new();
        for path in paths {
            if !unique.contains(&path) {
                unique.push(path);
            }
        }
        self.target = recommend(&unique);
        self.paths = unique;
        self.message.clear();
    }
    pub fn poll(&mut self) -> Option<Result<Imported, String>> {
        let result = self.receiver.as_ref()?.try_recv();
        match result {
            Ok(result) => {
                self.receiver = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                Some(Err("导入任务意外结束，请重试".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("从文件开始");
        ui.label("拖入文件，选择工具，再导入。所有处理在本机开始，文件不会被执行或自动上传。");
        ui.add_space(16.0);
        ui.label("也可以粘贴文件路径，每行一个");
        ui.add(
            egui::TextEdit::multiline(&mut self.manual)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("C:\\data\\report.csv"),
        );
        if ui
            .add_enabled(!self.busy(), egui::Button::new("使用这些路径"))
            .clicked()
        {
            self.accept(
                self.manual
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| PathBuf::from(s.trim_matches('"')))
                    .collect(),
            );
        }
        ui.separator();
        ui.strong(format!("{} 个待导入文件", self.paths.len()));
        egui::ScrollArea::vertical()
            .max_height(160.0)
            .id_salt("intake-files")
            .show(ui, |ui| {
                for path in &self.paths {
                    ui.label(path.display().to_string());
                }
            });
        ui.add_space(12.0);
        ui.add_enabled_ui(!self.busy(), |ui| {
            egui::ComboBox::from_label("打开方式")
                .selected_text(self.target.label())
                .show_ui(ui, |ui| {
                    for target in Target::ALL {
                        ui.selectable_value(&mut self.target, target, target.label());
                    }
                });
            ui.label("扩展名只用于推荐。日志无法仅凭文件名判断语言，请选择实际类型。");
            if self.target != Target::Files {
                ui.label(
                    "导入会替换目标工具的当前输入草稿；不会自动执行诊断。数据表导入后会解析预览。",
                );
            }
            let label = if self.target == Target::Files {
                "添加到文件校验"
            } else {
                "替换目标草稿并打开"
            };
            if ui
                .add_enabled(
                    !self.paths.is_empty(),
                    egui::Button::new(label).min_size(egui::vec2(230.0, 36.0)),
                )
                .clicked()
            {
                let paths = self.paths.clone();
                let target = self.target;
                let (tx, rx) = mpsc::channel();
                self.receiver = Some(rx);
                self.message.clear();
                std::thread::spawn(move || {
                    let _ = tx.send(read(paths, target));
                });
            }
        });
        if self.busy() {
            ui.spinner();
            ui.label("正在读取…");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if !self.message.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.message);
        }
        ui.add_space(12.0);
        ui.small("文本限 2 MiB / UTF-8；最多 64 个文件；不递归扫描目录。");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recommendation_is_case_insensitive_and_batches_use_hashing() {
        assert_eq!(recommend(&["测试.CSV".into()]), Target::Csv);
        assert_eq!(recommend(&["a.json".into()]), Target::Json);
        assert_eq!(recommend(&["a.csv".into(), "b.csv".into()]), Target::Files);
        assert_eq!(recommend(&["a.exe".into()]), Target::Files);
    }
    #[test]
    fn import_limits_and_bom_and_binary() {
        let dir = std::env::temp_dir().join(format!("zi-intake-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("样例.txt");
        std::fs::write(&path, b"\xef\xbb\xbfhello").unwrap();
        assert_eq!(
            read(vec![path.clone()], Target::Json).unwrap().text,
            "hello"
        );
        std::fs::write(&path, [255, 254]).unwrap();
        assert!(read(vec![path.clone()], Target::Java).is_err());
        assert!(read(vec![path.clone()], Target::Files).is_ok());
        std::fs::write(&path, vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(read(vec![path.clone()], Target::Json).is_err());
        assert!(read(vec![dir.clone()], Target::Files).is_err());
        assert!(read(vec![path; 65], Target::Files).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
