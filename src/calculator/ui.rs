use super::{Angle, Value, evaluate, valid_variable};
use eframe::egui::{self, RichText};
use std::collections::BTreeMap;

pub struct State {
    pub expression: String,
    pub(super) matrix_mode: bool,
    pub(super) plot_mode: bool,
    pub(super) date_mode: bool,
    pub(super) dates: super::dates::State,
    pub(super) plot: super::plot_ui::State,
    pub(super) matrix: super::matrix_ui::State,
    pub(super) degrees: bool,
    pub(super) variables: BTreeMap<String, Value>,
    pub(super) history: Vec<(String, Value)>,
    pub(super) message: String,
    pub(super) sheet_name: String,
    pub(super) files: super::worksheet_ui::Files,
    pub(super) examples_expanded: bool,
    #[cfg(feature = "ui-preview")]
    pub(super) input_rect: Option<(egui::Rect, egui::Rect)>,
    #[cfg(feature = "ui-preview")]
    pub preview_numeric_send: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    pub(super) preview_statistics_button: Option<(egui::Rect, egui::Rect)>,
}
impl Default for State {
    fn default() -> Self {
        let mut state = Self {
            expression: "0.1 + 0.2".into(),
            matrix_mode: false,
            plot_mode: false,
            date_mode: false,
            dates: Default::default(),
            plot: Default::default(),
            matrix: Default::default(),
            degrees: false,
            variables: BTreeMap::new(),
            history: Vec::new(),
            message: String::new(),
            sheet_name: "未命名工作表".into(),
            files: Default::default(),
            examples_expanded: false,
            #[cfg(feature = "ui-preview")]
            input_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_numeric_send: None,
            #[cfg(feature = "ui-preview")]
            preview_statistics_button: None,
        };
        state.files.baseline = Some(state.snapshot());
        state
    }
}
impl State {
    pub fn date_output_active(&self) -> bool {
        self.date_mode && self.dates.date_operation()
    }
    pub fn take_date_transfer(&mut self) -> Result<Option<(chrono::NaiveDate, String)>, String> {
        let snapshot = self.dates.take_transfer()?;
        if snapshot.is_some() && !self.date_output_active() {
            return Err("日期工作区已切换，请重新发送".into());
        }
        Ok(snapshot)
    }
    pub fn plot_active(&self) -> bool {
        self.plot_mode
    }
    pub fn plot_description(&self) -> Result<String, String> {
        self.plot.description(&self.variables, self.degrees)
    }
    pub fn plot_csv(&self) -> Result<&str, String> {
        self.plot.csv(&self.variables, self.degrees)
    }
    pub(super) fn preview(&self) -> Result<(Option<String>, Value), String> {
        let angle = if self.degrees {
            Angle::Degrees
        } else {
            Angle::Radians
        };
        if let Some((name, expression)) = self.expression.split_once('=') {
            let name = name.trim();
            if !valid_variable(name) {
                return Err("变量名需以英文字母开头，最多 32 字符；pi/e/ans 保留".into());
            }
            if !self.variables.contains_key(name) && self.variables.len() >= 64 {
                return Err("变量最多 64 个（含 ans）".into());
            }
            Ok((
                Some(name.into()),
                evaluate(expression.trim(), &self.variables, angle)?,
            ))
        } else {
            Ok((None, evaluate(&self.expression, &self.variables, angle)?))
        }
    }
    fn commit(&mut self) -> Result<(), String> {
        let (name, value) = self.preview()?;
        if let Some(name) = name {
            self.variables.insert(name, value);
        }
        self.variables.insert("ans".into(), value);
        self.history.insert(0, (self.expression.clone(), value));
        self.history.truncate(100);
        self.message = "已固定结果；ans 可引用上一次结果".into();
        Ok(())
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, version: &str) {
        ui.heading("全能计算器");
        if self.plot_mode || self.date_mode {
            ui.small(format!("v{version} · 独立工作区 · 主动保存工作表"));
            let status = if self.busy() {
                "后台任务中"
            } else if self.dirty() {
                "● 未保存修改"
            } else {
                "已保存 / 与基线一致"
            };
            egui::CollapsingHeader::new(format!("工作表管理 · {status}"))
                .id_salt("plot-worksheet-management")
                .show(ui, |ui| self.worksheet_ui(ui));
        } else {
            ui.label(format!(
                "v{version} · 开发中：精确表达式 / 科学函数 / 单位 / 矩阵 / 函数绘图 / 变量与历史"
            ));
            ui.small("精确模式采用 i128 有理数；科学函数为近似实数。可主动另存JSON工作表并恢复；默认只保留在内存。");
            self.worksheet_ui(ui);
        }
        ui.add_space(12.0);
        if !self.date_mode {
            ui.horizontal_wrapped(|ui| {
                ui.label("三角函数角度");
                ui.selectable_value(&mut self.degrees, false, "弧度 RAD");
                ui.selectable_value(&mut self.degrees, true, "角度 DEG");
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(
                    !self.matrix_mode && !self.plot_mode && !self.date_mode,
                    "表达式计算",
                )
                .clicked()
            {
                self.matrix_mode = false;
                self.plot_mode = false;
                self.date_mode = false;
            }
            if ui
                .selectable_label(self.matrix_mode && !self.plot_mode, "矩阵与数据统计")
                .clicked()
            {
                self.matrix_mode = true;
                self.plot_mode = false;
                self.date_mode = false;
            }
            if ui.selectable_label(self.plot_mode, "函数绘图").clicked() {
                self.matrix_mode = false;
                self.plot_mode = true;
                self.date_mode = false;
            }
            if ui
                .selectable_label(self.date_mode, "日期与工作日")
                .clicked()
            {
                self.date_mode = true;
                self.matrix_mode = false;
                self.plot_mode = false;
            }
        });
        if self.date_mode {
            self.dates.ui(ui);
            return;
        }
        if self.plot_mode {
            let files_busy = self.busy() && !self.plot.busy();
            self.plot.ui(ui, &self.variables, self.degrees, files_busy);
            return;
        }
        if self.matrix_mode {
            self.matrix.ui(ui, &self.variables, self.degrees);
            return;
        }
        let input = ui.add(
            egui::TextEdit::singleline(&mut self.expression)
                .char_limit(2048)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .hint_text("输入算式，Enter 固定结果；例如 price = 19.90"),
        );
        #[cfg(feature = "ui-preview")]
        {
            self.input_rect = Some((input.rect, ui.clip_rect()));
        }
        if self.expression.len() > 2048 {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "表达式超过 2048 字节；请缩短输入",
            );
        }
        let result = self.preview();
        egui::Frame::group(ui.style())
            .inner_margin(16.0)
            .show(ui, |ui| match &result {
                Ok((name, value)) => {
                    ui.label(RichText::new(value.display()).size(30.0).strong());
                    ui.small(match value {
                        Value::Exact(..) => "精确值 · 有限小数或最简分数",
                        Value::Approx(_) => "近似值 · 浮点计算，约 12 位显示精度",
                    });
                    if let Some(name) = name {
                        ui.small(format!("预览赋值：{name}；固定结果后才保存变量"));
                    }
                    if let Ok(n) = value.integer() {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(format!("HEX {n:#x}"));
                            ui.monospace(format!("OCT {n:#o}"));
                            ui.monospace(format!("BIN {n:#b}"));
                        });
                        ui.small("位运算采用有符号 128 位补码，左移溢出位丢弃");
                    }
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        let enter = (input.has_focus() || input.lost_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none());
        let mut commit = enter;
        ui.horizontal_wrapped(|ui| {
            commit |= ui
                .add_enabled(result.is_ok(), egui::Button::new("固定结果 · Enter"))
                .clicked();
            if ui
                .add_enabled(result.is_ok(), egui::Button::new("复制结果"))
                .clicked()
            {
                if let Ok((_, v)) = result {
                    ui.ctx().copy_text(v.display());
                }
            }
            if ui.button("复制算式").clicked() {
                ui.ctx().copy_text(self.expression.clone());
            }
            if ui.button("清空输入").clicked() {
                self.expression.clear();
                self.message.clear();
                input.request_focus();
            }
        });
        if commit {
            if let Err(error) = self.commit() {
                self.message = error;
            }
            if enter {
                input.request_focus();
            }
        }
        if !self.message.is_empty() {
            ui.small(&self.message);
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new("统计与组合 · 点选示例")
            .id_salt("calculator-statistics")
            .default_open(self.examples_expanded)
            .show(ui, |ui| {
                ui.small("1–64个值，用逗号分隔；可引用变量和分数。点选只填写算式，Enter才固定结果。");
                for (label, examples) in [
                    ("统计", &["sum(0.1,0.2,0.3)", "mean(1/3,2/3)", "median(5,1,2,4)"][..]),
                    ("离散程度", &["varp(1,2,3)", "vars(1,2,3)", "stdp(1,2,3)", "stds(1,2,3)"][..]),
                    ("组合与整数", &["fact(20)", "perm(5,3)", "comb(52,5)", "gcd(-48,18)", "lcm(12,18)"][..]),
                ] {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(label);
                        for example in examples {
                            let button = ui.button(*example);
                            #[cfg(feature = "ui-preview")]
                            if *example == "comb(52,5)" {
                                self.preview_statistics_button = Some((button.rect, ui.clip_rect()));
                            }
                            if button.clicked() {
                                self.expression = (*example).into();
                                self.message.clear();
                                input.request_focus();
                            }
                        }
                    });
                }
                ui.small("varp/stdp：总体，分母n；vars/stds：样本，分母n−1，至少2值。标准差为近似平方根。");
                ui.small("阶乘/排列组合：0≤r≤n≤10000，结果须能用i128精确表示。超限明确报错，不自动改近似。");
            });
        ui.collapsing("输入说明与示例",|ui| {
            ui.label("运算：+ - * / ^ % & | << >> ~；幂右结合，-2^2 = -4。输入需显式 *。");
            ui.label("函数：sin cos tan sqrt ln log10 exp abs floor ceil round min max xor；常量 pi/e。");
            ui.label("进制：0xff / 0b1010 / 0o17；科学计数法 1e-3；变量 price=19.90，后续 price*3。");
            ui.label("单位：mm/cm/m/km/in/ft/mi，mg/g/kg/lb，ms/s/min/h/day，B/KB/MB/GB/KiB/MiB/GiB，C/F/K。单位区分大小写。");
            ui.horizontal_wrapped(|ui| { for example in ["0.1+0.2","(128+64)*3","sin(30)","0xff & 0x0f","5 km -> m","25 C -> F","1 GiB -> MB","price = 19.90"] { if ui.button(example).clicked() { self.expression=example.into(); input.request_focus(); } } });
            ui.small("矩阵及线性方程见上方工作区；复数、非线性方程、原文件更新、任意精度与通用接力继续开发。");
        });
        ui.collapsing(format!("变量（{}）", self.variables.len()), |ui| {
            for (name, v) in &self.variables {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(name).clicked() {
                        self.expression = name.clone();
                        input.request_focus();
                    }
                    ui.monospace(v.display());
                });
            }
            if ui.button("清空所有变量").clicked() {
                self.variables.clear();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("工作表历史（{} / 100）", self.history.len()));
            if ui.button("清空历史").clicked() {
                self.history.clear();
            }
        });
        for (expression, value) in &self.history {
            ui.horizontal_wrapped(|ui| {
                if ui.button("复用").clicked() {
                    self.expression = expression.clone();
                    input.request_focus();
                }
                ui.monospace(expression);
                ui.label(format!("= {}", value.display()));
                if ui.small_button("复制").clicked() {
                    ui.ctx().copy_text(value.display());
                }
            });
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_matrix_fixture(&mut self) {
        self.matrix_mode = true;
        self.plot_mode = false;
        self.date_mode = false;
        self.matrix.preview_fixture();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_matrix_position(&self) -> egui::Pos2 {
        self.matrix.preview_position()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_matrix_check(&self) {
        self.matrix.preview_check();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.expression = "price = 19.90".into();
        self.commit().unwrap();
        self.expression = "price * 3".into();
        self.commit().unwrap();
        self.expression = "25 C -> F".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_input_position(&self) -> egui::Pos2 {
        let (rect, clip) = self.input_rect.expect("input rendered");
        assert!(clip.contains(rect.center()));
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_check(&self, phase: u8) {
        assert_eq!(
            self.history.len(),
            3,
            "expression={} message={}",
            self.expression,
            self.message
        );
        assert_eq!(self.variables["ans"].display(), "0.3");
        if phase == 1 {
            assert!(self.expression.contains("1/0"));
            assert!(self.preview().is_err());
        }
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_date_fixture(&mut self) {
        *self = Self::default();
        self.date_mode = true;
        self.dates.saved.operation = super::dates::Operation::WeekdayOffset;
        self.dates.saved.start = "2026-10-09".into();
        self.dates.saved.amount = "3".into();
        self.variables.insert("ans".into(), Value::Exact(7, 1));
        self.history = vec![("old".into(), Value::Exact(7, 1))];
    }
    pub fn preview_date_send_position(&self) -> egui::Pos2 {
        self.dates
            .send_rect
            .expect("date send button rendered")
            .center()
    }
    pub fn preview_date_snapshot(&self) -> (chrono::NaiveDate, String) {
        self.dates.date_snapshot().unwrap()
    }
    pub fn preview_date_position(&self) -> egui::Pos2 {
        self.dates
            .compute_rect
            .expect("date button rendered")
            .center()
    }
    pub fn preview_date_check(&mut self, phase: u8) {
        assert_eq!(self.variables["ans"], Value::Exact(7, 1));
        assert_eq!(self.history.len(), 1);
        match phase {
            0 => assert!(self.numeric_result().is_err()),
            1 => {
                assert_eq!(self.dates.current_text().unwrap(), "2026-10-14 · 星期三");
            }
            2 => {
                self.dates.saved.amount = "4".into();
                assert!(self.numeric_result().is_err());
                assert!(self.dates.current_text().is_err());
            }
            3 => {
                self.dates.saved.operation = super::dates::Operation::Difference;
                self.dates.saved.end = "2026-10-14".into();
            }
            4 => {
                assert_eq!(
                    self.numeric_result().unwrap().cells,
                    vec![Value::Exact(5, 1)]
                );
            }
            _ => panic!("unknown date preview phase"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_does_not_mutate_variables_and_errors_do_not_replace_ans_or_history() {
        let mut state = State {
            expression: "price = 19.90".into(),
            ..State::default()
        };
        assert_eq!(state.preview().unwrap().1.display(), "19.9");
        assert!(state.variables.is_empty());
        state.commit().unwrap();
        state.expression = "price*3".into();
        state.commit().unwrap();
        assert_eq!(state.variables["ans"].display(), "59.7");
        state.expression = "bad = 1/0".into();
        assert!(state.commit().is_err());
        assert!(!state.variables.contains_key("bad"));
        assert_eq!(state.history.len(), 2);
        assert_eq!(state.variables["ans"].display(), "59.7");
        state.expression = "ans+0.3".into();
        assert_eq!(state.preview().unwrap().1.display(), "60");
    }
}
