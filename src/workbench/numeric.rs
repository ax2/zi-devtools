use super::*;
use crate::calculator::{Value as Number, exchange::NumericTable};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum TextMode {
    #[default]
    Exact,
    Approximate,
}

fn number(value: &Value, mode: TextMode) -> Result<Number> {
    match value {
        Value::Object(_) => serde_json::from_value(value.clone()).map_err(Into::into),
        Value::Number(n) => {
            if let Some(n) = n.as_i64() {
                Ok(Number::Exact(i128::from(n), 1))
            } else if let Some(n) = n.as_u64() {
                Ok(Number::Exact(i128::from(n), 1))
            } else {
                let n = n
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| anyhow!("不是有限数字"))?;
                Ok(Number::Approx(n))
            }
        }
        Value::String(text) => {
            anyhow::ensure!(text.len() <= 512, "数字文本最多512字节");
            let text = text.trim();
            let text = if mode == TextMode::Approximate {
                text.strip_prefix('≈').unwrap_or(text).trim()
            } else {
                text
            };
            static LITERAL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let regex = LITERAL.get_or_init(|| {
                regex::Regex::new(r"^[+-]?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$").unwrap()
            });
            let parts: Vec<_> = text.split('/').map(str::trim).collect();
            anyhow::ensure!(
                (1..=2).contains(&parts.len()) && parts.iter().all(|s| regex.is_match(s)),
                "需要十进制/科学计数法/分数字面量；不执行算式、函数或赋值"
            );
            if mode == TextMode::Exact {
                let expression = if parts.len() == 2 {
                    format!("({})/({})", parts[0], parts[1])
                } else {
                    parts[0].into()
                };
                crate::calculator::evaluate(
                    &expression,
                    &Default::default(),
                    crate::calculator::Angle::Radians,
                )
                .map_err(anyhow::Error::msg)
            } else {
                let left = parts[0].parse::<f64>()?;
                let result = if parts.len() == 2 {
                    left / parts[1].parse::<f64>()?
                } else {
                    left
                };
                anyhow::ensure!(result.is_finite(), "近似转换不是有限值，检查溢出或零分母");
                Ok(Number::Approx(result))
            }
        }
        _ => bail!("空值、布尔、数组不是数值；请修正或选择其他范围"),
    }
}

pub fn select(
    data: &Dataset,
    view: &[usize],
    start: usize,
    count: usize,
    columns: &[usize],
    mode: TextMode,
) -> Result<NumericTable> {
    anyhow::ensure!(
        (1..=8).contains(&count) && (1..=8).contains(&columns.len()),
        "选择1–8行和1–8列"
    );
    anyhow::ensure!(
        columns.iter().copied().collect::<BTreeSet<_>>().len() == columns.len()
            && columns.iter().all(|c| *c < data.headers.len()),
        "列选择无效或重复"
    );
    let end = start
        .checked_add(count)
        .filter(|end| *end <= view.len())
        .ok_or_else(|| anyhow!("行范围超出当前筛选视图"))?;
    let mut values = Vec::new();
    for (index, &row) in view[start..end].iter().enumerate() {
        let cells = data.rows.get(row).ok_or_else(|| anyhow!("视图行无效"))?;
        for &col in columns {
            let value = cells.get(col).ok_or_else(|| anyhow!("单元格缺失"))?;
            values.push(number(value, mode).with_context(|| {
                format!("视图第{}行 · 列 {}", start + index + 1, data.headers[col])
            })?);
        }
    }
    NumericTable::new(count, columns.len(), values).map_err(anyhow::Error::msg)
}

#[derive(Default)]
pub(super) struct Selector {
    headers: Vec<String>,
    columns: Vec<usize>,
    start: usize,
    count: usize,
    mode: TextMode,
    request: Option<(String, NumericTable)>,
    error: String,
    #[cfg(feature = "ui-preview")]
    reveal: bool,
    #[cfg(feature = "ui-preview")]
    send_rect: Option<egui::Rect>,
}
impl Selector {
    pub fn take(&mut self) -> Option<(String, NumericTable)> {
        self.request.take()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, data: Option<&Dataset>, view: &[usize], busy: bool) {
        let Some(data) = data else {
            return;
        };
        if self.headers != data.headers {
            self.headers = data.headers.clone();
            self.columns.clear();
            self.start = 1;
            self.count = view.len().clamp(1, 8);
            self.error.clear();
        }
        #[cfg(feature = "ui-preview")]
        let open = self.reveal;
        #[cfg(not(feature = "ui-preview"))]
        let open = false;
        egui::CollapsingHeader::new("选取数值区域 → 计算器").default_open(open).show(ui,|ui| {
            ui.small("按当前已解析的筛选/排序视图选行。列按选择顺序组成矩阵，可调整；最多8×8。发送前不改变数据或计算器。");
            ui.horizontal_wrapped(|ui| {
                ui.label("视图开始行");ui.add(egui::DragValue::new(&mut self.start).range(1..=view.len().max(1)));
                ui.label("行数");ui.add(egui::DragValue::new(&mut self.count).range(1..=8));
                ui.selectable_value(&mut self.mode,TextMode::Exact,"精确数字文本");ui.selectable_value(&mut self.mode,TextMode::Approximate,"文本转近似f64");
            });
            ui.small("CSV字符串仅接受数字/分数字面量；JSON在整数解析范围内保留整数及数值类型；JSON小数/超范围数字已是近似值。近似转换可能舍入、下溢或丢失大整数精度，不执行表达式。");
            egui::ScrollArea::vertical().id_salt("numeric-columns").max_height(130.0).show(ui,|ui| {
                for (index,name) in data.headers.iter().enumerate() {
                    let mut checked=self.columns.contains(&index);
                    if ui.add_enabled(checked||self.columns.len()<8,egui::Checkbox::new(&mut checked,name)).changed() {
                        if checked {self.columns.push(index);}else{self.columns.retain(|i|*i!=index);}self.error.clear();
                    }
                }
            });
            let mut move_column=None;
            for pos in 0..self.columns.len() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("矩阵第{}列 ← {}",pos+1,data.headers[self.columns[pos]]));
                    if ui.add_enabled(pos>0,egui::Button::new("前移")).clicked(){move_column=Some((pos,pos-1));}
                    if ui.add_enabled(pos+1<self.columns.len(),egui::Button::new("后移")).clicked(){move_column=Some((pos,pos+1));}
                });
            }
            if let Some((a,b))=move_column {self.columns.swap(a,b);}
            let response=ui.add_enabled(!busy&&!view.is_empty()&&!self.columns.is_empty(),egui::Button::new("预览发送到计算器…"));
            #[cfg(feature="ui-preview")] {self.send_rect=Some(response.rect);}
            if response.clicked() {
                match select(data,view,self.start.saturating_sub(1),self.count,&self.columns,self.mode) {
                    Ok(table)=>{let names=self.columns.iter().map(|i|data.headers[*i].as_str()).collect::<Vec<_>>().join(" → ");
                        self.request=Some((format!("数据视图第{}–{}行 · 列 {names} · {}",self.start,self.start+self.count-1,if self.mode==TextMode::Exact{"精确文本模式"}else{"文本近似模式"}),table));self.error.clear();}
                    Err(e)=>self.error=format!("{e:#}"),
                }
            }
            if !self.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&self.error);}
        });
    }
}

