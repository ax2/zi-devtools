//! Native planner recurrence and iCalendar rule conversion without host or timezone I/O.
use anyhow::{Context, Result, bail, ensure};
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MIN_YEAR: i32 = 1901;
pub const MAX_YEAR: i32 = 2099;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    Once,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}
impl Repeat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "不重复",
            Self::Daily => "每天",
            Self::Weekly => "每周",
            Self::Monthly => "每月",
            Self::Yearly => "每年",
        }
    }
    pub fn days(self) -> Option<i64> {
        match self {
            Self::Once | Self::Monthly | Self::Yearly => None,
            Self::Daily => Some(1),
            Self::Weekly => Some(7),
        }
    }
}

pub struct Rule {
    pub repeat: Repeat,
    pub repeat_until: Option<NaiveDate>,
    pub clamp_missing_day: bool,
    pub count: Option<CountConversion>,
}

pub struct CountConversion {
    pub count: u32,
    pub last: NaiveDate,
    pub frequency: Repeat,
}

fn calendar_horizon_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(MAX_YEAR, 12, 31).expect("valid planner end date")
}

// Always anchor to the original day, not the last clamped occurrence.
pub fn date_in_month(
    start: NaiveDate,
    clamp_missing_day: bool,
    year: i32,
    month: u32,
) -> Option<NaiveDate> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, start.day()).or_else(|| {
        clamp_missing_day
            .then(|| {
                NaiveDate::from_ymd_opt(year, month, 1)?
                    .checked_add_months(chrono::Months::new(1))?
                    .pred_opt()
            })
            .flatten()
    })
}

/// Return the local date of the nth occurrence for a supported recurrence.
/// The calendar horizon bounds monthly/yearly scans to fewer than 2400 steps.
fn occurrence_date(
    start: NaiveDate,
    repeat: Repeat,
    clamp_missing_day: bool,
    count: u32,
) -> Option<NaiveDate> {
    if count == 0 || !(MIN_YEAR..=MAX_YEAR).contains(&start.year()) {
        return None;
    }
    if count == 1 {
        return Some(start);
    }
    match repeat {
        Repeat::Once => None,
        Repeat::Daily | Repeat::Weekly => {
            let interval = if repeat == Repeat::Daily { 1 } else { 7 };
            let offset = u64::from(count - 1).checked_mul(interval)?;
            let date = start.checked_add_days(Days::new(offset))?;
            (date.year() <= MAX_YEAR).then_some(date)
        }
        Repeat::Monthly => {
            let mut remaining = count;
            for offset in 0..((MAX_YEAR - start.year() + 1) as u32 * 12) {
                let month = start
                    .with_day(1)?
                    .checked_add_months(chrono::Months::new(offset))?;
                if let Some(date) =
                    date_in_month(start, clamp_missing_day, month.year(), month.month())
                {
                    remaining -= 1;
                    if remaining == 0 {
                        return Some(date);
                    }
                }
            }
            None
        }
        Repeat::Yearly => {
            let mut remaining = count;
            for year in start.year()..=MAX_YEAR {
                if let Some(date) = date_in_month(start, clamp_missing_day, year, start.month()) {
                    remaining -= 1;
                    if remaining == 0 {
                        return Some(date);
                    }
                }
            }
            None
        }
    }
}

