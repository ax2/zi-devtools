//! Bounded iCalendar exchange for schedules representable by the local planner.
use super::*;
use anyhow::{Context, bail};
use chrono::{Timelike, Utc};
use ical::property::Property;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Cursor, Read, Write},
    path::Path,
};

const LIMIT: usize = 16 * 1024 * 1024;
const UID_DOMAIN: &str = "@zi-devtools.zicode.com";
pub(super) struct Import {
    pub records: Vec<Item>,
    pub notices: Vec<String>,
}

fn extension(path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("ics")),
        "请选择 .ics 日历文件"
    );
    Ok(())
}
pub(super) fn read(path: &Path) -> Result<Import> {
    extension(path)?;
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= LIMIT as u64,
        "请选择不超过 16 MiB 的普通 ICS 文件"
    );
    let file = File::open(path)?;
    ensure!(file.metadata()?.is_file(), "所选路径不再是普通文件");
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    parse(std::str::from_utf8(&bytes).context("ICS 必须是 UTF-8 编码")?)
}
pub(super) fn write(path: &Path, text: &str) -> Result<usize> {
    extension(path)?;
    ensure!(text.len() <= LIMIT, "ICS 超过 16 MiB，请缩小导出范围");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("无法创建 ICS；已有文件不会覆盖，请使用新文件名")?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .context("写入未完成，目标可能包含部分内容；请检查并选择新文件名重试")?;
    Ok(text.len())
}

fn text_value(value: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        result.push(match chars.next() {
            Some('n' | 'N') => '\n',
            Some(ch @ ('\\' | ',' | ';')) => ch,
            _ => bail!("ICS 含无效的文本转义"),
        });
    }
    Ok(result)
}
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
fn prop<'a>(props: &'a [Property], name: &str) -> Result<Option<&'a Property>> {
    let mut found = props.iter().filter(|p| p.name == name);
    let value = found.next();
    ensure!(
        found.next().is_none(),
        "ICS 属性 {name} 重复，无法确定使用哪一个"
    );
    Ok(value)
}
fn value<'a>(props: &'a [Property], name: &str) -> Result<Option<&'a str>> {
    Ok(prop(props, name)?.and_then(|p| p.value.as_deref()))
}
fn required<'a>(props: &'a [Property], name: &str) -> Result<&'a str> {
    value(props, name)?
        .filter(|s| !s.is_empty())
        .with_context(|| format!("ICS 缺少 {name}"))
}

/// Preserve whitespace while unfolding, then let ical parse quoted parameters.
/// ical's line reader trims trailing whitespace; a removable suffix prevents it.
fn properties(text: &str) -> Result<Vec<Property>> {
    ensure!(
        text.len() <= LIMIT && !text.contains('\0'),
        "ICS 超过 16 MiB 或包含空字节"
    );
    let mut lines: Vec<String> = Vec::new();
    for (n, raw) in text.trim_start_matches('\u{feff}').split('\n').enumerate() {
        ensure!(n < 100_000, "ICS 行数超过 100000，请拆分文件");
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        ensure!(
            !line.chars().any(|c| c.is_control() && c != '\t'),
            "ICS 包含无效控制字符"
        );
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let last = lines.last_mut().context("ICS 第一行不能是续行")?;
            last.push_str(&line[1..]);
            ensure!(last.len() <= MAX_BODY * 3 + 4096, "ICS 单个属性过大");
        } else {
            ensure!(line.len() <= MAX_BODY * 3 + 4096, "ICS 单个属性过大");
            lines.push(line.to_owned());
        }
    }
    let mut protected = String::new();
    for line in lines {
        protected.push_str(&line);
        protected.push_str("\u{e000}\r\n");
    }
    ical::PropertyParser::from_reader(Cursor::new(protected))
        .map(|p| {
            let mut p = p.context("ICS 属性格式无效")?;
            ensure!(
                !p.name.is_empty()
                    && p.name.len() <= 64
                    && p.name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "ICS 属性名无效"
            );
            let value = p.value.as_mut().context("ICS 属性缺少值")?;
            ensure!(value.pop() == Some('\u{e000}'), "ICS 属性解析失败");
            Ok(p)
        })
        .collect()
}