#[cfg(feature = "ui-preview")]
impl DataState {
    pub fn preview_mapping_fixture(&mut self) {
        let text = "客户,收入,成本\n甲,7,2\n乙,1/3,3\n丙,5,4";
        let data = Dataset::parse(text, DataFormat::Csv, b',').unwrap();
        self.input = text.into();
        self.format = DataFormat::Csv;
        self.query.clear();
        self.sort = Some(1);
        self.descending = true;
        self.visible = data.view(&self.query, self.sort, self.descending);
        self.numeric_selector = Selector {
            headers: data.headers.clone(),
            columns: vec![2, 1],
            start: 2,
            count: 2,
            reveal: true,
            ..Default::default()
        };
        self.dataset = Some(data);
        self.output = "preserved export".into();
    }
    pub fn preview_mapping_request(&self) -> (String, NumericTable) {
        let t = select(
            self.dataset.as_ref().unwrap(),
            &self.visible,
            1,
            2,
            &[2, 1],
            TextMode::Exact,
        )
        .unwrap();
        ("数据视图2–3行 · 成本 → 收入".into(), t)
    }
    pub fn preview_mapping_position(&self) -> egui::Pos2 {
        self.numeric_selector.send_rect.unwrap().center()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_range_and_column_order_keep_exact_values_without_executing() {
        let data = Dataset::parse(
            "name,a,b\none,1/3,9\ntwo,0.1,8\nthree,170141183460469231731687303715884105727,7",
            DataFormat::Csv,
            b',',
        )
        .unwrap();
        let t = select(&data, &[2, 0, 1], 0, 2, &[2, 1], TextMode::Exact).unwrap();
        assert_eq!(
            t.cells,
            vec![
                Number::Exact(7, 1),
                Number::Exact(i128::MAX, 1),
                Number::Exact(9, 1),
                Number::Exact(1, 3)
            ]
        );
        for (rows, cols) in [
            (vec![99], vec![1]),
            (vec![0], vec![1, 1]),
            (vec![0], vec![99]),
            (vec![0], vec![0]),
        ] {
            assert!(select(&data, &rows, 0, 1, &cols, TextMode::Exact).is_err());
        }
        assert!(select(&data, &[0], 0, 2, &[1], TextMode::Exact).is_err());
        for text in ["1+2", "sin(30)", "x=1", "1/0", "", "NaN"] {
            assert!(number(&Value::String(text.into()), TextMode::Exact).is_err());
        }
    }
    #[test]
    fn decimal_json_is_approximate_and_csv_approximation_is_explicit() {
        let data=Dataset::parse(r#"[{"integer":12,"decimal":0.1,"fraction":{"kind":"exact","numerator":"1","denominator":"3"}}]"#,DataFormat::Json,b',').unwrap();
        let t = select(&data, &[0], 0, 1, &[0, 1, 2], TextMode::Exact).unwrap();
        assert!(matches!(t.cells[0], Number::Approx(_)));
        assert_eq!(t.cells[1], Number::Exact(1, 3));
        assert_eq!(t.cells[2], Number::Exact(12, 1));
        let tiny = Value::String("≈ 5e-324".into());
        assert!(number(&tiny, TextMode::Exact).is_err());
        assert!(
            matches!(number(&tiny,TextMode::Approximate).unwrap(),Number::Approx(x) if x.to_bits()==1)
        );
        for text in ["1e9999", "1/0", "sin(1)"] {
            assert!(number(&Value::String(text.into()), TextMode::Approximate).is_err());
        }
        for v in [Value::Null, Value::Bool(true), serde_json::json!([1, 2])] {
            assert!(number(&v, TextMode::Exact).is_err());
        }
    }
}
