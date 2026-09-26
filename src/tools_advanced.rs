//! Bounded, local-only structured data and text utilities.
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

fn pretty(value: &Value) -> Result<String> {
    let text = serde_json::to_string_pretty(value)?;
    ensure!(
        text.len() <= 8 * 1024 * 1024,
        "结果超过 8 MiB，请缩小输入范围"
    );
    Ok(text)
}

pub fn json_path(input: &str, query: &str) -> Result<String> {
    crate::tools_extra::bounded(input)?;
    ensure!(query.len() <= 4096, "查询过长（最多 4096 字节）");
    let value: Value = serde_json::from_str(input).context("输入不是有效 JSON")?;
    let query = query.trim();
    ensure!(query.starts_with('$'), "路径必须从 $ 开始");
    let mut rest = &query[1..];
    let mut values = vec![&value];
    let mut steps = 0;
    while !rest.is_empty() {
        steps += 1;
        ensure!(steps <= 128, "路径最多 128 层");
        enum Step {
            Key(String),
            Index(usize),
            Wildcard,
        }
        let step;
        if let Some(tail) = rest.strip_prefix('.') {
            let end = tail.find(['.', '[']).unwrap_or(tail.len());
            let key = &tail[..end];
            ensure!(
                !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_'),
                "点号后需要字段名；特殊字段名请使用 [\"字段\"]"
            );
            step = Step::Key(key.into());
            rest = &tail[end..];
        } else if let Some(tail) = rest.strip_prefix('[') {
            if tail.starts_with('"') {
                let mut stream = serde_json::Deserializer::from_str(tail).into_iter::<String>();
                let key = stream.next().context("缺少字段名")??;
                let end = stream.byte_offset();
                ensure!(tail[end..].starts_with(']'), "字段名后缺少 ]");
                step = Step::Key(key);
                rest = &tail[end + 1..];
            } else {
                let end = tail.find(']').context("缺少 ]")?;
                let token = &tail[..end];
                step = if token == "*" {
                    Step::Wildcard
                } else {
                    ensure!(
                        !token.is_empty() && token.bytes().all(|c| c.is_ascii_digit()),
                        "仅支持非负索引或 [*]，不支持过滤器与切片"
                    );
                    Step::Index(token.parse().context("数组索引过大")?)
                };
                rest = &tail[end + 1..];
            }
        } else {
            bail!("不支持的路径语法；可用 .字段、[\"字段\"]、[索引]、[*]");
        }
        let mut next = Vec::new();
        for value in values {
            match &step {
                Step::Key(key) => {
                    if let Some(v) = value.as_object().and_then(|o| o.get(key)) {
                        next.push(v);
                    }
                }
                Step::Index(i) => {
                    if let Some(v) = value.as_array().and_then(|a| a.get(*i)) {
                        next.push(v);
                    }
                }
                Step::Wildcard => match value {
                    Value::Array(a) => next.extend(a),
                    Value::Object(o) => next.extend(o.values()),
                    _ => {}
                },
            }
            ensure!(next.len() <= 10_000, "匹配超过 10000 项，请缩小路径范围");
        }
        values = next;
    }
    pretty(&json!(values))
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Array(values) => {
            let mut values: Vec<_> = values.iter().map(canonical).collect();
            values.sort_by_cached_key(|v| v.to_string());
            Value::Array(values)
        }
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub fn json_diff(left: &str, right: &str, unordered: bool) -> Result<String> {
    crate::tools_extra::bounded(left)?;
    crate::tools_extra::bounded(right)?;
    let left: Value = serde_json::from_str(left).context("左侧 JSON 无效")?;
    let right: Value = serde_json::from_str(right).context("右侧 JSON 无效")?;
    let mut changes = Vec::new();
    fn visit(
        a: &Value,
        b: &Value,
        path: &str,
        unordered: bool,
        changes: &mut Vec<Value>,
    ) -> Result<()> {
        if a == b || (unordered && canonical(a) == canonical(b)) {
            return Ok(());
        }
        ensure!(changes.len() < 10_000, "差异超过 10000 项，请缩小输入");
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    match (a.get(key), b.get(key)) {
                        (Some(a), Some(b)) => visit(a, b, &path, unordered, changes)?,
                        (Some(a), None) => {
                            changes.push(json!({"path":path,"kind":"removed","before":a}))
                        }
                        (None, Some(b)) => {
                            changes.push(json!({"path":path,"kind":"added","after":b}))
                        }
                        _ => unreachable!(),
                    }
                    ensure!(changes.len() <= 10_000, "差异超过 10000 项，请缩小输入");
                }
            }
            (Value::Array(a), Value::Array(b)) if !unordered => {
                for i in 0..a.len().max(b.len()) {
                    let path = format!("{path}/{i}");
                    match (a.get(i), b.get(i)) {
                        (Some(a), Some(b)) => visit(a, b, &path, unordered, changes)?,
                        (Some(a), None) => {
                            changes.push(json!({"path":path,"kind":"removed","before":a}))
                        }
                        (None, Some(b)) => {
                            changes.push(json!({"path":path,"kind":"added","after":b}))
                        }
                        _ => unreachable!(),
                    }
                    ensure!(changes.len() <= 10_000, "差异超过 10000 项，请缩小输入");
                }
            }
            _ => changes.push(json!({"path":path,"kind":"changed","before":a,"after":b})),
        }
        Ok(())
    }
    visit(&left, &right, "", unordered, &mut changes)?;
    pretty(
        &json!({"equal":changes.is_empty(),"arrayOrder":if unordered {"ignored (duplicates retained)"} else {"by index"},"changes":changes}),
    )
}

