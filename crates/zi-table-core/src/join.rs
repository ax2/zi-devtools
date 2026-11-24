use super::{COLUMN_LIMIT, Dataset, ROW_LIMIT};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Left,
    Inner,
    Append,
    Columns,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "左连接",
            Self::Inner => "内连接",
            Self::Append => "按行追加（同名列）",
            Self::Columns => "按行号拼列",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub data: Dataset,
    pub report: String,
}
pub fn key(value: &Value) -> Option<String> {
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
const DESKTOP_RESULT_BYTES: usize = 8 * 1024 * 1024;

fn push_row(
    rows: &mut Vec<Vec<Value>>,
    row: Vec<Value>,
    bytes: &mut usize,
    limit: usize,
) -> Result<()> {
    if rows.len() >= ROW_LIMIT {
        bail!("结果超过 10000 行，可能存在重复键展开；请先缩小范围");
    }
    *bytes += row.iter().map(|v| v.to_string().len()).sum::<usize>();
    if *bytes > limit {
        if limit == DESKTOP_RESULT_BYTES {
            bail!("结果内容超过 8 MiB；原数据未修改");
        }
        bail!("合并结果超出工具结果上限；请缩小输入范围");
    }
    rows.push(row);
    Ok(())
}
pub fn combine(
    left: &Dataset,
    right: &Dataset,
    mode: Mode,
    left_key: usize,
    right_key: &str,
    cancel: &AtomicBool,
) -> Result<Preview> {
    combine_bounded(
        left,
        right,
        mode,
        left_key,
        right_key.trim(),
        cancel,
        DESKTOP_RESULT_BYTES,
    )
}

/// The cell-byte limit is a lower bound for serialization, so this rejects oversized
/// expansions before allocating the result without changing which reports can fit.
pub fn combine_bounded(
    left: &Dataset,
    right: &Dataset,
    mode: Mode,
    left_key: usize,
    right_key: &str,
    cancel: &AtomicBool,
    limit: usize,
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
                push_row(&mut rows, row.clone(), &mut bytes, limit)?;
            }
            for row in &right.rows {
                check_cancel(cancel)?;
                push_row(
                    &mut rows,
                    columns.iter().map(|&i| row[i].clone()).collect(),
                    &mut bytes,
                    limit,
                )?;
            }
        }
        Mode::Columns => {
            if left.rows.len() != right.rows.len() {
                bail!("按行号拼列要求两表行数相同，避免错位或丢失数据");
            }
            for (l, r) in left.rows.iter().zip(&right.rows) {
                check_cancel(cancel)?;
                push_row(
                    &mut rows,
                    l.iter().chain(r).cloned().collect(),
                    &mut bytes,
                    limit,
                )?;
            }
        }
        Mode::Left | Mode::Inner => {
            if left_key >= left.headers.len() {
                bail!("左表关联列已变化，请重新选择");
            }
            let rk = if right_key.is_empty() {
                0
            } else {
                right
                    .headers
                    .iter()
                    .position(|h| h == right_key)
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
                            limit,
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
                            limit,
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
        data: Dataset {
            headers,
            rows: rows.into(),
        },
        report,
    })
}
