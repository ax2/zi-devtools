//! Bounded civil dates; weekdays explicitly exclude holiday rules.
use chrono::{Datelike, Duration, NaiveDate};
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    #[default]
    Difference,
    Offset,
    Weekdays,
    WeekdayOffset,
}
impl Operation {
    fn label(self) -> &'static str {
        match self {
            Self::Difference => "日期差",
            Self::Offset => "日期加减",
            Self::Weekdays => "工作日计数",
            Self::WeekdayOffset => "工作日加减",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Unit {
    #[default]
    Days,
    Months,
    Years,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    pub operation: Operation,
    pub start: String,
    pub end: String,
    pub amount: String,
    pub unit: Unit,
    pub clamp: bool,
    pub include_start: bool,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            operation: Operation::Difference,
            start: "2026-01-01".into(),
            end: "2026-12-31".into(),
            amount: "7".into(),
            unit: Unit::Days,
            clamp: false,
            include_start: false,
        }
    }
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        for s in [&self.start, &self.end, &self.amount] {
            if s.len() > 64 || s.chars().any(char::is_control) {
                return Err("日期草稿字段最多64字节，不含控制字符".into());
            }
        }
        Ok(())
    }
    fn compute(&self) -> Result<Output, String> {
        self.validate()?;
        let start = parse(&self.start)?;
        let result = match self.operation {
            Operation::Difference | Operation::Weekdays => {
                let end = parse(&self.end)?;
                let days = if self.operation == Operation::Difference {
                    (end - start).num_days()
                } else {
                    if end < start {
                        return Err(
                            "工作日计数的结束日期不能早于开始日期；加减日期请使用工作日加减".into(),
                        );
                    }
                    weekdays(start, end) - i64::from(!self.include_start && is_weekday(start))
                };
                return Ok(Output {
                    text: format!("{days} 天"),
                    number: Some(days),
                    date: None,
                });
            }
            Operation::Offset => offset(start, amount(&self.amount)?, self.unit, self.clamp)?,
            Operation::WeekdayOffset => weekday_offset(start, amount(&self.amount)?)?,
        };
        Ok(Output {
            text: format!("{} · {}", result.format("%Y-%m-%d"), weekday_name(result)),
            number: None,
            date: Some(result),
        })
    }
}
fn parse(s: &str) -> Result<NaiveDate, String> {
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
fn amount(s: &str) -> Result<i64, String> {
    let n = s
        .trim()
        .parse::<i64>()
        .map_err(|_| "偏移量必须是整数；负数表示向前")?;
    if n.unsigned_abs() > 4_000_000 {
        return Err("偏移量绝对值不能超过4000000".into());
    }
    Ok(n)
}
fn offset(d: NaiveDate, n: i64, unit: Unit, clamp: bool) -> Result<NaiveDate, String> {
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
fn is_weekday(d: NaiveDate) -> bool {
    d.weekday().num_days_from_monday() < 5
}
fn weekday_name(d: NaiveDate) -> &'static str {
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
fn weekday_offset(start: NaiveDate, n: i64) -> Result<NaiveDate, String> {
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
#[derive(Clone, Debug)]
struct Output {
    text: String,
    number: Option<i64>,
    date: Option<NaiveDate>,
}
#[derive(Default)]
pub(super) struct State {
    pub saved: Saved,
    send_requested: bool,
    result: Option<(Saved, Result<Output, String>)>,
    #[cfg(feature = "ui-preview")]
    pub compute_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    pub send_rect: Option<egui::Rect>,
}
impl State {
    pub fn calculate(&mut self) {
        self.result = Some((self.saved.clone(), self.saved.compute()));
    }
    fn current(&self) -> Result<&Output, String> {
        let (source, result) = self.result.as_ref().ok_or("请先计算日期结果")?;
        if source != &self.saved {
            return Err("输入已修改，请重新计算".into());
        }
        result.as_ref().map_err(Clone::clone)
    }
    #[cfg(feature = "ui-preview")]
    pub fn current_text(&self) -> Result<&str, String> {
        Ok(&self.current()?.text)
    }
    pub fn number(&self) -> Result<i64, String> {
        self.current()?
            .number
            .ok_or_else(|| "日期结果不是数值；请在日期工作台复制日期结果".into())
    }
    pub fn date_operation(&self) -> bool {
        matches!(
            self.saved.operation,
            Operation::Offset | Operation::WeekdayOffset
        )
    }
    pub fn date_snapshot(&self) -> Result<(NaiveDate, String), String> {
        let result = self.current()?;
        let date = result.date.ok_or("当前结果是天数，不是日期")?;
        let unit = if self.saved.operation == Operation::WeekdayOffset {
            "工作日"
        } else {
            match self.saved.unit {
                Unit::Days => "天",
                Unit::Months => "月",
                Unit::Years => "年",
            }
        };
        let policy = if self.saved.operation == Operation::WeekdayOffset {
            "工作日仅周一至周五，不含节假日与调休；不计开始日期"
        } else if self.saved.unit == Unit::Days {
            "按公历天数偏移，不进行月末替代"
        } else if self.saved.clamp {
            "目标月份缺少同一天时截到月末"
        } else {
            "目标月份缺少同一天时报错"
        };
        Ok((
            date,
            format!(
                "操作：{}\n开始日期：{}\n偏移：{} {}\n规则：{}\n结果：{}",
                self.saved.operation.label(),
                self.saved.start,
                self.saved.amount,
                unit,
                policy,
                result.text
            ),
        ))
    }
    pub fn take_transfer(&mut self) -> Result<Option<(NaiveDate, String)>, String> {
        if !std::mem::take(&mut self.send_requested) {
            return Ok(None);
        }
        self.date_snapshot().map(Some)
    }
    pub fn description(&self) -> Result<String, String> {
        Ok(format!("1×1 · 精确天数 {}", self.number()?))
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for op in [
                Operation::Difference,
                Operation::Offset,
                Operation::Weekdays,
                Operation::WeekdayOffset,
            ] {
                ui.selectable_value(&mut self.saved.operation, op, op.label());
            }
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("开始日期");
            ui.add(
                egui::TextEdit::singleline(&mut self.saved.start)
                    .char_limit(64)
                    .desired_width(130.0)
                    .hint_text("YYYY-MM-DD"),
            );
            if ui.button("今天").clicked() {
                self.saved.start = chrono::Local::now().format("%Y-%m-%d").to_string();
            }
        });
        match self.saved.operation {
            Operation::Difference | Operation::Weekdays => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("结束日期");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.saved.end)
                            .char_limit(64)
                            .desired_width(130.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    if ui.button("交换日期").clicked() {
                        std::mem::swap(&mut self.saved.start, &mut self.saved.end);
                    }
                });
                if self.saved.operation == Operation::Weekdays {
                    ui.checkbox(
                        &mut self.saved.include_start,
                        "包含开始日期（结束日期始终包含）",
                    );
                } else {
                    ui.small("结果 = 结束日期 − 开始日期；同一天为0，结束在前为负数。");
                }
            }
            Operation::Offset | Operation::WeekdayOffset => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("偏移量");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.saved.amount)
                            .char_limit(64)
                            .desired_width(130.0),
                    );
                    if self.saved.operation == Operation::Offset {
                        for (unit, label) in [
                            (Unit::Days, "天"),
                            (Unit::Months, "月"),
                            (Unit::Years, "年"),
                        ] {
                            ui.selectable_value(&mut self.saved.unit, unit, label);
                        }
                    } else {
                        ui.label("工作日");
                    }
                });
                ui.small("正数向后，负数向前；0保留原日期。");
                if self.saved.operation == Operation::Offset && self.saved.unit != Unit::Days {
                    ui.checkbox(
                        &mut self.saved.clamp,
                        "目标月份没有同一天时截到月末（未选则报错）",
                    );
                }
            }
        }
        if matches!(
            self.saved.operation,
            Operation::Weekdays | Operation::WeekdayOffset
        ) {
            ui.label("工作日仅指周一至周五，不计算法定节假日与调休。");
            if self.saved.operation == Operation::WeekdayOffset {
                ui.small("偏移不计开始日期；周末起点从下一个/上一个周一至周五日期开始计数。");
            }
        }
        ui.add_space(12.0);
        let button = ui.button("计算日期结果");
        #[cfg(feature = "ui-preview")]
        {
            self.compute_rect = Some(button.rect);
        }
        if button.clicked() {
            self.calculate();
        }
        let mut send = false;
        #[cfg(feature = "ui-preview")]
        let mut send_rect = None;
        egui::Frame::group(ui.style())
            .inner_margin(16.0)
            .show(ui, |ui| match self.current() {
                Ok(result) => {
                    ui.label(egui::RichText::new(&result.text).size(26.0).strong());
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("复制结果").clicked() {
                            ui.ctx().copy_text(result.text.clone());
                        }
                        if result.date.is_some() {
                            let response = ui.button("发送日期到工具…");
                            send = response.clicked();
                            #[cfg(feature = "ui-preview")]
                            {
                                send_rect = Some(response.rect);
                            }
                        }
                    });
                }
                Err(error) => {
                    ui.label(error);
                }
            });
        self.send_requested |= send;
        #[cfg(feature = "ui-preview")]
        {
            self.send_rect = send_rect;
        }
        ui.small("公历0001–9999年；无时区或夏令时换算。另存工作表会保存输入，恢复后需重新计算。");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        parse(s).unwrap()
    }
    #[test]
    fn date_handoff_captures_typed_snapshot_and_rechecks_same_frame_input() {
        let mut state = State::default();
        state.saved.operation = Operation::WeekdayOffset;
        state.saved.start = "2026-10-09".into();
        state.saved.amount = "3".into();
        state.calculate();
        state.send_requested = true;
        let (day, text) = state.take_transfer().unwrap().unwrap();
        assert_eq!(day, d("2026-10-14"));
        assert!(text.contains("3 工作日") && text.contains("不含节假日"));
        assert!(state.take_transfer().unwrap().is_none());
        state.send_requested = true;
        state.saved.amount = "4".into();
        assert!(state.take_transfer().is_err());
        assert!(state.take_transfer().unwrap().is_none());
        assert_eq!(day, d("2026-10-14"));
        assert!(text.contains("3 工作日"));
        state.calculate();
        assert_eq!(state.date_snapshot().unwrap().0, d("2026-10-15"));
        state.saved.operation = Operation::Difference;
        state.calculate();
        assert!(state.date_snapshot().is_err());
    }
    #[test]
    fn strict_dates_leap_years_and_bounds() {
        for invalid in [
            "1900-02-29",
            "0000-01-01",
            "10000-01-01",
            "2026-1-01",
            " 2026-01-01",
            "2026-04-31",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
        assert!(parse("2000-02-29").is_ok());
        assert!(offset(d("9999-12-31"), 1, Unit::Days, false).is_err());
        assert!(offset(d("0001-01-01"), -1, Unit::Days, false).is_err());
        assert_eq!(
            offset(d("2000-02-28"), 1, Unit::Days, false).unwrap(),
            d("2000-02-29")
        );
    }
    #[test]
    fn month_policy_is_explicit_and_signed() {
        assert!(offset(d("2024-01-31"), 1, Unit::Months, false).is_err());
        assert_eq!(
            offset(d("2024-01-31"), 1, Unit::Months, true).unwrap(),
            d("2024-02-29")
        );
        assert_eq!(
            offset(d("2024-03-31"), -1, Unit::Months, true).unwrap(),
            d("2024-02-29")
        );
        assert!(offset(d("2024-02-29"), 1, Unit::Years, false).is_err());
        assert_eq!(
            offset(d("2024-02-29"), 1, Unit::Years, true).unwrap(),
            d("2025-02-28")
        );
    }
    #[test]
    fn weekday_binary_search_matches_independent_daily_walk() {
        for day in 1..=28 {
            let start = d(&format!("2026-02-{day:02}"));
            for n in -35_i64..=35 {
                let mut expected = start;
                let mut remaining = n.abs();
                while remaining > 0 {
                    expected += Duration::days(n.signum());
                    if is_weekday(expected) {
                        remaining -= 1;
                    }
                }
                assert_eq!(weekday_offset(start, n).unwrap(), expected, "{start} {n}");
            }
        }
        assert!(weekday_offset(d("9999-12-31"), 1).is_err());
        assert!(weekday_offset(d("0001-01-01"), -1).is_err());
    }
    #[test]
    fn counts_endpoints_drafts_and_stale_results() {
        let mut saved = Saved {
            start: "2026-10-05".into(),
            end: "2026-10-09".into(),
            ..Default::default()
        };
        assert_eq!(saved.compute().unwrap().number, Some(4));
        saved.operation = Operation::Weekdays;
        assert_eq!(saved.compute().unwrap().number, Some(4));
        saved.include_start = true;
        assert_eq!(saved.compute().unwrap().number, Some(5));
        saved.end = "2026-10-05".into();
        assert_eq!(saved.compute().unwrap().number, Some(1));
        saved.include_start = false;
        assert_eq!(saved.compute().unwrap().number, Some(0));
        let mut state = State {
            saved: saved.clone(),
            send_requested: false,
            result: Some((saved.clone(), saved.compute())),
            #[cfg(feature = "ui-preview")]
            compute_rect: None,
            #[cfg(feature = "ui-preview")]
            send_rect: None,
        };
        assert_eq!(state.number().unwrap(), 0);
        state.saved.start = "invalid draft".into();
        assert!(state.saved.validate().is_ok());
        assert!(state.number().is_err());
        assert!(state.saved.compute().is_err());
        state.saved.amount = "x".repeat(65);
        assert!(state.saved.validate().is_err());
    }
}