pub fn quality(input: &str, action: usize) -> Result<String> {
    use crate::workbench::{DataFormat, Dataset};
    let data = Dataset::parse(
        input,
        if action == 0 {
            DataFormat::Json
        } else {
            DataFormat::Csv
        },
        if action == 2 { b'\t' } else { b',' },
    )?;
    let mut seen = BTreeSet::new();
    let duplicates = data
        .rows
        .iter()
        .filter(|row| !seen.insert(serde_json::to_string(row).unwrap()))
        .count();
    let columns: Vec<_> = data.headers.iter().enumerate().map(|(i,name)| {
        let mut types: BTreeMap<&str,usize> = BTreeMap::new();
        let mut nulls = 0;
        let mut blanks = 0;
        for row in &data.rows {
            let v = &row[i];
            let kind = match v {
                Value::Null => {nulls += 1; "null"},
                Value::Bool(_) => "boolean", Value::Number(_) => "number",
                Value::String(s) => {if s.trim().is_empty() {blanks += 1;} "string"},
                Value::Array(_) => "array", Value::Object(_) => "object"
            };
            *types.entry(kind).or_default() += 1;
        }
        json!({"column":name,"missingOrNull":nulls,"blankStrings":blanks,"types":types,"mixedNonNullTypes":types.keys().filter(|k| **k != "null").count()>1})
    }).collect();
    pretty(
        &json!({"rows":data.rows.len(),"columns":columns,"duplicateRowsAfterFirst":duplicates,"notes":"缺失字段与 null 合计；空白字符串单独统计。CSV/TSV 字段保持字符串，不猜测类型。重复行按完整行值比较。"}),
    )
}

fn cron_field(text: &str, min: u32, max: u32, sunday: bool) -> Result<BTreeSet<u32>> {
    let mut set = BTreeSet::new();
    for part in text.split(',') {
        let (range, step) = if let Some((range, step)) = part.split_once('/') {
            let step: u32 = step.parse().context("Cron 步长必须为正整数")?;
            ensure!(step > 0 && step <= max + 1, "Cron 步长越界");
            (range, step)
        } else {
            (part, 1)
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse::<u32>().context("Cron 范围无效")?,
                b.parse::<u32>().context("Cron 范围无效")?,
            )
        } else {
            let n = range
                .parse::<u32>()
                .context("Cron 仅支持数字、*、范围、列表和步长")?;
            (n, if part.contains('/') { max } else { n })
        };
        ensure!(
            start >= min && start <= end && end <= max,
            "Cron 数值应在 {min}..={max} 内且范围不能倒置"
        );
        for n in (start..=end).step_by(step as usize) {
            set.insert(if sunday && n == 7 { 0 } else { n });
        }
    }
    Ok(set)
}

