//! Per-request numeric options shared by the stateless calculator adapters.
use crate::{Angle, Value, parse_real_literal, valid_variable};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AngleInput {
    #[default]
    Radians,
    Degrees,
}

impl From<AngleInput> for Angle {
    fn from(angle: AngleInput) -> Self {
        match angle {
            AngleInput::Radians => Self::Radians,
            AngleInput::Degrees => Self::Degrees,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Exact(String),
    Approximate(f64),
}

pub fn parse_constants(
    input: BTreeMap<String, Literal>,
    reserve_coordinate: bool,
) -> Result<BTreeMap<String, Value>, String> {
    if input.len() > 64 {
        return Err("最多提供 64 个常量。".into());
    }
    input
        .into_iter()
        .map(|(name, literal)| {
            if !valid_variable(&name) || (reserve_coordinate && name == "x") {
                let reserved = if reserve_coordinate {
                    "x、pi、e、i 和 ans"
                } else {
                    "pi、e、i 和 ans"
                };
                return Err(format!(
                    "常量名须为 ASCII 字母、数字或下划线，并以字母开头。{reserved} 为保留名称。"
                ));
            }
            let value = match literal {
                Literal::Exact(text) if text.len() <= 64 => parse_real_literal(text.trim(), false),
                Literal::Exact(_) => Err("精确常量最多 64 UTF-8 字节".into()),
                Literal::Approximate(number) => Value::approximate(number),
            }
            .map_err(|reason| format!("{name}：{reason}"))?;
            Ok((name, value))
        })
        .collect()
}
