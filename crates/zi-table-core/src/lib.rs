//! Shared delimited/JSON datasets and native join semantics.
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};

pub mod join;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SharedRows<T>(Arc<Vec<T>>);

impl<T> Default for SharedRows<T> {
    fn default() -> Self {
        Self(Arc::new(Vec::new()))
    }
}

impl<T> From<Vec<T>> for SharedRows<T> {
    fn from(rows: Vec<T>) -> Self {
        Self(Arc::new(rows))
    }
}

impl<T> FromIterator<T> for SharedRows<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(Arc::new(iter.into_iter().collect()))
    }
}

impl<T> std::ops::Deref for SharedRows<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Clone> std::ops::DerefMut for SharedRows<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl<'a, T> IntoIterator for &'a SharedRows<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: Clone> IntoIterator for &'a mut SharedRows<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T: PartialEq> PartialEq<Vec<T>> for SharedRows<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.0.as_ref() == other
    }
}

pub const INPUT_LIMIT: usize = 2 * 1024 * 1024;
pub const ROW_LIMIT: usize = 10_000;
pub const COLUMN_LIMIT: usize = 128;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataFormat {
    #[default]
    Csv,
    Json,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub headers: Vec<String>,
    pub rows: SharedRows<Vec<Value>>,
}
impl Dataset {
    pub fn from_parts(
        headers: Vec<String>,
        rows: impl Into<SharedRows<Vec<Value>>>,
    ) -> Result<Self> {
        let data = Self {
            headers,
            rows: rows.into(),
        };
        data.validate_saved()?;
        Ok(data)
    }
    pub fn validate_saved(&self) -> Result<()> {
        anyhow::ensure!(
            !self.headers.is_empty() && self.headers.len() <= COLUMN_LIMIT,
            "保存表格的列数无效"
        );
        anyhow::ensure!(self.rows.len() <= ROW_LIMIT, "保存表格的行数超限");
        anyhow::ensure!(
            self.headers.iter().all(|h| !h.trim().is_empty())
                && self.headers.iter().collect::<BTreeSet<_>>().len() == self.headers.len(),
            "保存表格的列名无效"
        );
        anyhow::ensure!(
            self.rows.iter().all(|r| r.len() == self.headers.len()),
            "保存表格的行宽不一致"
        );
        Ok(())
    }
    pub fn parse(input: &str, format: DataFormat, delimiter: u8) -> Result<Self> {
        if input.len() > INPUT_LIMIT {
            bail!("输入最多 2 MiB");
        }
        if input.trim().is_empty() {
            bail!("请先粘贴内容或载入文件");
        }
        let data = match format {
            DataFormat::Csv => {
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(delimiter)
                    .from_reader(input.trim_start_matches('\u{feff}').as_bytes());
                let headers = reader
                    .headers()?
                    .iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if headers.iter().any(|s| s.trim().is_empty())
                    || headers.iter().collect::<BTreeSet<_>>().len() != headers.len()
                {
                    bail!("CSV 表头不能为空或重复；请先修正列名");
                }
                if headers.len() > COLUMN_LIMIT {
                    bail!("最多 128 列");
                }
                let mut rows = Vec::new();
                for (i, record) in reader.records().enumerate() {
                    if i >= ROW_LIMIT {
                        bail!("最多 10000 行数据");
                    }
                    let record =
                        record.with_context(|| format!("CSV 第 {} 条数据记录无效", i + 1))?;
                    rows.push(record.iter().map(|s| Value::String(s.to_owned())).collect());
                }
                Self {
                    headers,
                    rows: rows.into(),
                }
            }
            DataFormat::Json => {
                let value: Value = serde_json::from_str(input.trim_start_matches('\u{feff}'))?;
                if value.is_object() {
                    let data: Self = serde_json::from_str(input.trim_start_matches('\u{feff}'))
                        .context("JSON对象须为含headers/rows的列结构表格，不接受其他字段")?;
                    data.validate_saved()?;
                    return Ok(data);
                }
                let items = value
                    .as_array()
                    .ok_or_else(|| anyhow!("JSON 须为对象数组，例如 [{{\"name\":\"Zi\"}}]"))?;
                if items.is_empty() {
                    bail!("对象数组为空，无法推断列名");
                }
                if items.len() > ROW_LIMIT {
                    bail!("最多 10000 行数据");
                }
                let mut names = BTreeSet::new();
                for (i, item) in items.iter().enumerate() {
                    let object = item
                        .as_object()
                        .ok_or_else(|| anyhow!("第 {} 项不是对象", i + 1))?;
                    names.extend(object.keys().cloned());
                }
                let headers = names.into_iter().collect::<Vec<_>>();
                let rows: Vec<Vec<Value>> = items
                    .iter()
                    .map(|item| {
                        headers
                            .iter()
                            .map(|h| item.get(h).cloned().unwrap_or(Value::Null))
                            .collect()
                    })
                    .collect();
                Self {
                    headers,
                    rows: rows.into(),
                }
            }
        };
        if data.headers.is_empty() || data.headers.len() > COLUMN_LIMIT {
            bail!("数据须包含 1–128 列");
        }
        data.validate_saved()?;
        Ok(data)
    }
    pub fn export(&self, indices: &[usize], format: DataFormat, delimiter: u8) -> Result<String> {
        let out = match format {
            DataFormat::Json => {
                let rows = indices
                    .iter()
                    .map(|&i| {
                        self.headers
                            .iter()
                            .cloned()
                            .zip(self.rows[i].iter().cloned())
                            .collect::<serde_json::Map<_, _>>()
                    })
                    .collect::<Vec<_>>();
                serde_json::to_string_pretty(&rows)?
            }
            DataFormat::Csv => {
                let mut w = csv::WriterBuilder::new()
                    .delimiter(delimiter)
                    .from_writer(Vec::new());
                w.write_record(&self.headers)?;
                for &i in indices {
                    w.write_record(self.rows[i].iter().map(cell_text))?;
                }
                String::from_utf8(w.into_inner()?)?
            }
        };
        if out.len() > 8 * 1024 * 1024 {
            bail!("导出超过 8 MiB，请筛选后再试");
        }
        Ok(out)
    }
    pub fn view(&self, query: &str, sort: Option<usize>, descending: bool) -> Vec<usize> {
        let query = query.to_lowercase();
        let mut rows = (0..self.rows.len())
            .filter(|&i| {
                query.is_empty()
                    || self.rows[i]
                        .iter()
                        .any(|v| cell_text(v).to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        if let Some(col) = sort {
            rows.sort_by(|&a, &b| {
                let order = match (self.rows[a][col].as_f64(), self.rows[b][col].as_f64()) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    _ => cell_text(&self.rows[a][col]).cmp(&cell_text(&self.rows[b][col])),
                };
                if descending { order.reverse() } else { order }
            });
        }
        rows
    }
}
pub fn cell_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