pub fn cron(input: &str, reference: &str) -> Result<String> {
    ensure!(
        input.len() <= 1024 && reference.len() <= 100,
        "Cron 或参考时间过长"
    );
    let parts: Vec<_> = input.split_whitespace().collect();
    ensure!(
        parts.len() == 5,
        "请输入 5 个字段：分 时 日 月 周；不支持秒、年、L/W/# 或月份名称"
    );
    let minutes = cron_field(parts[0], 0, 59, false)?;
    let hours = cron_field(parts[1], 0, 23, false)?;
    let days = cron_field(parts[2], 1, 31, false)?;
    let months = cron_field(parts[3], 1, 12, false)?;
    let weekdays = cron_field(parts[4], 0, 7, true)?;
    let reference: DateTime<FixedOffset> = if reference.trim().is_empty() {
        Utc::now().fixed_offset()
    } else {
        DateTime::parse_from_rfc3339(reference.trim())
            .context("参考时间需为 RFC3339，例如 2026-09-26T09:00:00+08:00")?
    };
    let mut result = Vec::new();
    for offset in 0..=366 * 5 {
        let date = reference
            .date_naive()
            .checked_add_signed(Duration::days(offset))
            .context("日期超出支持范围")?;
        if !months.contains(&date.month()) {
            continue;
        }
        let dom = days.contains(&date.day());
        let dow = weekdays.contains(&date.weekday().num_days_from_sunday());
        let matches = if parts[2].starts_with('*') || parts[4].starts_with('*') {
            dom && dow
        } else {
            dom || dow
        };
        if !matches {
            continue;
        }
        for hour in &hours {
            for minute in &minutes {
                let naive = date.and_hms_opt(*hour, *minute, 0).context("日期无效")?;
                let time = reference
                    .offset()
                    .from_local_datetime(&naive)
                    .single()
                    .context("日期超出范围")?;
                if time > reference {
                    result.push(time.to_rfc3339());
                }
                if result.len() == 10 {
                    return pretty(
                        &json!({"reference":reference.to_rfc3339(),"timezone":"固定 UTC 偏移，不模拟夏令时","next":result}),
                    );
                }
            }
        }
    }
    ensure!(
        !result.is_empty(),
        "未来 5 年内没有匹配时间，请检查日期组合"
    );
    pretty(
        &json!({"reference":reference.to_rfc3339(),"timezone":"固定 UTC 偏移，不模拟夏令时","note":"仅列出未来 5 年内的匹配","next":result}),
    )
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RandomSettings {
    length: usize,
    count: usize,
    alphabet: String,
    seed: u64,
}
impl Default for RandomSettings {
    fn default() -> Self {
        Self {
            length: 24,
            count: 5,
            alphabet: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_".into(),
            seed: 42,
        }
    }
}
pub fn random(input: &str, seeded: bool) -> Result<String> {
    let settings: RandomSettings = serde_json::from_str(input)
        .context("请输入 JSON 参数：length、count、alphabet、seed（均可省略）")?;
    ensure!(
        (1..=256).contains(&settings.length) && (1..=100).contains(&settings.count),
        "length 范围 1–256；count 范围 1–100"
    );
    let chars: Vec<_> = settings
        .alphabet
        .chars()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ensure!(
        (2..=256).contains(&chars.len())
            && !chars.iter().any(|c| c.is_control() || c.is_whitespace()),
        "字符集去重后需包含 2–256 个非空白、非控制字符"
    );
    let mut state = settings.seed;
    let mut rows = Vec::new();
    for _ in 0..settings.count {
        let mut value = String::new();
        for _ in 0..settings.length {
            let index = if seeded {
                state = state.wrapping_add(0x9e3779b97f4a7c15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                ((z ^ (z >> 31)) % chars.len() as u64) as usize
            } else {
                // Rejection sampling prevents modulo bias for arbitrary alphabets.
                let limit = 256 - (256 % chars.len());
                loop {
                    let mut byte = [0u8];
                    getrandom::fill(&mut byte)
                        .map_err(|e| anyhow::anyhow!("操作系统随机源失败：{e}"))?;
                    if (byte[0] as usize) < limit {
                        break byte[0] as usize % chars.len();
                    }
                }
            };
            value.push(chars[index]);
        }
        rows.push(value);
    }
    pretty(
        &json!({"mode":if seeded {"可复现测试数据；不得用作密码或令牌"} else {"操作系统安全随机；seed 不参与生成"},"values":rows}),
    )
}

pub fn unicode(input: &str, action: usize) -> Result<String> {
    ensure!(
        input.chars().count() <= 10_000,
        "Unicode 工具最多处理 10000 个码点"
    );
    match action {
        1 => Ok(input.nfc().collect()),
        2 => Ok(input.nfkc().collect()),
        _ => {
            let points: Vec<_> = input.char_indices().map(|(offset,c)| json!({"byteOffset":offset,"character":c.to_string(),"codePoint":format!("U+{:04X}",c as u32),"utf8":c.to_string().as_bytes().iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),"attention": c.is_control() || c.is_whitespace() || matches!(c,'\u{00ad}'|'\u{034f}'|'\u{061c}'|'\u{180e}'|'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{feff}'|'\u{fe00}'..='\u{fe0f}') })).collect();
            pretty(
                &json!({"codePoints":points,"nfc":input.nfc().collect::<String>(),"nfkc":input.nfkc().collect::<String>(),"notes":"按 Unicode 码点而非可见字形列出；attention 标记常见控制、空白与格式字符，并非完整安全检测。NFKC 可能改变字符语义。"}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_handle_unicode_escaping_and_missing() {
        assert_eq!(
            json_path(r#"{"项":[{"a.b":1},{"a.b":2}]}"#, r#"$.项[*]["a.b"]"#).unwrap(),
            "[\n  1,\n  2\n]"
        );
        assert_eq!(json_path("{}", "$.missing[0]").unwrap(), "[]");
        for path in ["$..a", "$[-1]", "$[?(@.x)]", "$[", "$.a b"] {
            assert!(json_path("{}", path).is_err());
        }
    }
    #[test]
    fn diff_preserves_duplicates_and_escapes_pointers() {
        let report: Value =
            serde_json::from_str(&json_diff(r#"{"a/b~":1}"#, r#"{"a/b~":null}"#, false).unwrap())
                .unwrap();
        assert_eq!(report["changes"][0]["path"], "/a~1b~0");
        assert!(
            json_diff("[1,2]", "[2,1]", true)
                .unwrap()
                .contains("\"equal\": true")
        );
        assert!(
            json_diff("[1,1,2]", "[1,2,2]", true)
                .unwrap()
                .contains("\"equal\": false")
        );
        assert!(
            json_diff("[1,2]", "[2,1]", false)
                .unwrap()
                .contains("\"equal\": false")
        );
    }
    #[test]
    fn quality_counts_nulls_blanks_duplicates_and_types() {
        let report: Value =
            serde_json::from_str(&quality(r#"[{"x":1},{"x":" "},{"x":null},{"x":1}]"#, 0).unwrap())
                .unwrap();
        assert_eq!(report["duplicateRowsAfterFirst"], 1);
        assert_eq!(report["columns"][0]["missingOrNull"], 1);
        assert_eq!(report["columns"][0]["blankStrings"], 1);
        assert_eq!(report["columns"][0]["mixedNonNullTypes"], true);
        assert!(quality("x,x\n1,2", 1).is_err());
    }
    #[test]
    fn cron_respects_boundaries_offsets_and_calendar() {
        let report: Value =
            serde_json::from_str(&cron("*/15 9-10 * * 1-5", "2026-09-25T10:45:00+08:00").unwrap())
                .unwrap();
        assert_eq!(report["next"][0], "2026-09-28T09:00:00+08:00");
        assert!(cron("0 0 31 2 *", "2026-01-01T00:00:00Z").is_err());
        assert!(cron("*/0 * * * *", "").is_err());
        assert!(cron("0 0 * * 8", "").is_err());
        let report: Value =
            serde_json::from_str(&cron("0 0 29 2 *", "2026-01-01T00:00:00Z").unwrap()).unwrap();
        assert_eq!(report["next"][0], "2028-02-29T00:00:00+00:00");
    }
    #[test]
    fn random_is_bounded_and_seeded_mode_is_reproducible() {
        assert_eq!(random("{}", true).unwrap(), random("{}", true).unwrap());
        let report: Value = serde_json::from_str(
            &random(r#"{"length":17,"count":3,"alphabet":"abc"}"#, false).unwrap(),
        )
        .unwrap();
        for v in report["values"].as_array().unwrap() {
            let s = v.as_str().unwrap();
            assert_eq!(s.len(), 17);
            assert!(s.chars().all(|c| "abc".contains(c)));
        }
        assert!(random(r#"{"count":100000}"#, false).is_err());
        assert!(random(r#"{"alphabet":"aaaa"}"#, false).is_err());
        assert!(random(r#"{"lenght":10}"#, false).is_err());
    }
    #[test]
    fn normalization_and_invisible_characters() {
        assert_eq!(unicode("e\u{301}", 1).unwrap(), "é");
        assert_eq!(unicode("Ａ①", 2).unwrap(), "A1");
        let report: Value = serde_json::from_str(&unicode("a\u{200b}", 0).unwrap()).unwrap();
        assert_eq!(report["codePoints"][1]["attention"], true);
        assert_eq!(report["codePoints"][1]["byteOffset"], 1);
    }
}
