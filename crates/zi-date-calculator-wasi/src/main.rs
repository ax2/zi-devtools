//! Offline arithmetic over user-entered Gregorian dates.

use chrono::NaiveDate;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io::Read;
use zi_civil_date_core::{
    Unit, count_weekdays, difference, offset, parse, weekday_name, weekday_offset,
};

const REQUEST_LIMIT: usize = 4 * 1024;
const CONTRACT: &str = "1.0.0-rc.1";
const PLUGIN: &str = "com.zicode.devtools.date-calculator";

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Difference {
    start: String,
    end: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WorkdayCount {
    start: String,
    end: String,
    #[serde(default)]
    include_start: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Offset {
    start: String,
    amount: i64,
    unit: Unit,
    #[serde(default)]
    clamp: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkdayOffset {
    start: String,
    amount: i64,
}

struct Failure(&'static str, String);

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, Failure> {
    serde_json::from_value(value)
        .map_err(|_| Failure("INVALID_INPUT", "请检查本项工具的参数。".into()))
}

fn date(value: &str) -> Result<NaiveDate, Failure> {
    parse(value).map_err(|message| Failure("INVALID_DATE", message))
}

fn date_result(day: NaiveDate) -> Value {
    let date = day.to_string();
    let weekday = weekday_name(day);
    json!({"date":date,"weekday":weekday,"text":format!("{date} · {weekday}")})
}

fn compute(request: Request) -> Result<Value, Failure> {
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
    {
        return Err(Failure("INVALID_REQUEST", "请求结构无效。".into()));
    }
    match request.capability_id.as_str() {
        "devtools.date.difference" => {
            let input: Difference = decode(request.input)?;
            let days = difference(date(&input.start)?, date(&input.end)?);
            Ok(json!({"days":days,"text":format!("{days} 天")}))
        }
        "devtools.date.offset" => {
            let input: Offset = decode(request.input)?;
            let day = offset(date(&input.start)?, input.amount, input.unit, input.clamp)
                .map_err(|message| Failure("INVALID_OFFSET", message))?;
            Ok(date_result(day))
        }
        "devtools.date.workday_count" => {
            let input: WorkdayCount = decode(request.input)?;
            let days = count_weekdays(date(&input.start)?, date(&input.end)?, input.include_start)
                .map_err(|message| Failure("INVALID_RANGE", message))?;
            Ok(
                json!({"days":days,"weekdaysOnly":true,"text":format!("{days} 工作日（周一至周五）")}),
            )
        }
        "devtools.date.workday_offset" => {
            let input: WorkdayOffset = decode(request.input)?;
            let day = weekday_offset(date(&input.start)?, input.amount)
                .map_err(|message| Failure("INVALID_OFFSET", message))?;
            let mut value = date_result(day);
            value["weekdaysOnly"] = json!(true);
            Ok(value)
        }
        _ => Err(Failure("UNKNOWN_OPERATION", "未知日期计算操作。".into())),
    }
}

fn execute() -> Result<Value, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "无法读取请求。".into()))?;
    if bytes.len() > REQUEST_LIMIT {
        return Err(Failure("INPUT_TOO_LARGE", "请求超过 4 KiB。".into()));
    }
    let request = serde_json::from_slice(&bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "请求结构无效。".into()))?;
    compute(request)
}

fn main() {
    let result = match execute() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(Failure(code, message)) => {
            json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
        }
    };
    serde_json::to_writer(std::io::stdout().lock(), &result).expect("write date result");
}
