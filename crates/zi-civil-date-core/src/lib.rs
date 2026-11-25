//! Shared calendar arithmetic and original weekday counting rules.

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

pub const MAX_AMOUNT: i64 = 4_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Days,
    Months,
    Years,
}
pub fn parse(s: &str) -> Result<NaiveDate, String> {
    if s.len() != 10
        || !s.as_bytes().iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Err("日期格式必须是 YYYY-MM-DD，例如 2026-10-07".into());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| "无效公历日期（请检查闰年和月份天数）")?;
    bounded(d)
}
fn bounded(d: NaiveDate) -> Result<NaiveDate, String> {
    if (1..=9999).contains(&d.year()) {
        Ok(d)
    } else {
        Err("日期超出0001–9999年范围".into())
    }
}
pub fn amount(s: &str) -> Result<i64, String> {
    let n = s
        .trim()
        .parse::<i64>()
        .map_err(|_| "偏移量必须是整数；负数表示向前")?;
    bounded_amount(n)
}

fn bounded_amount(amount: i64) -> Result<i64, String> {
    if !(-MAX_AMOUNT..=MAX_AMOUNT).contains(&amount) {
        return Err("偏移量绝对值不能超过4000000".into());
    }
    Ok(amount)
}
pub fn offset(d: NaiveDate, n: i64, unit: Unit, clamp: bool) -> Result<NaiveDate, String> {
    bounded(d)?;
    bounded_amount(n)?;
    if unit == Unit::Days {
        return bounded(
            d.checked_add_signed(Duration::days(n))
                .ok_or("日期超出范围")?,
        );
    }
    let delta = if unit == Unit::Years { n * 12 } else { n };
    let index = i64::from(d.year()) * 12 + i64::from(d.month0()) + delta;
    let year = index.div_euclid(12);
    if !(1..=9999).contains(&year) {
        return Err("日期超出0001–9999年范围".into());
    }
    let month = index.rem_euclid(12) as u32 + 1;
    if let Some(date) = NaiveDate::from_ymd_opt(year as i32, month, d.day()) {
        return Ok(date);
    }
    if !clamp {
        return Err("目标月份不存在同一天；可勾选‘截到月末’后重新计算".into());
    }
    (28..=31)
        .rev()
        .find_map(|day| NaiveDate::from_ymd_opt(year as i32, month, day))
        .ok_or("无效目标月份".into())
}
pub fn is_weekday(d: NaiveDate) -> bool {
    d.weekday().num_days_from_monday() < 5
}
pub fn weekday_name(d: NaiveDate) -> &'static str {
    [
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
        "星期日",
    ][d.weekday().num_days_from_monday() as usize]
}
// Inclusive, constant-time counting even for the entire supported date range.
fn weekdays(start: NaiveDate, end: NaiveDate) -> i64 {
    let count = (end - start).num_days() + 1;
    let first = start.weekday().num_days_from_monday() as i64;
    count / 7 * 5 + (0..count % 7).filter(|i| (first + i) % 7 < 5).count() as i64
}
pub fn weekday_offset(start: NaiveDate, n: i64) -> Result<NaiveDate, String> {
    bounded(start)?;
    bounded_amount(n)?;
    if n == 0 {
        return Ok(start);
    }
    let direction = n.signum();
    let bound = if direction > 0 {
        NaiveDate::from_ymd_opt(9999, 12, 31).unwrap()
    } else {
        NaiveDate::from_ymd_opt(1, 1, 1).unwrap()
    };
    let mut high = (bound - start).num_days().abs();
    let count = |distance: i64| {
        let end = start + Duration::days(distance * direction);
        (if direction > 0 {
            weekdays(start, end)
        } else {
            weekdays(end, start)
        }) - i64::from(is_weekday(start))
    };
    if count(high) < n.abs() {
        return Err("工作日偏移超出0001–9999年范围".into());
    }
    let mut low = 1;
    while low < high {
        let mid = low + (high - low) / 2;
        if count(mid) >= n.abs() {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    Ok(start + Duration::days(low * direction))
}
pub fn difference(start: NaiveDate, end: NaiveDate) -> i64 {
    (end - start).num_days()
}

pub fn count_weekdays(
    start: NaiveDate,
    end: NaiveDate,
    include_start: bool,
) -> Result<i64, String> {
    if end < start {
        return Err("工作日计数的结束日期不能早于开始日期；加减日期请使用工作日加减".into());
    }
    Ok(weekdays(start, end) - i64::from(!include_start && is_weekday(start)))
}
