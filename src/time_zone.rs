//! Local IANA time-zone conversion with explicit daylight-saving ambiguity handling.
use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use eframe::egui;

const MAX_TARGET_ZONES: usize = 8;
const SEARCH_GAP_MINUTES: i64 = 24 * 60;

fn nearest_valid_minute(zone: Tz, local: NaiveDateTime, direction: i64) -> Option<NaiveDateTime> {
    let at_distance = |minutes: i64| {
        let candidate = if direction < 0 {
            local.checked_sub_signed(chrono::Duration::minutes(minutes))
        } else {
            local.checked_add_signed(chrono::Duration::minutes(minutes))
        }?;
        (!matches!(zone.from_local_datetime(&candidate), LocalResult::None)).then_some(candidate)
    };

    let mut invalid_distance = 0;
    let mut valid_distance = 1;
    while valid_distance <= SEARCH_GAP_MINUTES && at_distance(valid_distance).is_none() {
        invalid_distance = valid_distance;
        valid_distance = (valid_distance * 2).min(SEARCH_GAP_MINUTES + 1);
    }
    if valid_distance > SEARCH_GAP_MINUTES {
        valid_distance = SEARCH_GAP_MINUTES;
        at_distance(valid_distance)?;
    }

    while valid_distance - invalid_distance > 1 {
        let midpoint = invalid_distance + (valid_distance - invalid_distance) / 2;
        if at_distance(midpoint).is_some() {
            valid_distance = midpoint;
        } else {
            invalid_distance = midpoint;
        }
    }
    at_distance(valid_distance)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbiguousChoice {
    Earlier,
    Later,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalTimeStatus {
    Unique(DateTime<Utc>),
    Ambiguous {
        earlier: DateTime<Utc>,
        later: DateTime<Utc>,
    },
    Nonexistent {
        previous_valid: Option<NaiveDateTime>,
        next_valid: Option<NaiveDateTime>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionRow {
    pub zone: Tz,
    pub local: String,
    pub offset: String,
    pub abbreviation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub utc: DateTime<Utc>,
    pub source_zone: Tz,
    pub source_local: String,
    pub rows: Vec<ConversionRow>,
}

pub fn resolve_local_status(zone: Tz, local: NaiveDateTime) -> LocalTimeStatus {
    match zone.from_local_datetime(&local) {
        LocalResult::Single(value) => LocalTimeStatus::Unique(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => {
            let first = first.with_timezone(&Utc);
            let second = second.with_timezone(&Utc);
            LocalTimeStatus::Ambiguous {
                earlier: first.min(second),
                later: first.max(second),
            }
        }
        LocalResult::None => LocalTimeStatus::Nonexistent {
            previous_valid: nearest_valid_minute(zone, local, -1),
            next_valid: nearest_valid_minute(zone, local, 1),
        },
    }
}

fn format_source_offset(zone: Tz, utc: DateTime<Utc>) -> String {
    utc.with_timezone(&zone).format("%:z").to_string()
}

pub fn convert_local(
    local: NaiveDateTime,
    source_zone: Tz,
    targets: &[Tz],
    ambiguous_choice: Option<AmbiguousChoice>,
) -> Result<Conversion, String> {
    if targets.is_empty() || targets.len() > MAX_TARGET_ZONES {
        return Err(format!("目的时区数量必须在 1 到 {MAX_TARGET_ZONES} 之间"));
    }
    let utc = match resolve_local_status(source_zone, local) {
        LocalTimeStatus::Unique(utc) => utc,
        LocalTimeStatus::Ambiguous { earlier, later } => match ambiguous_choice {
            Some(AmbiguousChoice::Earlier) => earlier,
            Some(AmbiguousChoice::Later) => later,
            None => return Err("该本地时间出现两次，请明确选择较早或较晚的时刻".into()),
        },
        LocalTimeStatus::Nonexistent {
            previous_valid,
            next_valid,
        } => {
            let previous = previous_valid
                .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "无".into());
            let next = next_valid
                .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "无".into());
            return Err(format!(
                "该本地时间因夏令时切换而不存在；附近有效时间：之前 {previous}，之后 {next}"
            ));
        }
    };
    let rows = targets
        .iter()
        .map(|zone| {
            let value = utc.with_timezone(zone);
            ConversionRow {
                zone: *zone,
                local: value.format("%Y-%m-%d %H:%M:%S").to_string(),
                offset: value.format("%:z").to_string(),
                abbreviation: value.format("%Z").to_string(),
            }
        })
        .collect();
    Ok(Conversion {
        utc,
        source_zone,
        source_local: local.format("%Y-%m-%d %H:%M:%S").to_string(),
        rows,
    })
}

#[derive(Clone, Debug)]
pub struct State {
    local_text: String,
    source_zone: String,
    target_text: String,
    targets: Vec<Tz>,
    ambiguous_choice: Option<AmbiguousChoice>,
    result: Option<Conversion>,
    message: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            local_text: "2026-11-01 01:30".into(),
            source_zone: "America/New_York".into(),
            target_text: "Asia/Shanghai".into(),
            targets: vec![chrono_tz::Asia::Shanghai, chrono_tz::Europe::London],
            ambiguous_choice: None,
            result: None,
            message: String::new(),
        }
    }
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.local_text = "2026-10-17 10:30".into();
        self.source_zone = "Asia/Shanghai".into();
        self.target_text = "America/New_York".into();
        self.targets = vec![chrono_tz::Europe::London, chrono_tz::America::New_York];
        self.ambiguous_choice = None;
        self.message.clear();
        self.convert();
    }

    fn invalidate(&mut self) {
        self.result = None;
    }

    fn parse_input(&self) -> Result<(NaiveDateTime, Tz), String> {
        let local = NaiveDateTime::parse_from_str(&self.local_text, "%Y-%m-%d %H:%M")
            .or_else(|_| NaiveDateTime::parse_from_str(&self.local_text, "%Y-%m-%d %H:%M:%S"))
            .map_err(|_| "时间格式应为 YYYY-MM-DD HH:MM[:SS]".to_owned())?;
        let zone = self
            .source_zone
            .trim()
            .parse::<Tz>()
            .map_err(|_| "请输入有效的 IANA 时区，例如 Asia/Shanghai".to_owned())?;
        Ok((local, zone))
    }

    fn add_target(&mut self) {
        let target = match self.target_text.trim().parse::<Tz>() {
            Ok(target) => target,
            Err(_) => {
                self.message = "请输入有效的 IANA 时区，例如 Europe/London".into();
                return;
            }
        };
        if self.targets.contains(&target) {
            self.message = "该目的时区已在列表中".into();
        } else if self.targets.len() >= MAX_TARGET_ZONES {
            self.message = format!("最多添加 {MAX_TARGET_ZONES} 个目的时区");
        } else {
            self.targets.push(target);
            self.result = None;
            self.message.clear();
        }
    }

    fn convert(&mut self) {
        match self.parse_input().and_then(|(local, source)| {
            convert_local(local, source, &self.targets, self.ambiguous_choice)
        }) {
            Ok(result) => {
                self.result = Some(result);
                self.message.clear();
            }
            Err(error) => {
                self.result = None;
                self.message = error;
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_choice_and_utc(&self) -> Option<(AmbiguousChoice, String)> {
        Some((
            self.ambiguous_choice?,
            self.result.as_ref()?.utc.to_rfc3339(),
        ))
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_ui_state(&self) -> (Option<AmbiguousChoice>, Option<String>, String) {
        (
            self.ambiguous_choice,
            self.result.as_ref().map(|result| result.utc.to_rfc3339()),
            self.message.clone(),
        )
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("时区工作台");
        ui.label(
            "按 IANA 时区转换本地时间；夏令时重叠或缺失时间会明确提示，不用固定 UTC 偏移猜测。",
        );
        ui.horizontal(|ui| {
            ui.label("本地时间");
            if ui.text_edit_singleline(&mut self.local_text).changed() {
                self.invalidate();
                self.ambiguous_choice = None;
            }
            ui.label("来源时区");
            if ui.text_edit_singleline(&mut self.source_zone).changed() {
                self.invalidate();
                self.ambiguous_choice = None;
            }
        });
        ui.small("时间格式：YYYY-MM-DD HH:MM[:SS]；时区使用 IANA 名称，如 America/New_York。");
        match self.parse_input() {
            Ok((local, zone)) => {
                match resolve_local_status(zone, local) {
                    LocalTimeStatus::Unique(_) => {}
                    LocalTimeStatus::Ambiguous { earlier, later } => {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "该时间因夏令时结束而出现两次，请选取其一：",
                        );
                        ui.horizontal(|ui| {
                            let earlier_response = ui.radio_value(
                                &mut self.ambiguous_choice,
                                Some(AmbiguousChoice::Earlier),
                                format!("较早 {}", format_source_offset(zone, earlier)),
                            );
                            let later_response = ui.radio_value(
                                &mut self.ambiguous_choice,
                                Some(AmbiguousChoice::Later),
                                format!("较晚 {}", format_source_offset(zone, later)),
                            );
                            #[cfg(feature = "ui-preview")]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    egui::Id::new("time-zone-earlier-choice"),
                                    earlier_response.rect,
                                );
                                data.insert_temp(
                                    egui::Id::new("time-zone-later-choice"),
                                    later_response.rect,
                                );
                            });
                            let earlier_changed = earlier_response.changed();
                            let later_changed = later_response.changed();
                            if earlier_changed || later_changed {
                                self.invalidate();
                            }
                        });
                    }
                    LocalTimeStatus::Nonexistent {
                        previous_valid,
                        next_valid,
                    } => {
                        let previous = previous_valid
                            .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "—".into());
                        let next = next_valid
                            .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "—".into());
                        ui.colored_label(ui.visuals().warn_fg_color, format!("该时间因夏令时开始而不存在；附近有效时间：之前 {previous}，之后 {next}"));
                    }
                }
            }
            Err(error) => {
                ui.weak(error);
            }
        }
        ui.separator();
        ui.strong("目的时区");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.target_text);
            if ui.button("添加").clicked() {
                self.add_target();
            }
        });
        let mut remove_index = None;
        for (index, zone) in self.targets.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(zone.name());
                if ui.small_button("移除").clicked() {
                    remove_index = Some(index);
                }
            });
        }
        if let Some(index) = remove_index {
            self.targets.remove(index);
            self.invalidate();
        }
        let enabled = !self.targets.is_empty() && self.targets.len() <= MAX_TARGET_ZONES;
        let convert_response = ui.add_enabled(enabled, egui::Button::new("转换时间"));
        #[cfg(feature = "ui-preview")]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("time-zone-convert-button"),
                convert_response.rect,
            );
        });
        if convert_response.clicked() {
            self.convert();
        }
        if !self.message.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.message);
        }
        if let Some(result) = &self.result {
            ui.separator();
            let source_abbreviation = result.utc.with_timezone(&result.source_zone).format("%Z");
            ui.strong(format!(
                "来源：{} {} {}",
                result.source_zone.name(),
                result.source_local,
                source_abbreviation
            ));
            ui.label(format!(
                "对应 UTC：{}",
                result.utc.format("%Y-%m-%d %H:%M:%S UTC")
            ));
            for row in &result.rows {
                ui.horizontal(|ui| {
                    ui.monospace(row.zone.name());
                    ui.label(format!("{} {} {}", row.local, row.offset, row.abbreviation));
                    if ui.small_button("复制").clicked() {
                        ui.ctx().copy_text(format!(
                            "{} {} {} {}",
                            row.zone.name(),
                            row.local,
                            row.offset,
                            row.abbreviation
                        ));
                    }
                });
            }
            ui.small(format!(
                "时区规则数据库：IANA {}",
                chrono_tz::IANA_TZDB_VERSION
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn local(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn converts_one_source_time_to_multiple_zones_with_date_rollover() {
        let result = convert_local(
            local(2026, 1, 1, 9, 0),
            chrono_tz::Asia::Shanghai,
            &[chrono_tz::America::Los_Angeles, chrono_tz::Europe::London],
            None,
        )
        .unwrap();
        assert_eq!(result.utc.to_rfc3339(), "2026-01-01T01:00:00+00:00");
        assert_eq!(result.rows[0].local, "2025-12-31 17:00:00");
        assert_eq!(result.rows[0].offset, "-08:00");
        assert_eq!(result.rows[1].local, "2026-01-01 01:00:00");
        assert_eq!(result.rows[1].offset, "+00:00");
    }

    #[test]
    fn repeated_dst_time_requires_an_explicit_earlier_or_later_choice() {
        let time = local(2026, 11, 1, 1, 30);
        let zone = chrono_tz::America::New_York;
        let LocalTimeStatus::Ambiguous { earlier, later } = resolve_local_status(zone, time) else {
            panic!("expected a repeated local time")
        };
        assert_eq!(earlier.to_rfc3339(), "2026-11-01T05:30:00+00:00");
        assert_eq!(later.to_rfc3339(), "2026-11-01T06:30:00+00:00");
        assert_eq!(format_source_offset(zone, earlier), "-04:00");
        assert_eq!(format_source_offset(zone, later), "-05:00");
        assert!(convert_local(time, zone, &[chrono_tz::UTC], None).is_err());
        assert_eq!(
            convert_local(time, zone, &[chrono_tz::UTC], Some(AmbiguousChoice::Later))
                .unwrap()
                .utc,
            later
        );
    }

    #[test]
    fn skipped_dst_time_reports_nearest_valid_wall_times() {
        let time = local(2026, 3, 8, 2, 30);
        let LocalTimeStatus::Nonexistent {
            previous_valid,
            next_valid,
        } = resolve_local_status(chrono_tz::America::New_York, time)
        else {
            panic!("expected a skipped local time")
        };
        assert_eq!(previous_valid, Some(local(2026, 3, 8, 1, 59)));
        assert_eq!(next_valid, Some(local(2026, 3, 8, 3, 0)));
        assert!(
            convert_local(time, chrono_tz::America::New_York, &[chrono_tz::UTC], None)
                .unwrap_err()
                .contains("不存在")
        );
    }

    #[test]
    fn skipped_calendar_day_finds_both_nearest_valid_minutes() {
        let time = local(2011, 12, 30, 12, 0);
        let LocalTimeStatus::Nonexistent {
            previous_valid,
            next_valid,
        } = resolve_local_status(chrono_tz::Pacific::Apia, time)
        else {
            panic!("expected the skipped calendar day to be nonexistent")
        };
        assert_eq!(previous_valid, Some(local(2011, 12, 29, 23, 59)));
        assert_eq!(next_valid, Some(local(2011, 12, 31, 0, 0)));
    }

    #[test]
    fn rendering_an_ambiguous_time_keeps_the_selected_result() {
        let mut state = State {
            ambiguous_choice: Some(AmbiguousChoice::Earlier),
            ..State::default()
        };
        state.convert();
        assert!(state.result.is_some());

        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| state.ui(ui));
        });
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| state.ui(ui));
        });

        assert_eq!(state.ambiguous_choice, Some(AmbiguousChoice::Earlier));
        assert!(state.result.is_some());
    }

    #[test]
    fn rejects_empty_or_excessive_destination_sets() {
        assert!(convert_local(local(2026, 1, 1, 0, 0), chrono_tz::UTC, &[], None).is_err());
        let zones = vec![chrono_tz::UTC; MAX_TARGET_ZONES + 1];
        assert!(convert_local(local(2026, 1, 1, 0, 0), chrono_tz::UTC, &zones, None).is_err());
    }
}
