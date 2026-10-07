//! Explicit output of a frozen workflow result; never applies it to the source table.
use super::*;
use std::path::Path;

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Format {
    #[default]
    Csv,
    Sqlite,
}
impl Format {
    fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Sqlite => "SQLite",
        }
    }
}
enum Payload {
    Csv(Dataset, bool),
    Sqlite(sqlite_export::Review),
}
struct Review {
    source: Dataset,
    definition: workflow::Definition,
    result: Dataset,
    path: PathBuf,
    format: Format,
    payload: Payload,
    samples: Vec<String>,
    table: Option<String>,
}
impl Review {
    fn prepare(
        source: &Dataset,
        definition: &workflow::Definition,
        result: &Dataset,
        path: &Path,
        format: Format,
        table: &str,
        safe: bool,
    ) -> Result<Self> {
        result.validate_saved()?;
        struct Counter(usize);
        impl Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                if self.0 > 32 * 1024 * 1024 {
                    return Err(std::io::Error::other("流程结果快照超过32MiB"));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Counter(0), result)?;
        anyhow::ensure!(
            path.is_absolute() && path.file_name().is_some(),
            "请输入新文件的完整路径"
        );
        let parent = path
            .parent()
            .context("目标目录无效")?
            .canonicalize()
            .context("目标目录不存在")?;
        anyhow::ensure!(parent.is_dir(), "目标父路径不是目录");
        let path = parent.join(path.file_name().unwrap());
        anyhow::ensure!(
            std::fs::symlink_metadata(&path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "目标已存在或无法确认，不覆盖任何文件"
        );
        let payload = match format {
            Format::Csv => Payload::Csv(result.clone(), safe),
            Format::Sqlite => Payload::Sqlite(sqlite_export::Review::prepare(
                result,
                &(0..result.rows.len()).collect::<Vec<_>>(),
                table,
                &path,
                "完整流程结果 · 未应用",
            )?),
        };
        let samples = result
            .rows
            .iter()
            .take(5)
            .map(|row| {
                row.iter()
                    .map(|v| cell_text(v).chars().take(60).collect::<String>())
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect();
        Ok(Self {
            source: source.clone(),
            definition: definition.clone(),
            result: result.clone(),
            path,
            format,
            payload,
            samples,
            table: (format == Format::Sqlite).then(|| table.to_string()),
        })
    }
    fn current(
        &self,
        source: Option<&Dataset>,
        definition: &workflow::Definition,
        result: Option<&Dataset>,
    ) -> bool {
        source == Some(&self.source)
            && definition == &self.definition
            && result == Some(&self.result)
    }
}
fn protect(text: String, safe: bool) -> String {
    if safe
        && (text.starts_with(['\t', '\r', '\n'])
            || text.trim_start().starts_with(['=', '+', '-', '@']))
    {
        format!("'{text}")
    } else {
        text
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(sqlite_export::Cancelled.into())
    } else {
        Ok(())
    }
}
fn csv_write(
    data: &Dataset,
    path: &Path,
    safe: bool,
    cancel: &AtomicBool,
    before_publish: impl FnOnce(),
) -> Result<sqlite_export::Receipt> {
    check_cancel(cancel)?;
    let parent = path.parent().context("目标目录无效")?;
    anyhow::ensure!(
        parent.canonicalize()? == parent,
        "目标目录位置已改变，请重新预览"
    );
    let temporary = sqlite_export::Temporary(
        parent.join(format!(".zi-workflow-output-{}.csv", uuid::Uuid::new_v4())),
    );
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.0)?;
    struct Bounded {
        file: File,
        bytes: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.saturating_add(bytes.len()) > 32 * 1024 * 1024 {
                return Err(std::io::Error::other("CSV输出超过32MiB"));
            }
            let n = self.file.write(bytes)?;
            self.bytes += n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    let mut writer = csv::Writer::from_writer(Bounded { file, bytes: 0 });
    writer.write_record(data.headers.iter().map(|s| protect(s.clone(), safe)))?;
    for row in &data.rows {
        check_cancel(cancel)?;
        writer.write_record(
            row.iter()
                .map(|v| protect(cell_text(v), safe && v.is_string())),
        )?;
    }
    writer.flush()?;
    let bounded = writer.into_inner().map_err(|e| anyhow!(e.to_string()))?;
    bounded.file.sync_all()?;
    let bytes = bounded.bytes as u64;
    drop(bounded);
    before_publish();
    check_cancel(cancel)?;
    std::fs::hard_link(&temporary.0, path)
        .context("无法无覆盖发布新文件：目标可能已出现，或磁盘不支持硬链接")?;
    Ok(sqlite_export::Receipt {
        path: path.to_path_buf(),
        rows: data.rows.len(),
        bytes,
    })
}
enum Reply {
    Done(sqlite_export::Receipt),
    Cancelled,
    Failed(String),
}
pub(super) struct State {
    pub(super) job: Job,
    pub(super) reveal: bool,
    scroll_until: Option<std::time::Instant>,
    format: Format,
    path: String,
    table: String,
    safe: bool,
    review: Option<Review>,
    receiver: Option<Receiver<Reply>>,
    cancel: Arc<AtomicBool>,
    status: String,
    receipt: Option<sqlite_export::Receipt>,
    #[cfg(feature = "ui-preview")]
    pub(super) buttons: [Option<egui::Rect>; 5],
}
impl Default for State {
    fn default() -> Self {
        Self {
            job: Job::default(),
            reveal: false,
            scroll_until: None,
            format: Format::Csv,
            path: String::new(),
            table: "workflow_result".into(),
            safe: true,
            review: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            status: String::new(),
            receipt: None,
            #[cfg(feature = "ui-preview")]
            buttons: [None; 5],
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub(super) fn load_settings(&mut self, output: Option<&workflow::Output>) {
        self.path.clear();
        self.review = None;
        self.format = Format::Csv;
        self.safe = true;
        self.table = "workflow_result".into();
        match output {
            Some(workflow::Output::Csv {
                protect_formulas, ..
            }) => self.safe = *protect_formulas,
            Some(workflow::Output::Sqlite { table, .. }) => {
                self.format = Format::Sqlite;
                self.table = table.clone();
            }
            None => {}
        }
    }
    fn settings(&self) -> workflow::Output {
        match self.format {
            Format::Csv => workflow::Output::Csv {
                version: 1,
                protect_formulas: self.safe,
            },
            Format::Sqlite => workflow::Output::Sqlite {
                version: 1,
                table: self.table.trim().into(),
            },
        }
    }
    pub(super) fn wants_scroll(&self) -> bool {
        self.reveal
            || self
                .scroll_until
                .is_some_and(|until| std::time::Instant::now() < until)
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_prepare(&mut self, path: &Path, sqlite: bool) {
        self.path = path.display().to_string();
        self.format = if sqlite { Format::Sqlite } else { Format::Csv };
        self.reveal = true;
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_check(&self, phase: u8) -> bool {
        match phase {
            1 => assert!(self.review.is_some() && self.receipt.is_none()),
            2 => assert!(self.review.is_none() && self.receipt.is_none()),
            3 => {
                if self.job.phase.active() {
                    return false;
                }
                assert_eq!(self.job.phase, Phase::Done, "{}", self.status);
                assert_eq!(self.receipt.as_ref().unwrap().rows, 2);
            }
            _ => panic!("unknown workflow output fixture phase"),
        }
        true
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_imported(&self, output: &workflow::Output) {
        assert_eq!(&self.settings(), output);
        assert!(self.path.is_empty() && self.receiver.is_none() && self.review.is_none());
        assert_eq!(
            self.job.phase,
            Phase::Done,
            "import must not start another output"
        );
    }
    pub(super) fn busy(&self) -> bool {
        self.job.phase.active() || self.review.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.review = None;
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.job.cancelling();
    }
    pub(super) fn poll(&mut self) {
        let reply = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Reply::Failed(
                "输出线程意外结束，请检查目标文件后重试".into(),
            )),
        });
        let Some(reply) = reply else { return };
        self.receiver = None;
        self.reveal = true;
        match reply {
            Reply::Done(receipt) => {
                self.status = format!(
                    "已保存完整流程结果：{}行 · {}字节 · {}；原表与预览保留",
                    receipt.rows,
                    receipt.bytes,
                    display_path(&receipt.path)
                );
                self.job
                    .finish(Phase::Done, format!("已保存{}行流程结果", receipt.rows));
                self.receipt = Some(receipt);
            }
            Reply::Cancelled => {
                self.status = "输出已取消，未发布新文件；原表与预览保留".into();
                self.job.finish(Phase::Cancelled, "流程输出已取消");
            }
            Reply::Failed(error) => {
                self.status = error;
                self.job
                    .finish(Phase::Failed, "流程输出失败，原表与预览保留");
            }
        }
    }
    fn start(&mut self) {
        let Some(review) = self.review.take() else {
            return;
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status.clear();
        self.receipt = None;
        self.job.begin();
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let result = match review.payload {
                Payload::Csv(data, safe) => csv_write(&data, &review.path, safe, &cancel, || {}),
                Payload::Sqlite(sqlite) => sqlite_export::write(&sqlite, &cancel),
            };
            let reply = match result {
                Ok(receipt) => Reply::Done(receipt),
                Err(e) if e.downcast_ref::<sqlite_export::Cancelled>().is_some() => {
                    Reply::Cancelled
                }
                Err(e) => Reply::Failed(format!("{e:#}")),
            };
            let _ = tx.send(reply);
        });
    }
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        source: Option<&Dataset>,
        definition: &mut workflow::Definition,
        result: Option<&Dataset>,
    ) {
        if definition.steps.is_empty()
            && result.is_none()
            && self.review.is_none()
            && self.status.is_empty()
            && !self.job.phase.active()
        {
            return;
        }
        if self.reveal {
            self.scroll_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
        }
        let scroll = self
            .scroll_until
            .is_some_and(|until| std::time::Instant::now() < until);
        let panel = egui::CollapsingHeader::new("保存流程结果 · 原表无需替换")
            .id_salt("workflow-output")
            .open(scroll.then_some(true))
            .default_open(false)
            .show(ui, |ui| {
                ui.small("使用完整预览结果；选择新文件并确认。保存位置和授权不写入流程定义。");
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                ui.add_enabled_ui(!self.busy(), |ui| {
                    let settings_before = self.settings();
                    let mut remember = definition.output.is_some();
                    let save_settings =
                        ui.checkbox(&mut remember, "将输出设置保存在流程中（不含目标或授权）");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.buttons[4] = Some(save_settings.rect.intersect(ui.clip_rect()));
                    }
                    if save_settings.changed() && !remember {
                        definition.output = None;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut self.format, Format::Csv, "CSV");
                        ui.selectable_value(&mut self.format, Format::Sqlite, "SQLite");
                        if self.format == Format::Csv {
                            ui.checkbox(&mut self.safe, "防表格公式（文本加单引号前缀）");
                        }
                    });
                    if self.format == Format::Sqlite {
                        ui.horizontal(|ui| {
                            ui.label("表名");
                            ui.text_edit_singleline(&mut self.table);
                        });
                    }
                    if remember {
                        definition.version = 3;
                        definition.output = Some(self.settings());
                    }
                    if save_settings.changed() || settings_before != self.settings() {
                        self.scroll_until =
                            Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
                    }
                });
                ui.add_enabled_ui(result.is_some() && source.is_some() && !self.busy(), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.path)
                            .hint_text("新文件完整路径")
                            .desired_width(ui.available_width()),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("选择新文件…").clicked() {
                            let (name, extensions) = if self.format == Format::Csv {
                                ("workflow-result.csv", vec!["csv"])
                            } else {
                                ("workflow-result.sqlite", vec!["sqlite", "db"])
                            };
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter(self.format.label(), &extensions)
                                .set_file_name(name)
                                .save_file()
                            {
                                self.path = path.display().to_string();
                            }
                        }
                        let preview = ui.button("预览保存流程结果…");
                        #[cfg(feature = "ui-preview")]
                        {
                            self.buttons[0] = Some(preview.rect.intersect(ui.clip_rect()));
                        }
                        if preview.clicked()
                            && let (Some(source), Some(result)) = (source, result)
                        {
                            match Review::prepare(
                                source,
                                definition,
                                result,
                                Path::new(self.path.trim()),
                                self.format,
                                self.table.trim(),
                                self.safe,
                            ) {
                                Ok(review) => {
                                    self.review = Some(review);
                                    self.status.clear();
                                }
                                Err(e) => self.status = format!("{e:#}"),
                            }
                        }
                    });
                });
                if self.job.phase.active() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(self.job.phase.label());
                        let cancel = ui.button("取消流程输出");
                        #[cfg(feature = "ui-preview")]
                        {
                            self.buttons[3] = Some(cancel.rect.intersect(ui.clip_rect()));
                        }
                        if cancel.clicked() {
                            self.cancel();
                        }
                    });
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(30));
                }
            });
        self.reveal = false;
        if scroll {
            if let Some(body) = panel.body_response {
                body.scroll_to_me(Some(egui::Align::Max));
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let mut action = 0;
        if let Some(review) = &self.review {
            let current = review.current(source, definition, result);
            let modal=egui::Modal::new(ui.id().with("workflow-output-review")).show(ui.ctx(),|ui| {
                ui.set_max_width(620.0);ui.heading("确认保存流程结果为新文件");
                ui.label(display_path(&review.path));
                ui.label(format!("{} · 全部{}行 / {}列 · {}步流程；尚未应用到原表",review.format.label(),review.result.rows.len(),review.result.headers.len(),review.definition.steps.len()));
                if let Some(table)=&review.table {ui.label(format!("表名：{table}"));}
                match &review.payload {
                    Payload::Csv(_,safe)=>{ui.label(format!("CSV不保留JSON类型；null写为空单元格，数组/对象写为JSON文本；{}。",if *safe {"公式前缀防护开启，相关文本会增加单引号"} else {"公式前缀防护关闭，原文本保留"}));}
                    Payload::Sqlite(_)=>{ui.label("SQLite保留NULL/普通整数/浮点/文本；布尔转0/1，超大整数转十进制文本，数组/对象转JSON文本；CSV源字符串不转数字。");}
                }
                egui::ScrollArea::both().max_height(150.0).show(ui,|ui| {ui.label(review.result.headers.join(" · "));for sample in &review.samples {ui.monospace(sample);}});
                ui.label("只创建新文件，拒绝覆盖；发布前可取消，发布完成后保留文件。部分磁盘可能无法安全保存；最多32MiB CSV输出。");
                if !current {ui.colored_label(ui.visuals().error_fg_color,"来源、步骤或预览已变化，请返回后重新预览。");}
                ui.horizontal(|ui| {
                    let back = ui.button("返回编辑");
                    let save = ui.add_enabled(current, egui::Button::new("确认保存新文件"));
                    #[cfg(feature="ui-preview")]
                    {
                        self.buttons[1] = Some(back.rect);
                        self.buttons[2] = Some(save.rect);
                    }
                    if back.clicked() {
                        action = 1;
                    } else if save.clicked() {
                        action = 2;
                    }
                });
            });
            if modal.should_close() {
                action = 1;
            }
        }
        if action == 1 {
            self.review = None;
        }
        if action == 2 {
            self.start();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_output_settings_reset_target_without_starting_a_writer() {
        let mut state = State::default();
        for output in [
            workflow::Output::Sqlite {
                version: 1,
                table: "记录\"表".into(),
            },
            workflow::Output::Csv {
                version: 1,
                protect_formulas: false,
            },
        ] {
            state.path = "C:/prior-target.csv".into();
            state.load_settings(Some(&output));
            assert_eq!(state.settings(), output);
            assert!(state.path.is_empty() && state.review.is_none() && state.receiver.is_none());
            assert!(!state.job.phase.active());
        }
        state.load_settings(None);
        assert_eq!(
            state.settings(),
            workflow::Output::Csv {
                version: 1,
                protect_formulas: true
            }
        );
    }
    #[test]
    fn workflow_output_rejects_oversize_snapshot_before_creating_a_file() {
        let fixture = Fixture::new();
        let (source, definition, mut result) = frozen();
        result.rows = vec![result.rows[0].clone()];
        result.rows[0][0] = serde_json::Value::String("x".repeat(32 * 1024 * 1024));
        assert!(
            Review::prepare(
                &source,
                &definition,
                &result,
                &fixture.file("large.csv"),
                Format::Csv,
                "result",
                true
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
        assert_eq!(
            display_path(Path::new(r"\\?\C:\notes\result.csv")),
            r"C:\notes\result.csv"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\result.csv")),
            r"\\server\share\result.csv"
        );
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("zi-flow-output-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for item in std::fs::read_dir(&self.0).unwrap() {
                let path = item.unwrap().path();
                assert_eq!(path.parent(), Some(self.0.as_path()));
                assert!(
                    !std::fs::symlink_metadata(&path)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                std::fs::remove_file(path).unwrap();
            }
            std::fs::remove_dir(&self.0).unwrap();
        }
    }
    fn frozen() -> (Dataset, workflow::Definition, Dataset) {
        let mut source=Dataset::parse(r#"[{"id":"001","name":" 中文🦀 ","formula":"=1+1","flag":true,"large":18446744073709551615,"empty":null,"number":-2}]"#,DataFormat::Json,b',').unwrap();
        source.rows = vec![source.rows[0].clone(); 300];
        let definition = workflow::Definition {
            output: None,
            version: 2,
            name: "完整结果".into(),
            steps: vec![workflow::Step::Column {
                column: "name".into(),
                operation: workflow::ColumnOperation::Trim,
                value: String::new(),
            }],
        };
        let result = definition
            .preview(&source, &AtomicBool::new(false))
            .unwrap()
            .result;
        (source, definition, result)
    }
    #[test]
    fn workflow_csv_writes_all_transformed_rows_and_protects_only_text_formulas() {
        let fixture = Fixture::new();
        let (source, definition, result) = frozen();
        let original = source.clone();
        let review = Review::prepare(
            &source,
            &definition,
            &result,
            &fixture.file("result.csv"),
            Format::Csv,
            "ignored",
            true,
        )
        .unwrap();
        assert!(review.current(Some(&source), &definition, Some(&result)));
        let receipt =
            csv_write(&result, &review.path, true, &AtomicBool::new(false), || {}).unwrap();
        assert_eq!(receipt.rows, 300);
        let mut reader = csv::Reader::from_path(&receipt.path).unwrap();
        let headers = reader.headers().unwrap().clone();
        let rows = reader
            .records()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows.len(), 300);
        for row in rows {
            let cell = |name: &str| {
                row.get(headers.iter().position(|h| h == name).unwrap())
                    .unwrap()
            };
            assert_eq!(cell("id"), "001");
            assert_eq!(cell("name"), "中文🦀");
            assert_eq!(cell("formula"), "'=1+1");
            assert_eq!(cell("empty"), "");
            assert_eq!(cell("number"), "-2");
            assert_eq!(cell("large"), u64::MAX.to_string());
        }
        assert_eq!(source, original);
        assert_eq!(protect(" \t=1+1".into(), true), "' \t=1+1");
        assert_eq!(protect("=1+1".into(), false), "=1+1");
        assert_eq!(
            std::fs::read_dir(&fixture.0).unwrap().count(),
            1,
            "no temporary left"
        );
    }
    #[test]
    fn workflow_output_rejects_changed_source_steps_result_and_existing_target() {
        let fixture = Fixture::new();
        let (source, definition, result) = frozen();
        let target = fixture.file("result.csv");
        let review = Review::prepare(
            &source,
            &definition,
            &result,
            &target,
            Format::Csv,
            "data",
            true,
        )
        .unwrap();
        let mut changed = source.clone();
        changed.rows[0][0] = Value::String("changed source".into());
        assert!(!review.current(Some(&changed), &definition, Some(&result)));
        let mut changed_definition = definition.clone();
        changed_definition.name = "变化".into();
        assert!(!review.current(Some(&source), &changed_definition, Some(&result)));
        let mut changed_result = result.clone();
        changed_result.rows.clear();
        assert!(!review.current(Some(&source), &definition, Some(&changed_result)));
        assert!(!review.current(Some(&source), &definition, None));
        std::fs::write(&target, "owner").unwrap();
        assert!(
            Review::prepare(
                &source,
                &definition,
                &result,
                &target,
                Format::Csv,
                "data",
                true
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(target).unwrap(), "owner");
    }
    #[test]
    fn workflow_csv_cancel_and_destination_race_publish_nothing_or_preserve_owner() {
        let fixture = Fixture::new();
        let (_, _, result) = frozen();
        let path = fixture.file("result.csv");
        let cancel = AtomicBool::new(true);
        assert!(
            csv_write(&result, &path, true, &cancel, || {})
                .err()
                .unwrap()
                .downcast_ref::<sqlite_export::Cancelled>()
                .is_some()
        );
        cancel.store(false, Ordering::Relaxed);
        assert!(
            csv_write(&result, &path, true, &cancel, || cancel
                .store(true, Ordering::Relaxed))
            .is_err()
        );
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
        cancel.store(false, Ordering::Relaxed);
        assert!(
            csv_write(&result, &path, true, &cancel, || std::fs::write(
                &path,
                "racing owner"
            )
            .unwrap())
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "racing owner");
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 1);
    }
    #[test]
    fn workflow_sqlite_preserves_typed_result_without_applying_source() {
        let fixture = Fixture::new();
        let (source, definition, result) = frozen();
        let original = source.clone();
        let review = Review::prepare(
            &source,
            &definition,
            &result,
            &fixture.file("result.sqlite"),
            Format::Sqlite,
            "结果",
            true,
        )
        .unwrap();
        let Payload::Sqlite(sqlite) = review.payload else {
            panic!("SQLite payload")
        };
        let receipt = sqlite_export::write(&sqlite, &AtomicBool::new(false)).unwrap();
        let conn = rusqlite::Connection::open_with_flags(
            receipt.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let actual: (usize, String, i64, String, Option<String>) = conn
            .query_row(
                "SELECT COUNT(*), name, flag, large, empty FROM 结果",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            actual,
            (300, "中文🦀".into(), 1, u64::MAX.to_string(), None)
        );
        assert_eq!(source, original);
    }
    #[test]
    fn workflow_output_task_follows_owner_and_stale_cancel_cannot_target_new_run() {
        let fixture = Fixture::new();
        let (source, definition, result) = frozen();
        let mut workspace = sessions::Workspace::new(fixture.file("workspace.sqlite3"));
        let id = workspace.active_id().to_string();
        workspace.dataset = Some(source.clone());
        workspace.workflow.output.review = Some(
            Review::prepare(
                &source,
                &definition,
                &result,
                &fixture.file("result.csv"),
                Format::Csv,
                "data",
                true,
            )
            .unwrap(),
        );
        assert!(workspace.busy());
        workspace.workflow.output.start();
        let generation = workspace
            .workflow
            .output
            .job
            .snapshot("workflow-output", "", true)
            .unwrap()
            .generation;
        workspace.cancel_workflow_output(&id, generation + 1);
        assert!(!workspace.workflow.output.cancel.load(Ordering::Relaxed));
        let other = workspace.create("另一个实例").unwrap();
        assert_ne!(other, id);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            workspace.poll();
            let owner = workspace.instances.iter().find(|i| i.id == id).unwrap();
            if !owner.state.workflow.output.job.phase.active() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let row = workspace
            .snapshots()
            .into_iter()
            .find(|r| r.key == "workflow-output")
            .unwrap();
        assert_eq!(row.instance.as_deref(), Some(id.as_str()));
        assert_eq!(row.phase, Phase::Done);
        let owner = workspace.instances.iter().find(|i| i.id == id).unwrap();
        assert_eq!(owner.state.dataset.as_ref(), Some(&source));
        assert_eq!(
            owner.state.workflow.output.receipt.as_ref().unwrap().rows,
            300
        );
        assert_eq!(workspace.active_id(), other);
        assert!(workspace.dataset.is_none());
    }
}
