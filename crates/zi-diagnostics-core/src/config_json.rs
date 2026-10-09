//! Fast JSON configurations; YAML fallback preserves legacy flow mapping support.
use serde_json::Value;
pub fn parse(input: &str) -> anyhow::Result<Value> {
    match serde_json::from_str::<Value>(input) {
        Ok(value) => Ok(value),
        Err(_) => Ok(serde_yaml_ng::from_str(input)?),
    }
}
