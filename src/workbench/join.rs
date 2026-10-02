use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    #[default]
    Left,
    Inner,
    Append,
    Columns,
}
impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Left => "左连接",
            Self::Inner => "内连接",
            Self::Append => "按行追加（同名列）",
            Self::Columns => "按行号拼列",
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {
    data: Dataset,
    report: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    input: String,
    path: String,
    format: DataFormat,
    tsv: bool,
    mode: Mode,
    left_key: usize,
    right_key: String,
    preview: Option<Preview>,
    #[serde(skip)]
    receiver: Option<Receiver<std::result::Result<Preview, String>>>,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
    force_open: bool,
    #[serde(skip)]
    message: String,
    #[serde(skip)]
    pub(super) job: Job,
    #[serde(skip)]
    invalidated: bool,
}
impl State {
    pub(super) fn has_content(&self) -> bool {
        !self.input.is_empty() || self.preview.is_some()
    }
    pub(super) fn validate_saved(&self, columns: usize) -> Result<()> {
        anyhow::ensure!(
            self.input.len() <= INPUT_LIMIT
                && self.path.len() <= 32768
                && self.right_key.len() <= INPUT_LIMIT,
            "合并草稿过大"
        );
        anyhow::ensure!(self.left_key < columns.max(1), "合并列索引无效");
        if let Some(p) = &self.preview {
            p.data.validate_saved()?;
        }
        Ok(())
    }
    pub(super) fn invalidate(&mut self) {
        if self.receiver.is_some() {
            self.cancel.store(true, Ordering::Relaxed);
            self.job.cancelling();
            self.invalidated = true;
        }
        self.preview = None;
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn key(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("已取消，原数据未修改");
    }
    Ok(())
}
fn merged_headers(left: &[String], right: &[String]) -> (Vec<String>, usize) {
    let mut headers = left.to_vec();
    let mut used: HashSet<String> = left.iter().cloned().collect();
    let mut conflicts = 0;
    for name in right {
        let mut candidate = name.clone();
        let mut suffix = 1;
        while used.contains(&candidate) {
            candidate = format!("{name}_right{suffix}");
            suffix += 1;
        }
        conflicts += usize::from(candidate != *name);
        used.insert(candidate.clone());
        headers.push(candidate);
    }
    (headers, conflicts)
}
fn push_row(rows: &mut Vec<Vec<Value>>, row: Vec<Value>, bytes: &mut usize) -> Result<()> {
    if rows.len() >= ROW_LIMIT {
        bail!("结果超过 10000 行，可能存在重复键展开；请先缩小范围");
    }
    *bytes += row.iter().map(|v| v.to_string().len()).sum::<usize>();
    if *bytes > 8 * 1024 * 1024 {
        bail!("结果内容超过 8 MiB；原数据未修改");
    }
    rows.push(row);
    Ok(())
}
fn combine(
    left: &Dataset,
    right: &Dataset,
    mode: Mode,
    left_key: usize,
    right_key: &str,
    cancel: &AtomicBool,
) -> Result<Preview> {
    check_cancel(cancel)?;
    let (headers, conflicts) = if mode == Mode::Append {
        if left.headers.len() != right.headers.len()
            || left.headers.iter().any(|h| !right.headers.contains(h))
        {
            bail!("按行追加要求两表列名集合相同；列顺序可以不同");
        }
        (left.headers.clone(), 0)
    } else {
        merged_headers(&left.headers, &right.headers)
    };
    if headers.len() > COLUMN_LIMIT {
        bail!("合并后超过 128 列，请先选择保留列");
    }
    let mut bytes = headers.iter().map(String::len).sum();
    let mut rows = Vec::new();
    let mut matched = 0usize;
    let mut unmatched_left = 0usize;
    let mut unmatched_right = 0usize;
    let mut duplicate_left = 0usize;
    let mut duplicate_right = 0usize;
    match mode {
        Mode::Append => {
            let columns: Vec<_> = left
                .headers
                .iter()
                .map(|h| right.headers.iter().position(|r| r == h).unwrap())
                .collect();
            for row in &left.rows {
                check_cancel(cancel)?;
                push_row(&mut rows, row.clone(), &mut bytes)?;
            }
            for row in &right.rows {
                check_cancel(cancel)?;
                push_row(
                    &mut rows,
                    columns.iter().map(|&i| row[i].clone()).collect(),
                    &mut bytes,
                )?;
            }
        }
        Mode::Columns => {
            if left.rows.len() != right.rows.len() {
                bail!("按行号拼列要求两表行数相同，避免错位或丢失数据");
            }
            for (l, r) in left.rows.iter().zip(&right.rows) {
                check_cancel(cancel)?;
                push_row(&mut rows, l.iter().chain(r).cloned().collect(), &mut bytes)?;
            }
        }
        Mode::Left | Mode::Inner => {
            if left_key >= left.headers.len() {
                bail!("左表关联列已变化，请重新选择");
            }
            let rk = if right_key.trim().is_empty() {
                0
            } else {
                right
                    .headers
                    .iter()
                    .position(|h| h == right_key.trim())
                    .ok_or_else(|| anyhow!("右表不存在指定关联列"))?
            };
            let mut index: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, row) in right.rows.iter().enumerate() {
                check_cancel(cancel)?;
                if let Some(k) = key(&row[rk]) {
                    index.entry(k).or_default().push(i);
                }
            }
            duplicate_right = index.values().filter(|v| v.len() > 1).count();
            let mut seen = HashMap::<String, usize>::new();
            let mut used = HashSet::new();
            for l in &left.rows {
                check_cancel(cancel)?;
                let k = key(&l[left_key]);
                if let Some(k) = &k {
                    *seen.entry(k.clone()).or_default() += 1;
                }
                let matches = k.as_ref().and_then(|k| index.get(k));
                if let Some(matches) = matches {
                    matched += 1;
                    for &i in matches {
                        check_cancel(cancel)?;
                        used.insert(i);
                        push_row(
                            &mut rows,
                            l.iter().chain(&right.rows[i]).cloned().collect(),
                            &mut bytes,
                        )?;
                    }
                } else {
                    unmatched_left += 1;
                    if mode == Mode::Left {
                        push_row(
                            &mut rows,
                            l.iter()
                                .cloned()
                                .chain(vec![Value::Null; right.headers.len()])
                                .collect(),
                            &mut bytes,
                        )?;
                    }
                }
            }
            duplicate_left = seen.values().filter(|&&n| n > 1).count();
            unmatched_right = right.rows.len() - used.len();
        }
    }
    let mut report = format!(
        "{}：左表 {} 行，右表 {} 行 → 结果 {} 行 / {} 列。右表 {} 个同名列已加 _rightN 后缀。",
        mode.label(),
        left.rows.len(),
        right.rows.len(),
        rows.len(),
        headers.len(),
        conflicts
    );
    if matches!(mode, Mode::Left | Mode::Inner) {
        report.push_str(&format!(" 匹配左行 {matched}，未匹配左行 {unmatched_left}，未匹配右行 {unmatched_right}；重复键组：左 {duplicate_left}，右 {duplicate_right}。"));
    }
    check_cancel(cancel)?;
    Ok(Preview {
        data: Dataset { headers, rows },
        report,
    })
}

