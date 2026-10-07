use super::{
    Angle, Value, evaluate,
    matrix::{Matrix, Operation},
};
use eframe::egui;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    rows: usize,
    cols: usize,
    cells: Vec<String>,
    paste: String,
    #[serde(default = "empty_typed")]
    typed: Vec<Option<Value>>,
}
fn empty_typed() -> Vec<Option<Value>> {
    vec![None; 64]
}
impl Default for Input {
    fn default() -> Self {
        Self {
            rows: 3,
            cols: 3,
            cells: vec!["0".into(); 64],
            paste: String::new(),
            typed: empty_typed(),
        }
    }
}
impl Input {
    fn from_tsv(text: &str) -> Result<Self, String> {
        if text.len() > 16 * 1024 {
            return Err("TSV内容最多16KiB".into());
        }
        let lines: Vec<_> = text.trim_end_matches(['\r', '\n']).lines().collect();
        if lines.is_empty() || lines.len() > 8 {
            return Err("TSV需为1–8行".into());
        }
        let cols = lines[0].split('\t').count();
        if !(1..=8).contains(&cols) {
            return Err("TSV需为1–8列，以制表符分列".into());
        }
        let mut result = Self {
            rows: lines.len(),
            cols,
            ..Self::default()
        };
        for (r, line) in lines.iter().enumerate() {
            let cells: Vec<_> = line.split('\t').collect();
            if cells.len() != cols {
                return Err(format!("第{}行列数不一致", r + 1));
            }
            for (c, cell) in cells.iter().enumerate() {
                let text = cell.trim();
                if text.is_empty() || text.len() > 512 {
                    return Err(format!("({}, {})不能为空且最多512字节", r + 1, c + 1));
                }
                result.cells[r * 8 + c] = text.into();
            }
        }
        Ok(result)
    }
    fn parse(
        &self,
        name: &str,
        variables: &BTreeMap<String, Value>,
        angle: Angle,
    ) -> Result<Matrix, String> {
        let mut cells = Vec::with_capacity(self.rows * self.cols);
        for r in 0..self.rows {
            for c in 0..self.cols {
                let text = &self.cells[r * 8 + c];
                if text.len() > 512 {
                    return Err(format!("{name}({}, {}) 超过512字节", r + 1, c + 1));
                }
                cells.push(
                    self.typed[r * 8 + c]
                        .map(Ok)
                        .unwrap_or_else(|| evaluate(text, variables, angle))
                        .map_err(|e| format!("{name}({}, {}): {e}", r + 1, c + 1))?,
                );
            }
        }
        Matrix::new(self.rows, self.cols, cells)
    }
    fn bytes(&self) -> usize {
        (0..self.rows)
            .flat_map(|r| (0..self.cols).map(move |c| self.cells[r * 8 + c].len()))
            .sum()
    }
    fn ui(&mut self, ui: &mut egui::Ui, name: &str) -> bool {
        let mut changed = false;
        if self.typed.iter().any(Option::is_some) {
            ui.small("已接收数值类型字面量；修改单格转回表达式，清零/单位阵/TSV会清除原类型。≈表示保留的近似值。");
        }
        egui::Frame::group(ui.style()).show(ui,|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("矩阵 {name}"));
                ui.label("行"); changed |= ui.add(egui::DragValue::new(&mut self.rows).range(1..=8)).changed();
                ui.label("列"); changed |= ui.add(egui::DragValue::new(&mut self.cols).range(1..=8)).changed();
                if ui.small_button("清零").clicked() { self.cells.fill("0".into()); self.typed.fill(None); changed=true; }
                if ui.add_enabled(self.rows==self.cols,egui::Button::new("单位阵")).clicked() {
                    self.cells.fill("0".into()); self.typed.fill(None);
                    for i in 0..self.rows { self.cells[i*8+i]="1".into(); }
                    changed=true;
                }
            });
            egui::CollapsingHeader::new("批量粘贴 TSV").id_salt(("matrix-paste",name)).show(ui,|ui| {
                ui.small("制表符分列、换行分行，支持单元格表达式；只替换此矩阵，确认前不改变输入。");
                ui.add(egui::TextEdit::multiline(&mut self.paste).desired_rows(3).desired_width(f32::INFINITY).char_limit(16*1024).font(egui::TextStyle::Monospace));
                match Self::from_tsv(&self.paste) {
                    Ok(candidate) => {
                        if ui.button(format!("确认应用 {} × {}",candidate.rows,candidate.cols)).clicked() {
                            self.rows=candidate.rows;self.cols=candidate.cols;self.cells=candidate.cells;self.typed=candidate.typed;changed=true;
                        }
                    }
                    Err(error) if !self.paste.is_empty() => {ui.colored_label(ui.visuals().error_fg_color,error);}
                    _=>{}
                }
            });
            egui::ScrollArea::horizontal().id_salt(("matrix-input",name)).show(ui,|ui| {
                egui::Grid::new(("matrix-grid",name)).spacing([6.0,6.0]).min_row_height(26.0).show(ui,|ui| {
                    ui.spacing_mut().interact_size.y=26.0;
                    ui.small("行 / 列"); for c in 0..self.cols { ui.small((c+1).to_string()); } ui.end_row();
                    for r in 0..self.rows {
                        ui.small((r+1).to_string());
                        for c in 0..self.cols {
                            let index=r*8+c;
                            let typed=self.typed[index].is_some();
                            let response=ui.add(egui::TextEdit::singleline(&mut self.cells[index]).id_salt((name,r,c)).desired_width(84.0).char_limit(512).font(egui::TextStyle::Monospace)).on_hover_text(if typed { "已保留原数值类型；修改此格会转回表达式。近似值编辑时请删除≈，改为科学函数表达式。" } else { "可输入表达式和现有变量，如 1/3、sin(30)、price；不执行赋值" });
                            response.clone().on_hover_ui(|ui| {ui.strong("完整单元格");ui.monospace(&self.cells[index]);if typed {ui.small("保留原数值类型；修改后作为普通表达式解析。");}});
                            if response.changed() { self.typed[index]=None; changed=true; }
                        }
                        ui.end_row();
                    }
                });
            });
        });
        changed
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    a: Input,
    b: Input,
    operation: Operation,
}
impl Saved {
    pub(super) fn uses_statistics(&self) -> bool {
        self.operation.statistic().is_some()
    }
    pub fn validate(&self) -> Result<(), String> {
        for input in [&self.a, &self.b] {
            if !(1..=8).contains(&input.rows)
                || !(1..=8).contains(&input.cols)
                || input.typed.len() != 64
                || input
                    .typed
                    .iter()
                    .zip(&input.cells)
                    .any(|(v, s)| v.is_some_and(|v| super::exchange::literal(v) != *s))
                || input.cells.len() != 64
                || input.cells.iter().any(|s| s.len() > 512)
                || input.paste.len() > 16 * 1024
            {
                return Err("工作表矩阵维度、64格内容或粘贴草稿超出限制".into());
            }
        }
        Ok(())
    }
    pub fn describe(&self) -> String {
        format!(
            "A {}×{} · B {}×{} · {}",
            self.a.rows,
            self.a.cols,
            self.b.rows,
            self.b.cols,
            self.operation.label()
        )
    }
}