pub fn parse_rule(start: NaiveDateTime, all_day: bool, rule: &str) -> Result<Rule> {
    let mut repeat_until = None;
    let mut clamp_missing_day = false;
    let mut parts = HashMap::new();
    for part in rule.split(';') {
        let (key, value) = part.split_once('=').context("RRULE 格式无效")?;
        ensure!(
            parts
                .insert(key.to_ascii_uppercase(), value.to_ascii_uppercase())
                .is_none(),
            "RRULE 属性重复"
        );
    }
    ensure!(
        parts.remove("INTERVAL").is_none_or(|v| v == "1"),
        "暂不支持间隔大于 1 的重复规则"
    );
    let until = parts.remove("UNTIL");
    let count = parts.remove("COUNT");
    ensure!(
        until.is_none() || count.is_none(),
        "RRULE 不能同时设置 UNTIL 和 COUNT"
    );
    if let Some(until) = until {
        let day = if all_day {
            ensure!(
                until.len() == 8 && until.bytes().all(|b| b.is_ascii_digit()),
                "全天重复的 UNTIL 必须是 YYYYMMDD 日期"
            );
            NaiveDate::parse_from_str(&until, "%Y%m%d")?
        } else {
            ensure!(
                until.len() == 15 && until.as_bytes()[8] == b'T',
                "浮动时间的 UNTIL 必须是本地 YYYYMMDDTHHMMSS，不附 Z 或时区"
            );
            let at = NaiveDateTime::parse_from_str(&until, "%Y%m%dT%H%M%S")?;
            ensure!(
                at.nanosecond() == 0 && at >= start,
                "重复截止不能早于开始，且不能含闰秒"
            );
            // Each supported rule keeps DTSTART's clock time. A cutoff before
            // that clock excludes its date; retain the same occurrence set.
            if at.time() < start.time() {
                at.date().pred_opt().context("截止日期无效")?
            } else {
                at.date()
            }
        };
        ensure!(
            day >= start.date() && (MIN_YEAR..=MAX_YEAR).contains(&day.year()),
            "重复截止日期超出范围或早于开始"
        );
        repeat_until = (day != calendar_horizon_end()).then_some(day);
    }
    let mut repeat = match parts.remove("FREQ").as_deref() {
        Some("DAILY") => Repeat::Daily,
        Some("WEEKLY") => Repeat::Weekly,
        Some("MONTHLY") => Repeat::Monthly,
        Some("YEARLY") => Repeat::Yearly,
        _ => bail!("暂不支持此重复频率"),
    };
    if repeat == Repeat::Weekly {
        if let Some(day) = parts.remove("BYDAY") {
            ensure!(
                day == ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
                    [start.weekday().num_days_from_monday() as usize],
                "暂不支持每周多个星期日或与开始日期不同的星期日"
            );
        }
        if let Some(week) = parts.remove("WKST") {
            ensure!(
                ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].contains(&week.as_str()),
                "WKST 无效"
            );
        }
    }
    if repeat == Repeat::Yearly {
        if let Some(month) = parts.remove("BYMONTH") {
            ensure!(month == start.month().to_string(), "暂不支持多个重复月份");
        } else {
            ensure!(
                !parts.contains_key("BYMONTHDAY"),
                "每年按日号重复必须明确限定起始月份，否则会增加额外日期"
            );
        }
    }
    if matches!(repeat, Repeat::Monthly | Repeat::Yearly)
        && let Some(days) = parts.remove("BYMONTHDAY")
    {
        let clamped = (28..=start.day())
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if start.day() > 28
            && days == clamped
            && parts.get("BYSETPOS").map(String::as_str) == Some("-1")
        {
            parts.remove("BYSETPOS");
            clamp_missing_day = true;
        } else {
            ensure!(days == start.day().to_string(), "暂不支持多个重复日号");
        }
    }
    ensure!(
        parts.is_empty(),
        "RRULE 含当前无法保留的条件（如多值 BYDAY / BYSETPOS），未导入任何记录"
    );
    let count = if let Some(count) = count {
        let count = count.parse::<u32>().context("RRULE COUNT 必须是正整数")?;
        ensure!(count > 0, "RRULE COUNT 必须大于零");
        let Some(last) = occurrence_date(start.date(), repeat, clamp_missing_day, count) else {
            bail!("RRULE COUNT 超出日程支持的 1901–2099 年日期范围");
        };
        let frequency = repeat;
        if count == 1 {
            repeat = Repeat::Once;
            clamp_missing_day = false;
        } else {
            repeat_until = (last != calendar_horizon_end()).then_some(last);
        }
        Some(CountConversion {
            count,
            last,
            frequency,
        })
    } else {
        None
    };
    Ok(Rule {
        repeat,
        repeat_until,
        clamp_missing_day,
        count,
    })
}
