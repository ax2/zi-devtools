use super::{
    Value,
    plot::{self, Curve, Key, Output, Saved},
    plot_render::{self, Bounds},
};
use eframe::egui::{self, RichText};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

enum Completion {
    Sample(Result<Output, String>),
    Export(Result<PathBuf, String>),
}
struct Job {
    receiver: mpsc::Receiver<Completion>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub(super) struct State {
    pub saved: Saved,
    pub output: Option<Output>,
    pub view: Option<Bounds>,
    job: Option<Job>,
    message: String,
    error: bool,
    #[cfg(feature = "ui-preview")]
    pub controls: [Option<(egui::Rect, egui::Rect)>; 3],
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    fn fresh(&self, variables: &BTreeMap<String, Value>, degrees: bool) -> Result<&Output, String> {
        self.saved.validate()?;
        let output = self
            .output
            .as_ref()
            .ok_or("请先绘制函数，采样结果为近似值")?;
        if output.key != Key::new(&self.saved, variables, degrees) {
            return Err("公式、范围、精度、变量或角度已修改，请重新绘制".into());
        }
        Ok(output)
    }
    pub fn csv(&self, variables: &BTreeMap<String, Value>, degrees: bool) -> Result<&str, String> {
        Ok(&self.fresh(variables, degrees)?.csv)
    }
    pub fn description(
        &self,
        variables: &BTreeMap<String, Value>,
        degrees: bool,
    ) -> Result<String, String> {
        let output = self.fresh(variables, degrees)?;
        if output.csv.len() > 128 * 1024 {
            return Err(format!(
                "采样CSV {} KiB，超过接力128 KiB；可另存，或减少采样点",
                output.csv.len().div_ceil(1024)
            ));
        }
        Ok(format!(
            "{}条函数 · {}个x坐标 · 近似采样CSV",
            output.series.len(),
            output.x.len()
        ))
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else {
            return;
        };
        let completion = match job.receiver.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Completion::Sample(Err("绘图后台任务意外结束，旧图保留".into()))
            }
        };
        self.job = None;
        match completion {
            Completion::Sample(Ok(output)) => {
                self.output = Some(output);
                self.view = None;
                self.message = "已绘制启动时的公式与变量快照；不写入ans或历史".into();
                self.error = false;
            }
            Completion::Export(Ok(path)) => {
                self.message = format!("已另存：{}；未覆盖已有文件", path.display());
                self.error = false;
            }
            Completion::Sample(Err(e)) | Completion::Export(Err(e)) => {
                self.message = e;
                self.error = true;
            }
        }
    }
    pub fn start(
        &mut self,
        variables: &BTreeMap<String, Value>,
        degrees: bool,
        ctx: &egui::Context,
    ) {
        if self.busy() {
            return;
        }
        if let Err(e) = self.saved.validate() {
            self.message = e;
            self.error = true;
            return;
        }
        let key = Key::new(&self.saved, variables, degrees);
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.job = Some(Job {
            receiver,
            cancel: cancel.clone(),
        });
        self.error = false;
        self.message = "正在后台采样，可继续编辑；新修改不会被标作已计算".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Completion::Sample(plot::sample(key, &cancel)));
            ctx.request_repaint();
        });
    }
    fn export(&mut self, path: PathBuf, bytes: Vec<u8>, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.job = Some(Job {
            receiver,
            cancel: cancel.clone(),
        });
        self.message = "正在另存绘图结果；文件已存在则拒绝覆盖".into();
        self.error = false;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::local_files::save_new(&path, &bytes, &cancel)
                .map(|_| path)
                .map_err(|e| format!("另存失败：{e:#}"));
            let _ = tx.send(Completion::Export(result));
            ctx.request_repaint();
        });
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        variables: &BTreeMap<String, Value>,
        degrees: bool,
        files_busy: bool,
    ) {
        if let Err(error) = self.saved.validate() {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.small("y=f(x) · 最多4条 · x为临时采样坐标，不修改工作表变量 · 科学函数为近似值");
        let mut remove = None;
        let can_remove = self.saved.curves.len() > 1;
        for (index, curve) in self.saved.curves.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut curve.enabled,
                        RichText::new(format!("y{}", index + 1))
                            .color(plot_render::color(index, ui.visuals().dark_mode)),
                    );
                    let input = ui.add(
                        egui::TextEdit::singleline(&mut curve.expression)
                            .font(egui::TextStyle::Monospace)
                            .char_limit(2048)
                            .desired_width((ui.available_width() - 42.0).max(100.0))
                            .hint_text("例如 sin(x)、x^2、price*x"),
                    );
                    #[cfg(feature = "ui-preview")]
                    if index == 0 {
                        self.controls[0] = Some((input.rect, ui.clip_rect()));
                    }
                    #[cfg(not(feature = "ui-preview"))]
                    let _ = input;
                    if ui
                        .add_enabled(can_remove, egui::Button::new("×").small())
                        .on_hover_text("移除此函数；至少保留一条")
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            });
        }
        if let Some(i) = remove
            && self.saved.curves.len() > 1
        {
            self.saved.curves.remove(i);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.saved.curves.len() < 4, egui::Button::new("+ 添加函数"))
                .clicked()
            {
                self.saved.curves.push(Curve {
                    expression: "cos(x)".into(),
                    enabled: true,
                });
            }
            for (label, expression) in [
                ("正弦", "sin(x)"),
                ("抛物线", "x^2"),
                ("倒数", "1/x"),
                ("平方根", "sqrt(x)"),
            ] {
                if ui.small_button(label).clicked() {
                    self.saved.curves[0].expression = expression.into();
                }
            }
        });
        let axis = (self.saved.auto_y, self.saved.y_min, self.saved.y_max);
        ui.horizontal_wrapped(|ui| {
            ui.label("采样x范围");
            ui.add(egui::DragValue::new(&mut self.saved.x_min).speed(0.1));
            ui.label("至");
            ui.add(egui::DragValue::new(&mut self.saved.x_max).speed(0.1));
            egui::ComboBox::from_id_salt("plot-sample-count")
                .selected_text(format!("{}点 / 函数", self.saved.points))
                .show_ui(ui, |ui| {
                    for points in [129, 257, 513, 1025, 2049] {
                        ui.selectable_value(&mut self.saved.points, points, format!("{points}点"));
                    }
                });
            ui.checkbox(&mut self.saved.auto_y, "自动Y范围");
        });
        if !self.saved.auto_y {
            ui.horizontal_wrapped(|ui| {
                ui.label("显示Y范围");
                ui.add(egui::DragValue::new(&mut self.saved.y_min).speed(0.1));
                ui.label("至");
                ui.add(egui::DragValue::new(&mut self.saved.y_max).speed(0.1));
            });
        }
        if axis != (self.saved.auto_y, self.saved.y_min, self.saved.y_max) {
            self.view = None;
        }
        let fresh = self.fresh(variables, degrees).is_ok();
        ui.horizontal_wrapped(|ui| {
            let draw = ui.add_enabled(
                !self.busy() && !files_busy,
                egui::Button::new("绘制 / 更新函数"),
            );
            #[cfg(feature = "ui-preview")]
            {
                self.controls[1] = Some((draw.rect, ui.clip_rect()));
            }
            if draw.clicked() {
                self.start(variables, degrees, ui.ctx());
            }
            if let Some(job) = &self.job
                && ui.button("取消后台任务").clicked()
            {
                job.cancel.store(true, Ordering::Relaxed);
                self.message = "正在取消；已完成的文件另存不会撤销".into();
            }
            if ui
                .add_enabled(fresh, egui::Button::new("适应曲线"))
                .clicked()
            {
                self.view = None;
            }
            if ui
                .add_enabled(fresh, egui::Button::new("复制采样CSV"))
                .clicked()
                && let Ok(csv) = self.csv(variables, degrees)
            {
                ui.ctx().copy_text(csv.to_owned());
            }
            #[cfg(windows)]
            {
                if ui
                    .add_enabled(
                        fresh && !self.busy() && !files_busy,
                        egui::Button::new("另存CSV…"),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("CSV采样表", &["csv"])
                        .set_file_name("function-samples.csv")
                        .save_file()
                    && let Ok(csv) = self.csv(variables, degrees)
                {
                    self.export(path, csv.as_bytes().to_vec(), ui.ctx());
                }
                if ui
                    .add_enabled(
                        fresh && !self.busy() && !files_busy,
                        egui::Button::new("另存SVG…"),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("SVG函数图", &["svg"])
                        .set_file_name("function-plot.svg")
                        .save_file()
                    && let Some(output) = &self.output
                {
                    let view = self
                        .view
                        .unwrap_or_else(|| Bounds::fit(output, &self.saved));
                    self.export(path, plot_render::svg(output, view).into_bytes(), ui.ctx());
                }
            }
        });
        if !self.message.is_empty() {
            ui.colored_label(
                if self.error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().weak_text_color()
                },
                &self.message,
            );
        }
        if let Some(output) = &self.output {
            if !fresh {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "旧图已过期：公式、采样范围、精度、变量或角度改变，请更新后再复制或发送",
                );
            }
            if self.saved.auto_y && output.y_range().is_none() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "自动Y超出±1e12显示范围，当前使用手动Y范围；完整数值仍保留在采样表",
                );
            }
            let view = self
                .view
                .get_or_insert_with(|| Bounds::fit(output, &self.saved));
            let (_rect, _clip) = plot_render::chart(ui, output, view);
            #[cfg(feature = "ui-preview")]
            {
                self.controls[2] = Some((_rect, _clip));
            }
            ui.small(
                "拖动平移 · 图内滚轮缩放 · 悬停读数 · 适应曲线重置视图；视窗变化不改变采样范围",
            );
            for series in &output.series {
                ui.colored_label(
                    plot_render::color(series.index, ui.visuals().dark_mode),
                    format!(
                        "y{} · {}个无效采样 / {}段未连线 · {}",
                        series.index + 1,
                        series.invalid,
                        series.connect.iter().filter(|v| !**v).count(),
                        series.expression
                    ),
                );
                if series.invalid > 0 {
                    ui.small(&series.diagnostic);
                }
            }
        } else {
            egui::Frame::group(ui.style())
                .inner_margin(18.0)
                .show(ui, |ui| {
                    ui.label("输入函数与范围，点击绘制；例如同时比较 sin(x) 与 cos(x)");
                });
        }
        ui.small("采样/中点探测是近似显示，不能证明连续性或找全奇点；无效定义域留空，不跨已探测断点连线。函数用显式乘号；赋值和单位箭头不属于绘图公式。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_svg_export_is_atomic_no_overwrite_and_keeps_current_plot() {
        let root = std::env::temp_dir().join(format!("zi-plot-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("plot.svg");
        let mut state = State::default();
        let vars = BTreeMap::new();
        state.output = Some(
            plot::sample(
                Key::new(&state.saved, &vars, false),
                &AtomicBool::new(false),
            )
            .unwrap(),
        );
        let output = state.output.as_ref().unwrap();
        let bytes = plot_render::svg(output, Bounds::fit(output, &state.saved)).into_bytes();
        let ctx = egui::Context::default();
        let poll = |state: &mut State| {
            let start = std::time::Instant::now();
            while state.busy() {
                assert!(start.elapsed() < std::time::Duration::from_secs(5));
                state.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        state.export(path.clone(), bytes.clone(), &ctx);
        poll(&mut state);
        assert!(!state.error);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(state.csv(&vars, false).is_ok());
        state.export(path.clone(), b"must not replace".to_vec(), &ctx);
        poll(&mut state);
        assert!(state.error);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn later_edits_and_angle_changes_keep_snapshot_stale_and_block_exports() {
        let mut state = State::default();
        let vars = BTreeMap::new();
        let key = Key::new(&state.saved, &vars, false);
        let (tx, receiver) = mpsc::channel();
        state.job = Some(Job {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        state.saved.curves[0].expression = "cos(x)".into();
        tx.send(Completion::Sample(plot::sample(
            key,
            &AtomicBool::new(false),
        )))
        .unwrap();
        state.poll(&egui::Context::default());
        assert!(state.output.is_some());
        assert!(state.csv(&vars, false).is_err());
        assert!(!state.busy());
        state.saved.curves[0].expression = "sin(x)".into();
        assert!(state.csv(&vars, false).is_ok());
        assert!(state.csv(&vars, true).is_err());
        let mut changed = vars;
        changed.insert("price".into(), Value::Exact(2, 1));
        assert!(state.csv(&changed, false).is_err());
    }
    #[test]
    fn cancelled_sampling_keeps_old_output_and_svg_escapes_formula() {
        let mut state = State::default();
        let vars = BTreeMap::new();
        let key = Key::new(&state.saved, &vars, false);
        state.output = Some(plot::sample(key, &AtomicBool::new(false)).unwrap());
        let before = state.output.as_ref().unwrap().csv.clone();
        let (tx, receiver) = mpsc::channel();
        state.job = Some(Job {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        tx.send(Completion::Sample(Err("cancelled".into())))
            .unwrap();
        state.poll(&egui::Context::default());
        assert_eq!(state.output.as_ref().unwrap().csv, before);
        let output = state.output.as_mut().unwrap();
        output.series[0].expression = "<script>&\"'".into();
        let svg = plot_render::svg(output, Bounds::fit(output, &state.saved));
        assert!(!svg.contains("<script>"));
        assert!(svg.contains("&lt;script&gt;&amp;"));
        assert!(!svg.contains("NaN"));
    }
}
