//! Bounded real root finding for a user-supplied bracketed interval.
use super::{Angle, Value};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) use zi_matrix_core::root::{ResultValue, solve};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    pub formula: String,
    pub lower: String,
    pub upper: String,
    pub tolerance: String,
    pub iterations: String,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            formula: "x^2 - 2".into(),
            lower: "1".into(),
            upper: "2".into(),
            tolerance: "1e-10".into(),
            iterations: "100".into(),
        }
    }
}

impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        if self.formula.len() > 2048
            || self.lower.len() > 64
            || self.upper.len() > 64
            || self.tolerance.len() > 64
            || self.iterations.len() > 8
        {
            return Err("非线性求解器输入超过工作表限制".into());
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct State {
    pub saved: Saved,
    pub(super) last: Option<(Saved, BTreeMap<String, Value>, Angle, ResultValue)>,
    message: String,
}

impl State {
    pub(super) fn current_result(
        &self,
        variables: &BTreeMap<String, Value>,
        angle: Angle,
    ) -> Option<ResultValue> {
        self.last
            .as_ref()
            .filter(|(saved, vars, saved_angle, _)| {
                saved == &self.saved && vars == variables && *saved_angle == angle
            })
            .map(|(_, _, _, result)| *result)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, variables: &BTreeMap<String, Value>, angle: Angle) {
        ui.heading("非线性方程 · 区间二分");
        ui.label("求 f(x)=0；输入含 x 的表达式和异号端点，再手动计算。最多 200 次迭代。");
        ui.horizontal(|ui| {
            ui.label("f(x)");
            ui.add(egui::TextEdit::singleline(&mut self.saved.formula).desired_width(320.0));
        });
        ui.horizontal_wrapped(|ui| {
            for (label, value) in [
                ("左端", &mut self.saved.lower),
                ("右端", &mut self.saved.upper),
                ("区间宽度容差", &mut self.saved.tolerance),
                ("迭代上限", &mut self.saved.iterations),
            ] {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(value).desired_width(100.0));
            }
        });
        if ui.button("计算区间根").clicked() {
            let parsed = (|| -> Result<(f64, f64, f64, usize), String> {
                self.saved.validate()?;
                let parse = |text: &str, name: &str| {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| format!("{name}必须是有限数字"))
                };
                let lower = parse(&self.saved.lower, "左端点")?;
                let upper = parse(&self.saved.upper, "右端点")?;
                let tolerance = parse(&self.saved.tolerance, "容差")?;
                let iterations = self
                    .saved
                    .iterations
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "迭代上限必须是整数".to_owned())?;
                Ok((lower, upper, tolerance, iterations))
            })();
            self.last = None;
            self.message = match parsed.and_then(|(lower, upper, tolerance, iterations)| {
                solve(
                    &self.saved.formula,
                    lower,
                    upper,
                    tolerance,
                    iterations,
                    variables,
                    angle,
                )
            }) {
                Ok(result) => {
                    self.last = Some((self.saved.clone(), variables.clone(), angle, result));
                    String::new()
                }
                Err(error) => error,
            };
        }
        let current = self.current_result(variables, angle);
        if let Some(error) = (!self.message.is_empty()).then_some(self.message.as_str()) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(result) = current {
            egui::Frame::group(ui.style())
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.label(if result.converged {
                        "区间已收敛（候选点，请检查残差）"
                    } else {
                        "达到迭代上限（尚未收敛）"
                    });
                    ui.monospace(format!("x ≈ {:.17e}", result.root));
                    ui.monospace(format!(
                        "|f(x)| ≈ {:.6e} · {} 次迭代",
                        result.residual.abs(),
                        result.iterations
                    ));
                    if ui
                        .add_enabled(result.converged, egui::Button::new("复制近似根"))
                        .clicked()
                    {
                        ui.ctx().copy_text(format!("{:.17e}", result.root));
                    }
                });
        } else if self.last.is_some() {
            ui.weak("函数、区间、精度、变量或角度已变化；请重新计算。旧根不会复制。");
        }
        ui.small("方法要求区间端点函数值异号，只能检测该区间内的变号根；偶重根可能无法发现。函数在区间内需连续，算法不会证明连续性；残差会单独显示。结果为 f64 近似值，不写入 ans 或历史。");
        if !variables.is_empty() {
            ui.small(format!(
                "可引用当前计算器变量：{}；变量变化后旧结果失效。",
                variables.keys().cloned().collect::<Vec<_>>().join("、")
            ));
        }
    }
}