impl DataState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_join(&mut self) {
        self.input = "id,name\n1,Alice\n2,Bob\n3,Carol".into();
        self.format = DataFormat::Csv;
        self.dataset = Some(Dataset::parse(&self.input, self.format, b',').unwrap());
        self.visible = vec![0, 1, 2];
        self.join = State::default();
        self.join.input = "id,team\n1,Design\n1,Engineering\n2,Support\n4,Research".into();
        self.join.preview = Some(
            combine(
                self.dataset.as_ref().unwrap(),
                &Dataset::parse(&self.join.input, DataFormat::Csv, b',').unwrap(),
                Mode::Left,
                0,
                "id",
                &AtomicBool::new(false),
            )
            .unwrap(),
        );
        self.join.force_open = true;
    }
    pub fn show_join(&mut self) {
        self.join.force_open = true;
    }
    pub(super) fn poll_join(&mut self) {
        if let Some(receiver) = &self.join.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.join.receiver = None;
                    if self.join.invalidated || self.join.cancel.load(Ordering::Relaxed) {
                        self.join
                            .job
                            .finish(Phase::Cancelled, "任务已取消，原数据未修改");
                        self.join.message = "任务已取消，原数据未修改".into();
                        self.join.invalidated = false;
                        return;
                    }
                    match result {
                        Ok(preview) => {
                            self.join
                                .job
                                .finish(Phase::Done, "合并预览已生成，等待应用");
                            self.join.message.clear();
                            self.join.preview = Some(preview);
                        }
                        Err(e) => {
                            self.join.job.finish(
                                if self.join.cancel.load(Ordering::Relaxed) {
                                    Phase::Cancelled
                                } else {
                                    Phase::Failed
                                },
                                "打开合并工具查看详情",
                            );
                            self.join.message = e;
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.join.receiver = None;
                    self.join.job.finish(Phase::Failed, "任务意外结束");
                    self.join.message = "合并任务意外结束，请重试".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
    pub fn join_job(&self) -> &Job {
        &self.join.job
    }
    pub fn cancel_join(&mut self) {
        self.join.cancel.store(true, Ordering::Relaxed);
        self.join.job.cancelling();
    }
    pub(super) fn join_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(left) = &self.dataset else {
            return;
        };
        self.join.left_key = self.join.left_key.min(left.headers.len() - 1);
        let mut apply = false;
        if self.join.force_open {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        egui::CollapsingHeader::new("合并与关联 · 当前数据表作为左表").id_salt("data-join")
            .open(self.join.force_open.then_some(true)).show(ui, |ui| {
                let busy = self.join.receiver.is_some();
                let mut changed = false;
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for mode in [Mode::Left, Mode::Inner, Mode::Append, Mode::Columns] { changed |= ui.selectable_value(&mut self.join.mode, mode, mode.label()).changed(); }
                    });
                    ui.small("使用左表全部行（忽略筛选）。关联按类型和值精确匹配，不自动去空白；null、空白字符串和对象不匹配。重复键会展开所有组合。按行号拼列不按键匹配。");
                    ui.horizontal(|ui| {
                        changed |= ui.selectable_value(&mut self.join.format, DataFormat::Csv, "右表 CSV / TSV").changed();
                        changed |= ui.selectable_value(&mut self.join.format, DataFormat::Json, "右表 JSON 数组").changed();
                        changed |= ui.checkbox(&mut self.join.tsv, "Tab 分隔").changed();
                    });
                    changed |= ui.add(egui::TextEdit::multiline(&mut self.join.input).desired_rows(4).desired_width(f32::INFINITY).hint_text("粘贴右表；CSV 首行为列名")).changed();
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.join.path).hint_text("右表 UTF-8 文件路径"));
                        if ui.add_enabled(self.join.input.is_empty(), egui::Button::new("载入右表文件")).clicked() {
                            match read_text_file(&self.join.path) { Ok(s) => { self.join.input = s; changed = true; }, Err(e) => self.join.message = e.to_string() }
                        }
                    });
                    if matches!(self.join.mode, Mode::Left | Mode::Inner) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("左键");
                            egui::ComboBox::from_id_salt("join-left-key").selected_text(&left.headers[self.join.left_key]).show_ui(ui, |ui| {
                                for (i, h) in left.headers.iter().enumerate() { changed |= ui.selectable_value(&mut self.join.left_key, i, h).changed(); }
                            });
                            ui.label("右键");
                            changed |= ui.add(egui::TextEdit::singleline(&mut self.join.right_key).hint_text("列名；留空使用右表第一列")).changed();
                        });
                    }
                    if changed { self.join.preview = None; }
                    if ui.button("生成合并预览").clicked() {
                        self.join.preview = None; self.join.message.clear();
                        let left = left.clone(); let input = self.join.input.clone(); let format = self.join.format;
                        let delimiter = if self.join.tsv { b'\t' } else { b',' };
                        let mode = self.join.mode; let lk = self.join.left_key; let rk = self.join.right_key.clone();
                        let cancel = Arc::new(AtomicBool::new(false)); self.join.cancel = cancel.clone();
                        let (tx, rx) = mpsc::channel(); self.join.receiver = Some(rx); self.join.job.begin(); self.join.invalidated = false;
                        std::thread::spawn(move || {
                            let result = Dataset::parse(&input, format, delimiter).and_then(|right| combine(&left, &right, mode, lk, &rk, &cancel));
                            let _ = tx.send(result.map_err(|e| e.to_string()));
                        });
                        ctx.request_repaint();
                    }
                });
                if busy {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("正在生成预览…"); if ui.button("取消").clicked() { self.join.cancel.store(true, Ordering::Relaxed); self.join.job.cancelling(); } });
                }
                if let Some(preview) = &self.join.preview {
                    ui.label(&preview.report);
                    egui::ScrollArea::both().id_salt("join-result").max_height(180.0).min_scrolled_height(180.0).show(ui, |ui| {
                        egui::Grid::new("join-result-grid").striped(true).show(ui, |ui| {
                            for h in &preview.data.headers { ui.strong(h); } ui.end_row();
                            for row in preview.data.rows.iter().take(8) { for cell in row { let text = cell_text(cell); ui.label(text.chars().take(60).collect::<String>()).on_hover_text(text); } ui.end_row(); }
                        });
                    });
                    ui.small("预览最多 8 行。应用将替换当前数据表并重置筛选；可在列转换面板撤销最近一次操作。");
                    apply = ui.button("应用合并结果").clicked();
                }
                if !self.join.message.is_empty() { ui.label(&self.join.message); }
            });
        self.join.force_open = false;
        if apply && let Some(preview) = self.join.preview.take() {
            self.message = preview.report;
            self.replace_with_join(preview.data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_join_keeps_right_input_and_unapplied_result_without_a_job() {
        let mut state = DataState::default();
        state.input = "id,left\n1,A".into();
        state.dataset = Some(Dataset::parse(&state.input, DataFormat::Csv, b',').unwrap());
        state.join.input = "id,right\n1,B".into();
        state.join.preview = Some(
            combine(
                state.dataset.as_ref().unwrap(),
                &Dataset::parse(&state.join.input, DataFormat::Csv, b',').unwrap(),
                Mode::Left,
                0,
                "id",
                &AtomicBool::new(false),
            )
            .unwrap(),
        );
        let restored = DataState::restore(&state.snapshot().unwrap()).unwrap();
        assert_eq!(restored.join.input, state.join.input);
        assert_eq!(
            restored.join.preview.as_ref().unwrap().data,
            state.join.preview.as_ref().unwrap().data
        );
        assert_eq!(restored.join.preview.as_ref().unwrap().data.rows[0][3], "B");
        assert!(restored.join.receiver.is_none());
        assert_eq!(restored.join.job.phase, Phase::Idle);
        assert_eq!(restored.dataset.as_ref().unwrap().headers.len(), 2);
    }
    fn csv(s: &str) -> Dataset {
        Dataset::parse(s, DataFormat::Csv, b',').unwrap()
    }
    #[test]
    fn invalidation_waits_for_worker_and_never_applies_stale_result() {
        let mut state = DataState::default();
        let (tx, rx) = mpsc::channel();
        state.join.cancel = Arc::new(AtomicBool::new(false));
        state.join.receiver = Some(rx);
        state.join.job.begin();
        state.join.invalidate();
        assert_eq!(state.join.job.phase, Phase::Cancelling);
        tx.send(Ok(Preview {
            data: csv("id\n1"),
            report: "old".into(),
        }))
        .unwrap();
        state.poll();
        assert_eq!(state.join.job.phase, Phase::Cancelled);
        assert!(state.join.preview.is_none());
    }
    #[test]
    fn applying_join_can_be_undone_and_left_changes_invalidate_pending_result() {
        let original = csv("id,value\n1,left");
        let right = csv("id,extra\n1,right");
        let result = combine(
            &original,
            &right,
            Mode::Left,
            0,
            "id",
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut state = DataState {
            dataset: Some(original.clone()),
            input: "original input".into(),
            query: "left".into(),
            output: "stale export".into(),
            ..Default::default()
        };
        state.join.cancel = Arc::new(AtomicBool::new(false));
        let token = state.join.cancel.clone();
        state.replace_with_join(result.data);
        assert_eq!(state.dataset.as_ref().unwrap().headers.len(), 4);
        assert!(state.output.is_empty() && state.query.is_empty());
        assert_eq!(state.input, "original input");
        assert!(!token.load(Ordering::Relaxed));
        state.undo_transform();
        assert_eq!(state.dataset.unwrap(), original);
    }
    #[test]
    fn duplicate_keys_expand_stably_and_unmatched_left_rows_survive() {
        let l = csv("id,a\n1,L\n2,M\n1,N\n,empty");
        let r = csv("id,a\n1,X\n1,Y\n3,Z\n,blank");
        let p = combine(&l, &r, Mode::Left, 0, "id", &AtomicBool::new(false)).unwrap();
        assert_eq!(p.data.rows.len(), 6);
        assert_eq!(p.data.rows[0][3], "X");
        assert_eq!(p.data.rows[1][3], "Y");
        assert!(p.data.rows[2][2].is_null());
        assert_eq!(p.data.headers, vec!["id", "a", "id_right1", "a_right1"]);
        assert!(p.report.contains("未匹配左行 2"));
        assert!(p.report.contains("未匹配右行 2"));
        assert!(p.report.contains("左 1，右 1"));
        let inner = combine(&l, &r, Mode::Inner, 0, "id", &AtomicBool::new(false)).unwrap();
        assert_eq!(inner.data.rows.len(), 4);
    }
    #[test]
    fn append_reorders_columns_and_column_zip_requires_equal_rows() {
        let l = csv("a,b\n1,2");
        let r = csv("b,a\n4,3");
        let p = combine(&l, &r, Mode::Append, 0, "", &AtomicBool::new(false)).unwrap();
        assert_eq!(p.data.rows[1], vec![Value::from("3"), Value::from("4")]);
        assert!(
            combine(
                &l,
                &csv("a,b\n1,2\n3,4"),
                Mode::Columns,
                0,
                "",
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(
            combine(
                &l,
                &csv("a,c\n1,2"),
                Mode::Append,
                0,
                "",
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(
            combine(&l, &r, Mode::Columns, 0, "", &AtomicBool::new(false))
                .unwrap()
                .data
                .rows[0]
                .len(),
            4
        );
    }
    #[test]
    fn cancellation_limits_and_header_collisions_are_explicit() {
        let l = csv("id,id_right1\n1,x");
        let r = csv("id\n1");
        assert_eq!(
            combine(&l, &r, Mode::Left, 0, "id", &AtomicBool::new(false))
                .unwrap()
                .data
                .headers[2],
            "id_right2"
        );
        assert!(combine(&l, &r, Mode::Left, 0, "id", &AtomicBool::new(true)).is_err());
        let many = Dataset {
            headers: vec!["id".into()],
            rows: vec![vec![Value::from("1")]; 101],
        };
        assert!(
            combine(&many, &many, Mode::Inner, 0, "id", &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("10000")
        );
    }
    #[test]
    fn keys_do_not_coerce_types_or_match_empty_values() {
        assert_ne!(key(&Value::from(1)), key(&Value::from("1")));
        assert!(key(&Value::Null).is_none());
        assert!(key(&Value::from(" ")).is_none());
        let l = Dataset::parse(r#"[{"id":1}]"#, DataFormat::Json, b',').unwrap();
        let r = csv("id\n1");
        assert!(
            combine(&l, &r, Mode::Inner, 0, "id", &AtomicBool::new(false))
                .unwrap()
                .data
                .rows
                .is_empty()
        );
    }
}
