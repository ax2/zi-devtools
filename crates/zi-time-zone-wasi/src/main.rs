use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use std::{
    fmt::Write as _,
    io::{Read, Write},
};
use zi_time_zone_core::{
    AmbiguousChoice, IANA_TZDB_VERSION, LocalTimeStatus, MAX_TARGET_ZONES, Tz, convert_local,
    format_source_offset, parse_local_time, resolve_local_status,
};

const PLUGIN: &str = "com.zicode.devtools.time-zone";
const CAPABILITY: &str = "devtools.time-zone.convert";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_REQUEST: usize = 8192;
const MAX_RESULT: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request<'a> {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    #[serde(borrow)]
    input: &'a RawValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Input {
    local_time: String,
    source_zone: String,
    target_zones: Vec<String>,
    ambiguous_choice: Option<Choice>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Choice {
    Earlier,
    Later,
}

struct Failure(&'static str, &'static str);

const BAD_REQUEST: Failure = Failure("INVALID_REQUEST", "时区请求结构无效。");
const BAD_INPUT: Failure = Failure("INVALID_INPUT", "请检查源时间、时区和重叠时间设置。");
const BAD_DATE: Failure = Failure(
    "INVALID_LOCAL_TIME",
    "请使用 YYYY-MM-DD HH:MM[:SS] 格式输入有效本地时间。",
);
const BAD_SOURCE: Failure = Failure(
    "INVALID_SOURCE_ZONE",
    "请输入有效的来源 IANA 时区，例如 Asia/Shanghai。",
);
const BAD_TARGET: Failure = Failure(
    "INVALID_TARGET_ZONE",
    "请输入有效的目的 IANA 时区，例如 Europe/London。",
);

fn parse_zone(value: &str, invalid: Failure) -> Result<Tz, Failure> {
    if value.len() > 64 {
        return Err(invalid);
    }
    value.trim().parse().map_err(|_| invalid)
}

fn execute(request: Request<'_>) -> Result<Value, Failure> {
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
    {
        return Err(BAD_REQUEST);
    }
    if request.capability_id != CAPABILITY {
        return Err(Failure("UNKNOWN_OPERATION", "未知时区操作。"));
    }
    let input: Input = serde_json::from_str(request.input.get()).map_err(|_| BAD_INPUT)?;
    if input.local_time.len() > 32 {
        return Err(BAD_DATE);
    }
    let local = parse_local_time(&input.local_time).map_err(|_| BAD_DATE)?;
    let source_zone = parse_zone(&input.source_zone, BAD_SOURCE)?;
    if !(1..=MAX_TARGET_ZONES).contains(&input.target_zones.len()) {
        return Err(Failure("TARGET_COUNT", "请输入 1 至 8 个目的时区。"));
    }
    let mut targets = Vec::with_capacity(input.target_zones.len());
    for value in input.target_zones {
        let zone = parse_zone(&value, BAD_TARGET)?;
        if targets.contains(&zone) {
            return Err(Failure("DUPLICATE_TARGET", "目的时区列表中有重复项。"));
        }
        targets.push(zone);
    }

    let choice = input.ambiguous_choice.map(|choice| match choice {
        Choice::Earlier => AmbiguousChoice::Earlier,
        Choice::Later => AmbiguousChoice::Later,
    });
    let source_local = local.format("%Y-%m-%d %H:%M:%S").to_string();
    let source_name = source_zone.name();
    let mut data = json!({
        "tzdbVersion":IANA_TZDB_VERSION, "sourceZone":source_name, "sourceLocal":source_local,
        "status":"unique", "utc":null, "sourceOffset":null, "sourceAbbreviation":null,
        "earlier":null, "later":null, "selectedChoice":null,
        "previousValid":null, "nextValid":null, "rows":[]
    });
    let mut text = format!("{source_name} {source_local} · IANA {IANA_TZDB_VERSION}");
    match resolve_local_status(source_zone, local) {
        LocalTimeStatus::Unique(_) => {}
        LocalTimeStatus::Ambiguous { earlier, later } => {
            let early_offset = format_source_offset(source_zone, earlier);
            let late_offset = format_source_offset(source_zone, later);
            let early_utc = earlier.to_rfc3339();
            let late_utc = later.to_rfc3339();
            data["status"] = json!("ambiguous");
            data["earlier"] = json!({"utc":early_utc,"offset":early_offset});
            data["later"] = json!({"utc":late_utc,"offset":late_offset});
            write!(text, "\n该本地时间出现两次\n较早 earlier：{early_utc} ({early_offset})\n较晚 later：{late_utc} ({late_offset})").unwrap();
            data["selectedChoice"] = match choice {
                Some(AmbiguousChoice::Earlier) => json!("earlier"),
                Some(AmbiguousChoice::Later) => json!("later"),
                None => {
                    write!(text, "\n请选择较早或较晚的时刻后再执行换算。").unwrap();
                    data["text"] = json!(text);
                    return Ok(data);
                }
            };
        }
        LocalTimeStatus::Nonexistent {
            previous_valid,
            next_valid,
        } => {
            let previous = previous_valid.map(|value| value.format("%Y-%m-%d %H:%M").to_string());
            let next = next_valid.map(|value| value.format("%Y-%m-%d %H:%M").to_string());
            write!(
                text,
                "\n该本地时间不存在\n之前最近的有效分钟：{}\n之后最近的有效分钟：{}",
                previous.as_deref().unwrap_or("无"),
                next.as_deref().unwrap_or("无")
            )
            .unwrap();
            data["status"] = json!("nonexistent");
            data["previousValid"] = json!(previous);
            data["nextValid"] = json!(next);
            data["text"] = json!(text);
            return Ok(data);
        }
    }

    let conversion = convert_local(local, source_zone, &targets, choice)
        .map_err(|_| Failure("CONVERSION_FAILED", "无法换算该时间。"))?;
    let utc = conversion.utc.to_rfc3339();
    let offset = format_source_offset(source_zone, conversion.utc);
    let abbreviation = conversion
        .utc
        .with_timezone(&source_zone)
        .format("%Z")
        .to_string();
    write!(text, "\nUTC：{utc} · 来源 {offset} {abbreviation}").unwrap();
    data["utc"] = json!(utc);
    data["sourceOffset"] = json!(offset);
    data["sourceAbbreviation"] = json!(abbreviation);
    let mut rows = Vec::with_capacity(conversion.rows.len());
    for row in conversion.rows {
        let zone = row.zone.name();
        write!(
            text,
            "\n{zone}  {}  {} {}",
            row.local, row.offset, row.abbreviation
        )
        .unwrap();
        rows.push(json!({"zone":zone,"local":row.local,"offset":row.offset,"abbreviation":row.abbreviation}));
    }
    data["rows"] = json!(rows);
    data["text"] = json!(text);
    Ok(data)
}

fn run() -> Result<Value, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| BAD_REQUEST)?;
    if bytes.len() > MAX_REQUEST {
        return Err(Failure("INPUT_TOO_LARGE", "请求超过 8 KiB。"));
    }
    execute(serde_json::from_slice(&bytes).map_err(|_| BAD_REQUEST)?)
}

fn error(Failure(code, message): Failure) -> Value {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() -> std::io::Result<()> {
    let value = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&value).expect("valid time zone result");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(Failure("RESULT_TOO_LARGE", "时区结果超过 4 KiB。")))
            .expect("fixed time zone error");
    }
    std::io::stdout().write_all(&bytes)
}