pub(super) fn parse(text: &str) -> Result<Import> {
    let properties = properties(text)?;
    let mut stack: Vec<String> = Vec::new();
    let mut calendar = Vec::new();
    let mut event = Vec::new();
    let mut records = Vec::new();
    let mut notices = HashSet::new();
    let mut ended = false;
    for p in properties {
        let val = p.value.as_deref().unwrap_or_default();
        if p.name == "BEGIN" {
            ensure!(!ended && p.params.is_none(), "ICS 只能包含一个完整日历");
            let component = val.to_ascii_uppercase();
            let allowed = matches!(
                (stack.last().map(String::as_str), component.as_str()),
                (None, "VCALENDAR") | (Some("VCALENDAR"), "VEVENT") | (Some("VEVENT"), "VALARM")
            );
            ensure!(
                allowed,
                "暂不支持 ICS 组件 {component}；未导入任何记录（仅支持日程，不支持时区定义、任务或邀请）"
            );
            if component == "VEVENT" {
                ensure!(records.len() < MAX_ITEMS, "ICS 超过 2000 个日程");
                event.clear();
            }
            if component == "VALARM" {
                notices.insert("文件中的提醒不导入，新增或更新日程的提醒默认关闭。".to_owned());
            }
            stack.push(component);
        } else if p.name == "END" {
            ensure!(
                p.params.is_none()
                    && stack.pop().as_deref() == Some(val.to_ascii_uppercase().as_str()),
                "ICS 组件结束标记不匹配"
            );
            if val.eq_ignore_ascii_case("VEVENT") {
                let item = parse_event(&event, &mut notices)
                    .with_context(|| format!("第 {} 个日程无法导入", records.len() + 1))?;
                records.push(item);
            }
            if val.eq_ignore_ascii_case("VCALENDAR") {
                ended = true;
            }
        } else {
            match stack.last().map(String::as_str) {
                Some("VCALENDAR") => calendar.push(p),
                Some("VEVENT") => event.push(p),
                Some("VALARM") => {}
                _ => bail!("ICS 日历之外存在内容"),
            }
        }
    }
    ensure!(
        ended && stack.is_empty(),
        "ICS 缺少完整 BEGIN / END，未导入任何记录"
    );
    ensure!(
        required(&calendar, "VERSION")? == "2.0",
        "仅支持 iCalendar 2.0"
    );
    ensure!(
        prop(&calendar, "X-WR-TIMEZONE")?.is_none(),
        "文件带有日历时区提示 X-WR-TIMEZONE，当前不能保证按原时区导入；请导出为单次 UTC 或无时区本地日程"
    );
    for name in ["VERSION", "CALSCALE", "METHOD"] {
        ensure!(
            prop(&calendar, name)?.is_none_or(|p| p.params.is_none()),
            "日历属性 {name} 含不支持的参数"
        );
    }
    ensure!(
        value(&calendar, "CALSCALE")?.is_none_or(|s| s.eq_ignore_ascii_case("GREGORIAN")),
        "仅支持公历 ICS"
    );
    ensure!(
        value(&calendar, "METHOD")?.is_none_or(|s| s.eq_ignore_ascii_case("PUBLISH")),
        "此文件是会议邀请或取消请求；暂不作为普通日程导入"
    );
    ensure!(!records.is_empty(), "ICS 没有可导入的日程");
    let mut ids = HashSet::new();
    ensure!(
        records.iter().all(|i| ids.insert(&i.id)),
        "ICS 含重复 UID；重复实例 / 例外暂不支持"
    );
    backup::validate_records(&records)?;
    let mut notices: Vec<_> = notices.into_iter().collect();
    notices.sort();
    Ok(Import { records, notices })
}