struct ResultSnapshot {
    value: Matrix,
    revision: u64,
    variables: BTreeMap<String, Value>,
    degrees: bool,
    operation: Operation,
}

pub(super) struct State {
    a: Input,
    b: Input,
    operation: Operation,
    revision: u64,
    result: Option<ResultSnapshot>,
    error: String,
    #[cfg(feature = "ui-preview")]
    compute_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    compute_count: u32,
    #[cfg(feature = "ui-preview")]
    operation_rects: [Option<(egui::Rect, egui::Rect)>; 2],
}
impl Default for State {
    fn default() -> Self {
        let mut a = Input::default();
        for i in 0..3 {
            a.cells[i * 8 + i] = "1".into();
        }
        Self {
            a,
            b: Input {
                cols: 1,
                ..Input::default()
            },
            operation: Operation::Solve,
            revision: 0,
            result: None,
            error: String::new(),
            #[cfg(feature = "ui-preview")]
            compute_rect: None,
            #[cfg(feature = "ui-preview")]
            compute_count: 0,
            #[cfg(feature = "ui-preview")]
            operation_rects: [None; 2],
        }
    }
}
impl State {
    pub(super) fn numeric_description(
        &self,
        variables: &BTreeMap<String, Value>,
        degrees: bool,
    ) -> Result<String, String> {
        if !self.current(variables, degrees) {
            return Err("矩阵无结果或结果已过期；请先重新计算".into());
        }
        let value = &self.result.as_ref().ok_or("请先计算矩阵")?.value;
        Ok(format!(
            "{}×{} · {}",
            value.rows,
            value.cols,
            if value.approximate() {
                "含近似值"
            } else {
                "精确值"
            }
        ))
    }
    pub(super) fn numeric_result(
        &self,
        variables: &BTreeMap<String, Value>,
        degrees: bool,
    ) -> Result<super::exchange::NumericTable, String> {
        if !self.current(variables, degrees) {
            return Err("矩阵无结果或结果已过期；请先重新计算".into());
        }
        let value = &self.result.as_ref().ok_or("请先计算矩阵")?.value;
        super::exchange::NumericTable::new(value.rows, value.cols, value.cells.clone())
    }
    #[cfg(any(test, feature = "ui-preview"))]
    pub(super) fn receive_numeric(&mut self, table: super::exchange::NumericTable) {
        self.receive_numeric_into(table, super::exchange::MatrixSlot::A);
    }
    pub(super) fn receive_numeric_into(
        &mut self,
        table: super::exchange::NumericTable,
        slot: super::exchange::MatrixSlot,
    ) {
        let mut input = Input {
            rows: table.rows,
            cols: table.cols,
            ..Input::default()
        };
        for r in 0..table.rows {
            for c in 0..table.cols {
                let value = table.cells[r * table.cols + c];
                let index = r * 8 + c;
                input.cells[index] = super::exchange::literal(value);
                input.typed[index] = Some(value);
            }
        }
        match slot {
            super::exchange::MatrixSlot::A => self.a = input,
            super::exchange::MatrixSlot::B => self.b = input,
        };
        self.revision += 1;
        self.result = None;
        self.error.clear();
    }
    pub(super) fn snapshot(&self) -> Saved {
        Saved {
            a: self.a.clone(),
            b: self.b.clone(),
            operation: self.operation,
        }
    }
    pub(super) fn matches(&self, saved: &Saved) -> bool {
        self.a == saved.a && self.b == saved.b && self.operation == saved.operation
    }
    pub(super) fn restore(&mut self, saved: Saved) {
        self.a = saved.a;
        self.b = saved.b;
        self.operation = saved.operation;
        self.revision += 1;
        self.result = None;
        self.error.clear();
    }
    fn compute(
        &mut self,
        variables: &BTreeMap<String, Value>,
        degrees: bool,
    ) -> Result<(), String> {
        self.result = None;
        self.error.clear();
        if self.a.bytes()
            + if self.operation.needs_b() {
                self.b.bytes()
            } else {
                0
            }
            > 16 * 1024
        {
            return Err("参与计算的单元格内容总计最多16KiB".into());
        }
        let angle = if degrees {
            Angle::Degrees
        } else {
            Angle::Radians
        };
        let a = self.a.parse("A", variables, angle)?;
        let b = if self.operation.needs_b() {
            Some(self.b.parse("B", variables, angle)?)
        } else {
            None
        };
        let result = a.apply(self.operation, b.as_ref())?;
        self.result = Some(ResultSnapshot {
            value: result,
            revision: self.revision,
            variables: variables.clone(),
            degrees,
            operation: self.operation,
        });
        #[cfg(feature = "ui-preview")]
        {
            self.compute_count += 1;
        }
        Ok(())
    }
    fn current(&self, variables: &BTreeMap<String, Value>, degrees: bool) -> bool {
        self.result.as_ref().is_some_and(|result| {
            result.revision == self.revision
                && &result.variables == variables
                && result.degrees == degrees
                && result.operation == self.operation
        })
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, variables: &BTreeMap<String, Value>, degrees: bool) {
        ui.label("矩阵、线性方程与数值区域统计");
        ui.small("每轴 1–8；单元格可输入表达式及现有变量。显式计算，不改写 ans 或变量。");
        ui.horizontal_wrapped(|ui| {
            ui.label("运算");
            let picker = egui::ComboBox::from_id_salt("calculator-matrix-operation")
                .height(420.0)
                .selected_text(self.operation.label())
                .show_ui(ui, |ui| {
                    ui.strong("矩阵与线性方程");
                    for op in Operation::ALL {
                        if op == Operation::Sum {
                            ui.separator();
                            ui.strong("数值区域统计 · 仅A");
                        }
                        let option = ui.selectable_value(&mut self.operation, op, op.label());
                        #[cfg(feature = "ui-preview")]
                        if op == Operation::Mean {
                            self.operation_rects[1] = Some((option.rect, ui.clip_rect()));
                        }
                        if option.changed() {
                            self.error.clear();
                        }
                    }
                });
            #[cfg(feature = "ui-preview")]
            {
                self.operation_rects[0] = Some((picker.response.rect, ui.clip_rect()));
            }
            #[cfg(not(feature = "ui-preview"))]
            let _ = picker;
        });
        if self.operation.statistic().is_some() {
            ui.small(format!("统计A当前{}×{}区域的全部{}个值；0计入，错误/空值不跳过，B不参与。样本至少2值；结果不写ans或变量。",self.a.rows,self.a.cols,self.a.rows*self.a.cols));
        }
        ui.small(match self.operation {
            Operation::Multiply => "A 的列数需等于 B 的行数，结果为 A行 × B列。",
            Operation::Solve => {
                "A 为方阵，B 同行；B 可有多列，分别求解各组右端。奇异时不返回假解。"
            }
            Operation::Add | Operation::Subtract => "A 与 B 的行列数需相同。",
            Operation::Transpose => "行列互换，适用于矩形矩阵。",
            op if op.statistic().is_some() => {
                "结果为1×1数值，可继续发送到工具；精确输入保持有理数，标准差或混合近似值标近似。"
            }
            _ => "A 需为方阵；精确运算超出 i128 范围会报错。",
        });
        let current = self.current(variables, degrees);
        let mut compute = false;
        let mut copy = false;
        ui.horizontal_wrapped(|ui| {
            let button = ui.button(if self.operation.statistic().is_some() {
                "计算区域统计"
            } else {
                "计算矩阵结果"
            });
            #[cfg(feature = "ui-preview")]
            {
                self.compute_rect = Some(button.rect);
            }
            compute = button.clicked();
            copy = ui
                .add_enabled(current, egui::Button::new("复制结果 TSV"))
                .clicked();
            if ui.small_button("载入方程示例").clicked() {
                self.fixture();
            }
        });
        if self.operation.statistic().is_some() {
            self.result_ui(ui, variables, degrees);
        }
        if self.a.ui(ui, "A") {
            self.revision += 1;
            self.error.clear();
        }
        if self.operation.needs_b() && self.b.ui(ui, "B") {
            self.revision += 1;
            self.error.clear();
        }
        if compute && let Err(e) = self.compute(variables, degrees) {
            self.error = e;
        }
        if copy
            && self.current(variables, degrees)
            && let Some(result) = &self.result
        {
            ui.ctx().copy_text(result.value.tsv());
        }
        if self.operation.statistic().is_none() {
            self.result_ui(ui, variables, degrees);
        }
    }
    fn result_ui(&self, ui: &mut egui::Ui, variables: &BTreeMap<String, Value>, degrees: bool) {
        if !self.error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &self.error);
        }
        if let Some(snapshot) = &self.result {
            let result = &snapshot.value;
            if !self.current(variables, degrees) {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "输入、操作、变量或角度已改变，下面是上次结果；重新计算后才能复制。",
                );
            }
            egui::Frame::group(ui.style()).inner_margin(12.0).show(ui,|ui| {
                ui.strong(format!("{} · {} × {}",snapshot.operation.label(),result.rows,result.cols));
                if result.approximate() {
                    ui.label(if snapshot.operation.statistic().is_some(){"≈ 近似统计 · f64受舍入影响；标准差为近似平方根。"}else{"≈ 近似结果 · 使用浮点运算；未估计条件数，不保证病态矩阵误差。"});
                } else {ui.label("精确结果 · 有理数运算，无自动降精度");}
                egui::ScrollArea::horizontal().id_salt("matrix-result-scroll").show(ui,|ui| {
                    egui::Grid::new("matrix-result-grid").spacing([18.0,8.0]).show(ui,|ui| {
                        for row in result.cells.chunks(result.cols) { for value in row {ui.monospace(value.display());} ui.end_row(); }
                    });
                });
                ui.small("TSV 用制表符分列，可粘贴到表格；近似标记不写入数值，复制前请留意精度说明。");
            });
        }
    }
    fn fixture(&mut self) {
        self.a = Input::default();
        self.b = Input {
            cols: 2,
            ..Input::default()
        };
        for (i, v) in ["0", "2", "1", "1", "1", "0", "2", "0", "1"]
            .iter()
            .enumerate()
        {
            self.a.cells[i / 3 * 8 + i % 3] = (*v).into();
        }
        for (i, v) in ["5", "7", "3", "4", "4", "6"].iter().enumerate() {
            self.b.cells[i / 2 * 8 + i % 2] = (*v).into();
        }
        self.operation = Operation::Solve;
        self.revision += 1;
        self.error.clear();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_mapping_check(&self) {
        assert_eq!(self.a, State::default().a);
        assert!(self.result.is_none());
        assert_eq!((self.b.rows, self.b.cols), (2, 2));
        assert_eq!(self.b.typed[0], Some(Value::Exact(4, 1)));
        assert_eq!(self.b.typed[1], Some(Value::Exact(5, 1)));
        assert_eq!(self.b.typed[8], Some(Value::Exact(3, 1)));
        assert_eq!(self.b.typed[9], Some(Value::Exact(1, 3)));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_boundary_check(&self) {
        assert_eq!(self.a, State::default().a);
        assert!(self.result.is_none());
        assert_eq!((self.b.rows, self.b.cols), (8, 8));
        let order = [1, 0, 2, 3, 4, 5, 6, 7];
        for r in 0..8 {
            for (c, &original) in order.iter().enumerate() {
                let expected = if r == 0 && original == 2 {
                    Value::Exact(1, 3)
                } else if r == 7 && original == 7 {
                    Value::Exact(i128::MAX, 1)
                } else {
                    Value::Exact((r * 100 + original + 1) as i128, 1)
                };
                assert_eq!(self.b.typed[r * 8 + c], Some(expected));
            }
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_numeric_fixture(&mut self) {
        self.receive_numeric(
            super::exchange::NumericTable::new(
                2,
                2,
                vec![
                    Value::Exact(1, 3),
                    Value::Exact(i128::MAX, 1),
                    Value::Approx(f64::from_bits(1)),
                    Value::Approx(-0.0),
                ],
            )
            .unwrap(),
        );
        self.operation = Operation::Transpose;
        self.compute(&BTreeMap::new(), false).unwrap();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_numeric_compute(&mut self) {
        self.operation = Operation::Transpose;
        self.compute(&BTreeMap::new(), false).unwrap();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.fixture();
        self.compute(&BTreeMap::new(), false).unwrap();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_position(&self) -> egui::Pos2 {
        self.compute_rect.expect("matrix compute rendered").center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistic_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = self.operation_rects[if index == 2 { 1 } else { index }]
            .expect("operation control rendered");
        if index == 2 {
            return egui::pos2(rect.center().x, clip.center().y);
        }
        assert!(
            clip.contains(rect.center()),
            "operation control clipped {rect:?} {clip:?}"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistic_fixture(&mut self, ready: bool) {
        self.receive_numeric(
            super::exchange::NumericTable::new(
                8,
                8,
                (1..=64).map(|n| Value::Exact(n, 1)).collect(),
            )
            .unwrap(),
        );
        self.operation = Operation::Mean;
        if ready {
            self.compute(&BTreeMap::from([("ans".into(), Value::Exact(7, 1))]), false)
                .unwrap();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistic_check(&self, computed: bool) {
        assert_eq!(self.operation, Operation::Mean);
        assert_eq!(self.b, Self::default().b);
        for i in 0..64 {
            assert_eq!(self.a.typed[i], Some(Value::Exact((i + 1) as i128, 1)));
        }
        if computed {
            assert_eq!(
                self.result.as_ref().unwrap().value.cells,
                vec![Value::Exact(65, 2)]
            );
            assert_eq!(self.compute_count, 1);
        } else {
            assert!(self.result.is_none());
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_check(&self) {
        assert!(
            self.compute_count >= 2,
            "native calculate button must execute after fixture"
        );
        let result = &self
            .result
            .as_ref()
            .expect("actual compute button clicked")
            .value;
        assert_eq!(result.tsv(), "1.25\t1.75\n1.75\t2.25\n1.5\t2.5");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_statistics_preserve_types_ignore_b_and_restore_without_execution() {
        let mut state = State::default();
        state.receive_numeric(
            super::super::exchange::NumericTable::new(
                8,
                8,
                (1..=64).map(|n| Value::Exact(n, 1)).collect(),
            )
            .unwrap(),
        );
        state.b.cells[0] = "1/0".into();
        let b = state.b.clone();
        let a = state.a.clone();
        let vars = BTreeMap::from([("ans".into(), Value::Exact(7, 1))]);
        state.operation = Operation::Mean;
        assert!(state.numeric_result(&vars, false).is_err());
        state.compute(&vars, false).unwrap();
        assert_eq!(
            state.numeric_result(&vars, false).unwrap().cells,
            vec![Value::Exact(65, 2)]
        );
        assert_eq!(state.a, a);
        assert_eq!(state.b, b);
        assert_eq!(vars["ans"], Value::Exact(7, 1));
        let saved = state.snapshot();
        assert!(saved.uses_statistics());
        saved.validate().unwrap();
        state.restore(saved);
        assert!(state.result.is_none());
        state.compute(&vars, false).unwrap();
        state.operation = Operation::SampleVariance;
        assert!(!state.current(&vars, false));
        state.a.rows = 1;
        state.a.cols = 1;
        assert!(state.compute(&vars, false).is_err());
        assert!(state.result.is_none());
    }
    #[test]
    fn variable_and_angle_changes_invalidate_result_and_error_removes_previous_solution() {
        let mut state = State::default();
        state.fixture();
        let mut vars = BTreeMap::new();
        state.a.cells[8] = "coefficient".into();
        vars.insert("coefficient".into(), Value::Exact(1, 1));
        state.compute(&vars, false).unwrap();
        assert!(state.current(&vars, false));
        vars.insert("coefficient".into(), Value::Exact(2, 1));
        assert!(!state.current(&vars, false));
        assert!(!state.current(&BTreeMap::new(), true));
        state.a.cells[0] = "1/0".into();
        assert!(state.compute(&vars, false).is_err());
        assert!(state.result.is_none());
    }
    #[test]
    fn receive_b_preserves_a_and_requires_manual_recalculation() {
        let mut state = State::default();
        let a = state.a.clone();
        state.receive_numeric_into(
            super::super::exchange::NumericTable::new(
                3,
                1,
                vec![Value::Exact(2, 1), Value::Exact(3, 1), Value::Exact(4, 1)],
            )
            .unwrap(),
            super::super::exchange::MatrixSlot::B,
        );
        assert_eq!(state.a, a);
        assert!(state.result.is_none());
        state.compute(&BTreeMap::new(), false).unwrap();
        assert_eq!(
            state.result.as_ref().unwrap().value.cells,
            vec![Value::Exact(2, 1), Value::Exact(3, 1), Value::Exact(4, 1)]
        );
    }
    #[test]
    fn typed_cells_survive_compute_storage_and_explicit_edit() {
        let values = vec![
            Value::Exact(1, 3),
            Value::Approx(f64::from_bits(1)),
            Value::Approx(-0.0),
        ];
        let mut state = State::default();
        state.receive_numeric(
            super::super::exchange::NumericTable::new(1, 3, values.clone()).unwrap(),
        );
        state.operation = Operation::Transpose;
        state.compute(&BTreeMap::new(), false).unwrap();
        let result = state.numeric_result(&BTreeMap::new(), false).unwrap();
        assert_eq!(result.cells, values);
        assert!(matches!(result.cells[2],Value::Approx(x) if x.to_bits()==(-0.0f64).to_bits()));
        let bytes = serde_json::to_vec(&state.snapshot()).unwrap();
        let saved: Saved = serde_json::from_slice(&bytes).unwrap();
        saved.validate().unwrap();
        state.restore(saved);
        assert!(state.result.is_none());
        state.compute(&BTreeMap::new(), false).unwrap();
        assert!(
            matches!(state.result.as_ref().unwrap().value.cells[1],Value::Approx(x) if x.to_bits()==1)
        );
        state.a.cells[0] = "1/2".into();
        state.a.typed[0] = None;
        state.revision += 1;
        assert!(state.numeric_result(&BTreeMap::new(), false).is_err());
        state.compute(&BTreeMap::new(), false).unwrap();
        assert_eq!(
            state.result.as_ref().unwrap().value.cells[0],
            Value::Exact(1, 2)
        );
        let mut bad = state.snapshot();
        bad.a.cells[1] = "wrong display".into();
        assert!(bad.validate().is_err());
    }
    #[test]
    fn bulk_tsv_preserves_formulas_and_rejects_empty_ragged_or_oversized_data() {
        let input = Input::from_tsv("1/3\tprice\r\n2\tsin(30)\r\n").unwrap();
        assert_eq!((input.rows, input.cols), (2, 2));
        assert_eq!(input.cells[0], "1/3");
        assert_eq!(input.cells[9], "sin(30)");
        for text in ["", "1\t\n2\t3", "1\t2\n3", "\t1", "1,2\n3\t4"] {
            assert!(Input::from_tsv(text).is_err(), "{text:?}");
        }
        assert!(Input::from_tsv(&"1\n".repeat(9)).is_err());
        assert!(Input::from_tsv(&["1"; 9].join("\t")).is_err());
        assert!(Input::from_tsv(&"中".repeat(200)).is_err());
    }
    #[test]
    fn restored_draft_keeps_input_but_clears_old_derived_result_without_evaluation() {
        let mut state = State::default();
        state.fixture();
        state.compute(&BTreeMap::new(), false).unwrap();
        assert!(state.result.is_some());
        let mut saved = state.snapshot();
        saved.a.cells[0] = "unbound = 10".into();
        saved.a.paste = "unfinished\t".into();
        saved.validate().unwrap();
        state.restore(saved.clone());
        assert!(state.result.is_none());
        assert!(state.matches(&saved));
        assert!(state.compute(&BTreeMap::new(), false).is_err());
    }
    #[test]
    fn bounds_and_invalid_cells_report_coordinates() {
        let mut state = State::default();
        state.fixture();
        state.a.cells[9] = "unknown".into();
        assert!(
            state
                .compute(&BTreeMap::new(), false)
                .unwrap_err()
                .contains("A(2, 2)")
        );
        state.a.cells[9] = "1".repeat(513);
        assert!(state.compute(&BTreeMap::new(), false).is_err());
        state.a.rows = 8;
        state.a.cols = 8;
        state.a.cells.fill("1".repeat(512));
        assert!(
            state
                .compute(&BTreeMap::new(), false)
                .unwrap_err()
                .contains("16KiB")
        );
    }
}
