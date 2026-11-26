//! Shared IANA conversion for ZiDevTools and the Studio Worker.
use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Utc};
pub use chrono_tz::{IANA_TZDB_VERSION, Tz};

pub fn parse_local_time(value: &str) -> Result<NaiveDateTime, String> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
        .map_err(|_| "时间格式应为 YYYY-MM-DD HH:MM[:SS]".to_owned())
}

pub const MAX_TARGET_ZONES: usize = 8;
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

pub fn format_source_offset(zone: Tz, utc: DateTime<Utc>) -> String {
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