fn parse_time(p: &Property) -> Result<(NaiveDateTime, bool, bool)> {
    let mut date_only = false;
    if let Some(params) = &p.params {
        ensure!(params.len() <= 1, "{} 含不支持的时间参数", p.name);
        for (key, values) in params {
            ensure!(
                key == "VALUE" && values.len() == 1,
                "{} 的 TZID 或其他参数暂不支持；请导出为浮动本地时间，单次日程也可用 UTC",
                p.name
            );
            match values[0].to_ascii_uppercase().as_str() {
                "DATE" => date_only = true,
                "DATE-TIME" => {}
                _ => bail!("不支持的 ICS 时间类型"),
            }
        }
    }
    let raw = p.value.as_deref().context("缺少时间值")?;
    if date_only {
        ensure!(
            raw.len() == 8 && raw.bytes().all(|b| b.is_ascii_digit()),
            "全天日期需要 YYYYMMDD"
        );
        return Ok((
            NaiveDate::parse_from_str(raw, "%Y%m%d")?.and_time(NaiveTime::MIN),
            true,
            false,
        ));
    }
    let utc = raw.ends_with('Z');
    let raw = raw.strip_suffix('Z').unwrap_or(raw);
    ensure!(
        raw.len() == 15 && raw.as_bytes()[8] == b'T',
        "日程时间需要 YYYYMMDDTHHMMSS，可附加 Z 表示 UTC"
    );
    let at = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S")?;
    ensure!(
        at.second() == 0 && at.nanosecond() == 0,
        "日程编辑精度为分钟；请先将非零秒时间调整为整分钟"
    );
    let at = if utc {
        at.and_utc().with_timezone(&Local).naive_local()
    } else {
        at
    };
    ensure!(
        at.second() == 0 && at.nanosecond() == 0,
        "时区换算后不是整分钟，本机编辑器无法准确保留该时间"
    );
    ensure!(
        Local.from_local_datetime(&at).single().is_some(),
        "导入时间在本机时区不存在或存在夏令时歧义"
    );
    Ok((at, false, utc))
}
fn uid_id(uid: &str) -> String {
    if let Some(id) = uid.strip_suffix(UID_DOMAIN)
        && let Ok(id) = uuid::Uuid::parse_str(id)
        && format!("{id}{UID_DOMAIN}") == uid
    {
        return id.to_string();
    }
    let digest = Sha256::digest(format!("zi-devtools:ical:{uid}"));
    let mut bytes: [u8; 16] = digest[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 15) | 0x80;
    bytes[8] = (bytes[8] & 63) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}
