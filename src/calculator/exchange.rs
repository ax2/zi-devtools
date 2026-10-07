//! Bounded numeric tables for explicit, in-memory tool handoff.
use super::{State, Value, matrix::Matrix};
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation {
    Typed,
    Text,
    Approximate,
}
impl Representation {
    pub const ALL: [Self; 3] = [Self::Typed, Self::Text, Self::Approximate];
    pub fn label(self) -> &'static str {
        match self {
            Self::Typed => "保留数值类型 · JSON",
            Self::Text => "完整数值文本",
            Self::Approximate => "近似数字 · f64",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatrixSlot {
    #[default]
    A,
    B,
}
impl MatrixSlot {
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "矩阵A",
            Self::B => "矩阵B",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumericTable {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Value>,
}
impl NumericTable {
    pub fn new(rows: usize, cols: usize, cells: Vec<Value>) -> Result<Self, String> {
        Matrix::new(rows, cols, cells.clone())?;
        // Use the strict wire validator as well; callers cannot inject noncanonical fractions.
        for value in &cells {
            let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
            let _: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(Self { rows, cols, cells })
    }
    pub fn json(&self, representation: Representation) -> Result<String, String> {
        Self::new(self.rows, self.cols, self.cells.clone())?;
        let mut rows = Vec::new();
        for cells in self.cells.chunks(self.cols) {
            let mut row = serde_json::Map::new();
            for (col, value) in cells.iter().enumerate() {
                let v = match representation {
                    Representation::Typed => {
                        serde_json::to_value(value).map_err(|e| e.to_string())?
                    }
                    Representation::Text => serde_json::Value::String(literal(*value)),
                    Representation::Approximate => serde_json::Value::Number(
                        serde_json::Number::from_f64(value.float())
                            .ok_or("数值不能转换为有限f64")?,
                    ),
                };
                row.insert(format!("c{}", col + 1), v);
            }
            rows.push(row);
        }
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }
    pub fn delimited(
        &self,
        representation: Representation,
        delimiter: u8,
    ) -> Result<String, String> {
        if representation == Representation::Typed {
            return Err("保留类型需使用JSON对象数组；CSV/TSV不能保留数值类型".into());
        }
        Self::new(self.rows, self.cols, self.cells.clone())?;
        let mut writer = csv::WriterBuilder::new()
            .delimiter(delimiter)
            .from_writer(Vec::new());
        writer
            .write_record((1..=self.cols).map(|c| format!("c{c}")))
            .map_err(|e| e.to_string())?;
        for row in self.cells.chunks(self.cols) {
            writer
                .write_record(row.iter().map(|v| {
                    if representation == Representation::Text {
                        literal(*v)
                    } else {
                        format!("{:e}", v.float())
                    }
                }))
                .map_err(|e| e.to_string())?;
        }
        String::from_utf8(writer.into_inner().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn read_json(text: &str) -> Result<Self, String> {
        if text.len() > 128 * 1024 {
            return Err("类型表格最多128KiB".into());
        }
        let rows: Vec<Row> = serde_json::from_str(text)
            .map_err(|e| format!("需要c1..cN规范数值类型对象数组：{e}"))?;
        let cols = rows.first().map_or(0, |r| r.0.len());
        if !(1..=8).contains(&rows.len())
            || !(1..=8).contains(&cols)
            || rows.iter().any(|r| r.0.len() != cols)
        {
            return Err("类型表格需为完整的1–8行、1–8列矩形".into());
        }
        Self::new(
            rows.len(),
            cols,
            rows.into_iter().flat_map(|r| r.0).collect(),
        )
    }
    pub fn description(&self) -> String {
        let approx = self
            .cells
            .iter()
            .filter(|v| matches!(v, Value::Approx(_)))
            .count();
        format!(
            "{}×{} · {}个精确值 / {}个近似值",
            self.rows,
            self.cols,
            self.cells.len() - approx,
            approx
        )
    }
}

// Unlike serde_json::Value, reject duplicate JSON column names before information is lost.
struct Row(Vec<Value>);
impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Row;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("columns c1..c8 with strict typed numbers")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Row, M::Error> {
                let mut cells = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    let col = key
                        .strip_prefix('c')
                        .and_then(|s| s.parse::<usize>().ok())
                        .filter(|c| (1..=8).contains(c) && key == format!("c{c}"))
                        .ok_or_else(|| serde::de::Error::custom("列名需为c1..c8"))?;
                    if cells.insert(col, map.next_value::<Value>()?).is_some() {
                        return Err(serde::de::Error::custom("重复列名"));
                    }
                }
                if cells.keys().copied().ne(1..=cells.len()) {
                    return Err(serde::de::Error::custom("列名不连续"));
                }
                Ok(Row(cells.into_values().collect()))
            }
        }
        d.deserialize_map(V)
    }
}

pub(super) fn literal(value: Value) -> String {
    match value {
        Value::Exact(n, 1) => n.to_string(),
        Value::Exact(n, d) => format!("{n}/{d}"),
        Value::Approx(x) => format!("≈ {x:e}"),
    }
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistic_state(&mut self, ready: bool) {
        self.matrix_mode = true;
        self.plot_mode = false;
        self.variables.insert("ans".into(), Value::Exact(7, 1));
        self.history = vec![("old history".into(), Value::Exact(7, 1))];
        if ready {
            self.matrix.preview_statistic_fixture(true);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistic_operation_position(&self, index: usize) -> eframe::egui::Pos2 {
        self.matrix.preview_statistic_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistic_state_check(&self, computed: bool) {
        assert_eq!(self.variables.len(), 1);
        assert_eq!(self.variables["ans"], Value::Exact(7, 1));
        assert_eq!(
            self.history,
            vec![("old history".into(), Value::Exact(7, 1))]
        );
        if computed {
            self.matrix.preview_statistic_check(true);
        }
    }
    pub fn numeric_description(&self) -> Result<String, String> {
        if self.matrix_mode {
            self.matrix
                .numeric_description(&self.variables, self.degrees)
        } else {
            let value = self.preview()?.1;
            Ok(format!(
                "1×1 · {}",
                if matches!(value, Value::Approx(_)) {
                    "近似值"
                } else {
                    "精确值"
                }
            ))
        }
    }
    pub fn numeric_result(&self) -> Result<NumericTable, String> {
        if self.matrix_mode {
            self.matrix.numeric_result(&self.variables, self.degrees)
        } else {
            NumericTable::new(1, 1, vec![self.preview()?.1])
        }
    }
    pub fn receive_numeric(&mut self, text: &str) -> Result<(), String> {
        self.receive_numeric_into(text, MatrixSlot::A)
    }
    pub fn receive_numeric_into(&mut self, text: &str, slot: MatrixSlot) -> Result<(), String> {
        self.receive_numeric_with_policy(text, slot, false)
    }
    pub fn receive_numeric_with_policy(
        &mut self,
        text: &str,
        slot: MatrixSlot,
        allow_replace: bool,
    ) -> Result<(), String> {
        let table = NumericTable::read_json(text)?;
        if self.busy() || self.awaiting_restore() || (self.dirty() && !allow_replace) {
            return Err(
                "计算器有未保存工作、待读取确认或后台任务；请先保存、取消读取或放弃修改，再接收"
                    .into(),
            );
        }
        self.matrix.receive_numeric_into(table, slot);
        self.matrix_mode = true;
        self.plot_mode = false;
        self.message = format!(
            "类型表格已填入{}；保留另一矩阵、算式和变量，尚未计算",
            slot.label()
        );
        Ok(())
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_mapping_check(&self) {
        assert!(self.dirty());
        assert!(!self.variables.contains_key("preserve_price"));
        self.matrix.preview_mapping_check();
    }
    pub fn preview_boundary_check(&self) {
        assert!(self.dirty());
        assert!(!self.variables.contains_key("preserve_price"));
        self.matrix.preview_boundary_check();
    }
    pub fn preview_numeric_fixture(&mut self) {
        *self = Self::default();
        self.matrix_mode = true;
        self.matrix.preview_numeric_fixture();
    }
    pub fn preview_numeric_compute(&mut self) {
        self.matrix.preview_numeric_compute();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lossless_types_and_explicit_lossy_conversion() {
        let t = NumericTable::new(
            1,
            3,
            vec![
                Value::Exact(i128::MAX, 1),
                Value::Exact(1, 3),
                Value::Approx(f64::from_bits(1)),
            ],
        )
        .unwrap();
        assert_eq!(
            NumericTable::read_json(&t.json(Representation::Typed).unwrap()).unwrap(),
            t
        );
        let strings = t.json(Representation::Text).unwrap();
        assert!(strings.contains("1/3") && strings.contains(&i128::MAX.to_string()));
        assert!(NumericTable::read_json(&strings).is_err());
        assert!(t.delimited(Representation::Typed, b'\t').is_err());
        assert!(
            t.delimited(Representation::Text, b'\t')
                .unwrap()
                .starts_with("c1\tc2\tc3")
        );
        let lossy: serde_json::Value =
            serde_json::from_str(&t.json(Representation::Approximate).unwrap()).unwrap();
        assert!(lossy[0]["c1"].is_number());
    }
    #[test]
    fn reject_duplicates_shapes_plain_numbers_and_noncanonical_values() {
        let cell = r#"{"kind":"exact","numerator":"1","denominator":"3"}"#;
        for text in [
            format!(r#"[{{"c1":{cell},"c1":{cell}}}]"#),
            format!(r#"[{{"c2":{cell}}}]"#),
            "[{\"c1\":0.5}]".into(),
            "[]".into(),
            "[{}]".into(),
            format!(r#"[{{"c01":{cell}}}]"#),
            format!(r#"[{{"c1":{cell}}},{{"c1":{cell},"c2":{cell}}}]"#),
        ] {
            assert!(NumericTable::read_json(&text).is_err(), "{text}");
        }
        assert!(NumericTable::new(1, 1, vec![Value::Exact(2, 6)]).is_err());
        assert!(NumericTable::new(1, 1, vec![Value::Approx(f64::NAN)]).is_err());
        assert!(NumericTable::read_json(&" ".repeat(128 * 1024 + 1)).is_err());
    }
    #[test]
    fn explicit_replacement_only_changes_selected_matrix() {
        let text = NumericTable::new(
            3,
            1,
            vec![Value::Exact(2, 1), Value::Exact(3, 1), Value::Exact(4, 1)],
        )
        .unwrap()
        .json(Representation::Typed)
        .unwrap();
        let mut state = State {
            expression: "price=19.90".into(),
            ..Default::default()
        };
        assert!(state.receive_numeric_into(&text, MatrixSlot::B).is_err());
        state
            .receive_numeric_with_policy(&text, MatrixSlot::B, true)
            .unwrap();
        assert_eq!(state.expression, "price=19.90");
        assert!(!state.variables.contains_key("price"));
        assert!(state.dirty());
        assert!(state.numeric_result().is_err());
    }
    #[test]
    fn receiving_protects_work_and_preserves_other_inputs() {
        let table = NumericTable::new(1, 1, vec![Value::Exact(1, 3)])
            .unwrap()
            .json(Representation::Typed)
            .unwrap();
        let mut state = State::default();
        let expression = state.expression.clone();
        state.receive_numeric(&table).unwrap();
        assert_eq!(state.expression, expression);
        assert!(state.dirty() && state.matrix_mode);
        assert!(state.receive_numeric(&table).is_err());
        assert!(state.numeric_result().is_err());
    }
}