fn parse_event(props: &[Property], notices: &mut HashSet<String>) -> Result<Item> {
    for p in props {
        let expected = match p.name.as_str() {
            "UID" | "SUMMARY" | "DESCRIPTION" | "LOCATION" | "STATUS" => Some("TEXT"),
            "RRULE" => Some("RECUR"),
            "URL" => Some("URI"),
            _ => None,
        };
        if let Some(expected) = expected
            && let Some(params) = &p.params
        {
            let mut seen = HashSet::new();
            for (key, values) in params {
                ensure!(
                    seen.insert(key) && values.len() == 1,
                    "{} 的参数无效或重复",
                    p.name
                );
                match key.as_str() {
                    "VALUE" => ensure!(
                        values[0].eq_ignore_ascii_case(expected),
                        "{} 的值类型不受支持",
                        p.name
                    ),
                    "LANGUAGE" | "ALTREP"
                        if ["SUMMARY", "DESCRIPTION", "LOCATION"].contains(&p.name.as_str()) =>
                    {
                        notices
                            .insert("文本保留原文；语言标签和替代表示链接不导入、不打开。".into());
                    }
                    _ => bail!("{} 含不支持的参数 {key}，未导入任何记录", p.name),
                }
            }
        }
    }
    for name in ["RECURRENCE-ID", "EXDATE", "RDATE", "EXRULE", "DURATION"] {
        ensure!(
            prop(props, name)?.is_none(),
            "暂不支持 {name}，不会将有限或例外日程错误改为无限重复；未导入任何记录"
        );
    }
    ensure!(
        value(props, "STATUS")?.is_none_or(|s| s.eq_ignore_ascii_case("CONFIRMED")),
        "暂不导入取消 / 暂定日程，请先在原日历中确认状态"
    );
    let uid = text_value(required(props, "UID")?)?;
    let start_prop = prop(props, "DTSTART")?.context("ICS 缺少 DTSTART")?;
    let (start, all_day, utc) = parse_time(start_prop)?;
    let mut item = Item::new(Some(start.date()));
    item.id = uid_id(&uid);
    item.calendar_uid = Some(uid);
    item.revision = 1;
    item.updated = Utc::now().timestamp();
    item.title = text_value(required(props, "SUMMARY")?)?;
    item.body = text_value(value(props, "DESCRIPTION")?.unwrap_or_default())?;
    for (key, label) in [("LOCATION", "地点"), ("URL", "链接")] {
        if let Some(raw) = value(props, key)? {
            let text = if key == "LOCATION" {
                text_value(raw)?
            } else {
                raw.to_owned()
            };
            if !text.is_empty() {
                if !item.body.is_empty() {
                    item.body.push_str("\n\n");
                }
                item.body.push_str(&format!("{label}：{text}"));
                notices.insert("地点和链接保留为详情中的纯文本，不会打开链接。".into());
            }
        }
    }
    let s = item.schedule.as_mut().unwrap();
    s.start = start;
    s.all_day = all_day;
    s.remind = false;
    s.minutes = 0;
    s.reminder_time = all_day.then(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    s.end = if let Some(end) = prop(props, "DTEND")? {
        let (end, end_day, end_utc) = parse_time(end)?;
        ensure!(
            end_day == all_day && end_utc == utc,
            "开始与结束需要使用相同的日期 / 时间类型"
        );
        Some(end)
    } else if all_day {
        start.checked_add_signed(Duration::days(1))
    } else {
        None
    };
    if let Some(rule) = value(props, "RRULE")? {
        ensure!(
            !utc,
            "UTC 重复日程暂不导入：转换为本机钟表重复会改变夏令时后的安排"
        );
        parse_rule(s, rule)?;
    }
    if utc {
        notices.insert("单次 UTC 时间已换算为本机时区；导入后按本机钟表安排。".into());
    }
    let known = [
        "UID",
        "DTSTART",
        "DTEND",
        "SUMMARY",
        "DESCRIPTION",
        "LOCATION",
        "URL",
        "RRULE",
        "STATUS",
        "DTSTAMP",
        "CREATED",
        "LAST-MODIFIED",
        "SEQUENCE",
    ];
    let extras: std::collections::BTreeSet<_> = props
        .iter()
        .filter(|p| !known.contains(&p.name.as_str()))
        .map(|p| p.name.clone())
        .collect();
    if !extras.is_empty() {
        notices.insert(format!(
            "不导入附加属性：{}；不会发送邀请、下载附件或执行链接。",
            extras.into_iter().collect::<Vec<_>>().join("、")
        ));
    }
    item.validate()?;
    Ok(item)
}

fn parse_rule(s: &mut Schedule, rule: &str) -> Result<()> {
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
    if let Some(until) = parts.remove("UNTIL") {
        ensure!(
            until
                == if s.all_day {
                    "20991231"
                } else {
                    "20991231T235959"
                },
            "暂不支持自定义截止时间 / 次数，不会改为无限重复"
        );
    }
    s.repeat = match parts.remove("FREQ").as_deref() {
        Some("DAILY") => Repeat::Daily,
        Some("WEEKLY") => Repeat::Weekly,
        Some("MONTHLY") => Repeat::Monthly,
        Some("YEARLY") => Repeat::Yearly,
        _ => bail!("暂不支持此重复频率"),
    };
    if s.repeat == Repeat::Weekly {
        if let Some(day) = parts.remove("BYDAY") {
            ensure!(
                day == ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
                    [s.start.weekday().num_days_from_monday() as usize],
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
    if s.repeat == Repeat::Yearly {
        if let Some(month) = parts.remove("BYMONTH") {
            ensure!(month == s.start.month().to_string(), "暂不支持多个重复月份");
        } else {
            ensure!(
                !parts.contains_key("BYMONTHDAY"),
                "每年按日号重复必须明确限定起始月份，否则会增加额外日期"
            );
        }
    }
    if matches!(s.repeat, Repeat::Monthly | Repeat::Yearly)
        && let Some(days) = parts.remove("BYMONTHDAY")
    {
        let clamped = (28..=s.start.day())
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if s.start.day() > 28
            && days == clamped
            && parts.get("BYSETPOS").map(String::as_str) == Some("-1")
        {
            parts.remove("BYSETPOS");
            s.clamp_missing_day = true;
        } else {
            ensure!(days == s.start.day().to_string(), "暂不支持多个重复日号");
        }
    }
    ensure!(
        parts.is_empty(),
        "RRULE 含当前无法保留的条件（如 COUNT / BYDAY / BYSETPOS），未导入任何记录"
    );
    Ok(())
}

fn line(out: &mut String, text: &str) {
    let mut column = 0;
    for ch in text.chars() {
        if column + ch.len_utf8() > 75 {
            out.push_str("\r\n ");
            column = 1;
        }
        out.push(ch);
        column += ch.len_utf8();
    }
    out.push_str("\r\n");
}
pub(super) fn export(items: &[Item], reminders: bool) -> Result<String> {
    ensure!(!items.is_empty(), "当前范围没有可导出的日程");
    let mut out = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//ZiCode//Zi DevTools//ZH\r\nCALSCALE:GREGORIAN\r\n",
    );
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    for item in items {
        item.validate()?;
        let s = item.schedule.as_ref().context("只能导出日程")?;
        ensure!(!item.trash, "回收站日程需先恢复再导出");
        line(&mut out, "BEGIN:VEVENT");
        let uid = item
            .calendar_uid
            .clone()
            .unwrap_or_else(|| format!("{}{UID_DOMAIN}", item.id));
        line(&mut out, &format!("UID:{}", escape(&uid)));
        line(&mut out, &format!("DTSTAMP:{stamp}"));
        line(&mut out, &format!("SUMMARY:{}", escape(&item.title)));
        line(&mut out, &format!("DESCRIPTION:{}", escape(&item.body)));
        let format = if s.all_day { "%Y%m%d" } else { "%Y%m%dT%H%M%S" };
        let param = if s.all_day { ";VALUE=DATE" } else { "" };
        line(
            &mut out,
            &format!("DTSTART{param}:{}", s.start.format(format)),
        );
        if let Some(end) = s.end {
            line(&mut out, &format!("DTEND{param}:{}", end.format(format)));
        }
        if s.repeat != Repeat::Once {
            let freq = match s.repeat {
                Repeat::Daily => "DAILY",
                Repeat::Weekly => "WEEKLY",
                Repeat::Monthly => "MONTHLY",
                Repeat::Yearly => "YEARLY",
                Repeat::Once => unreachable!(),
            };
            let mut rule = format!(
                "RRULE:FREQ={freq};UNTIL={}",
                if s.all_day {
                    "20991231"
                } else {
                    "20991231T235959"
                }
            );
            if s.clamp_missing_day && s.start.day() > 28 {
                if s.repeat == Repeat::Yearly {
                    rule.push_str(&format!(";BYMONTH={}", s.start.month()));
                }
                rule.push_str(&format!(
                    ";BYMONTHDAY={};BYSETPOS=-1",
                    (28..=s.start.day())
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            line(&mut out, &rule);
        }
        if reminders && s.remind && !s.done {
            let seconds = i64::from(s.reminder_time.map_or(0, |t| t.num_seconds_from_midnight()))
                - i64::from(s.minutes) * 60;
            line(&mut out, "BEGIN:VALARM");
            line(&mut out, "ACTION:DISPLAY");
            line(
                &mut out,
                &format!(
                    "TRIGGER:{}PT{}S",
                    if seconds < 0 { "-" } else { "" },
                    seconds.abs()
                ),
            );
            line(&mut out, &format!("DESCRIPTION:{}", escape(&item.title)));
            line(&mut out, "END:VALARM");
        }
        line(&mut out, "END:VEVENT");
        ensure!(
            out.len() + 15 <= LIMIT,
            "ICS 超过 16 MiB，请选择单条日程导出"
        );
    }
    line(&mut out, "END:VCALENDAR");
    Ok(out)
}
