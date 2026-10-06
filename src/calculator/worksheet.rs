use super::{Value, matrix_ui, valid_variable};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path, sync::atomic::AtomicBool};

pub(super) const LIMIT: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Number {
    Exact {
        numerator: String,
        denominator: String,
    },
    Approx {
        value: String,
    },
}
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        validate_value(*self).map_err(serde::ser::Error::custom)?;
        match self {
            Self::Exact(n, d) => Number::Exact {
                numerator: n.to_string(),
                denominator: d.to_string(),
            },
            Self::Approx(value) => Number::Approx {
                value: format!("{value:e}"),
            },
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let parse = |s: String| -> Result<i128, String> {
            if s.len() > 40 {
                return Err("精确整数文本过长".into());
            }
            let n = s.parse::<i128>().map_err(|_| "精确整数超范围")?;
            if n.to_string() != s {
                return Err("精确整数必须是规范十进制字符串".into());
            }
            Ok(n)
        };
        let value = match Number::deserialize(deserializer)? {
            Number::Exact {
                numerator,
                denominator,
            } => Self::Exact(
                parse(numerator).map_err(serde::de::Error::custom)?,
                parse(denominator).map_err(serde::de::Error::custom)?,
            ),
            Number::Approx { value } => {
                if value.len() > 64 {
                    return Err(serde::de::Error::custom("近似数值文本过长"));
                }
                Self::Approx(value.parse::<f64>().map_err(serde::de::Error::custom)?)
            }
        };
        validate_value(value).map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
fn validate_value(value: Value) -> Result<(), String> {
    match value {
        Value::Exact(n, d) if d > 0 && Value::exact(n, d)? == value => Ok(()),
        Value::Approx(x) if x.is_finite() => Ok(()),
        _ => Err("工作表数值必须是约分后的正分母有理数或有限近似值".into()),
    }
}
fn variables<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Value>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, Value>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("最多64个独立变量")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((name, value)) = map.next_entry::<String, Value>()? {
                if result.len() >= 64
                    || result.contains_key(&name)
                    || !(valid_variable(&name) || name == "ans")
                {
                    return Err(serde::de::Error::custom("变量名重复、保留或超出64个限制"));
                }
                result.insert(name, value);
            }
            Ok(result)
        }
    }
    d.deserialize_map(Visitor)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Data {
    pub name: String,
    pub expression: String,
    pub degrees: bool,
    pub matrix_mode: bool,
    #[serde(deserialize_with = "variables")]
    pub variables: BTreeMap<String, Value>,
    pub history: Vec<(String, Value)>,
    pub matrix: matrix_ui::Saved,
}
impl Data {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "工作表名称需为1–128字节且不含控制字符"
        );
        ensure!(
            self.expression.len() <= 2048
                && self.variables.len() <= 64
                && self.history.len() <= 100,
            "表达式、变量或历史超过工作表限制"
        );
        for (name, value) in &self.variables {
            ensure!(valid_variable(name) || name == "ans", "工作表变量名无效");
            validate_value(*value).map_err(anyhow::Error::msg)?;
        }
        for (expression, value) in &self.history {
            ensure!(expression.len() <= 2048, "历史表达式过长");
            validate_value(*value).map_err(anyhow::Error::msg)?;
        }
        self.matrix.validate().map_err(anyhow::Error::msg)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    format: String,
    schema: u32,
    pub tool_version: String,
    pub created_utc: i64,
    pub data: Data,
}
impl Document {
    pub fn new(data: Data) -> Result<Self> {
        data.validate()?;
        Ok(Self {
            format: "zi-devtools-calculator".into(),
            schema: 2,
            tool_version: "0.4.0".into(),
            created_utc: chrono::Utc::now().timestamp(),
            data,
        })
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.format == "zi-devtools-calculator" && matches!(self.schema, 1 | 2),
            "不支持的计算工作表格式或版本"
        );
        ensure!(
            self.tool_version.len() <= 32
                && !self.tool_version.is_empty()
                && self
                    .tool_version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
            "工具版本无效"
        );
        ensure!(
            chrono::DateTime::from_timestamp(self.created_utc, 0).is_some(),
            "工作表UTC时间无效"
        );
        self.data.validate()
    }
}
fn extension(path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("json")),
        "请选择.json计算工作表"
    );
    Ok(())
}
pub(super) fn read(path: &Path) -> Result<Document> {
    extension(path)?;
    let file = crate::local_files::open_regular(path, LIMIT)?;
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "工作表超过1MiB");
    let doc: Document = serde_json::from_slice(&bytes).context("工作表JSON无效")?;
    doc.validate()?;
    Ok(doc)
}
pub(super) fn save(path: &Path, data: Data) -> Result<()> {
    extension(path)?;
    let bytes = serde_json::to_vec_pretty(&Document::new(data)?)?;
    ensure!(bytes.len() <= LIMIT, "工作表超过1MiB");
    crate::local_files::save_new(path, &bytes, &AtomicBool::new(false))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data() -> Data {
        Data {
            name: "合成工作表".into(),
            expression: "1/3".into(),
            degrees: true,
            matrix_mode: true,
            variables: BTreeMap::from([
                ("ans".into(), Value::Exact(i128::MAX, 1)),
                ("third".into(), Value::Exact(1, 3)),
                ("approx".into(), Value::Approx(0.5)),
            ]),
            history: vec![("previous = 1/0".into(), Value::Exact(7, 1))],
            matrix: matrix_ui::State::default().snapshot(),
        }
    }
    #[test]
    fn legacy_worksheet_without_typed_cells_remains_readable() {
        let data = super::super::State::default().snapshot();
        let mut doc = serde_json::to_value(Document::new(data).unwrap()).unwrap();
        doc["schema"] = 1.into();
        doc["tool_version"] = "0.3.0".into();
        for input in ["a", "b"] {
            doc["data"]["matrix"][input]
                .as_object_mut()
                .unwrap()
                .remove("typed");
        }
        let doc: Document = serde_json::from_value(doc).unwrap();
        doc.validate().unwrap();
    }
    #[test]
    fn exact_values_and_unsolved_drafts_roundtrip_without_execution_or_overwrite() {
        let root = std::env::temp_dir().join(format!("zi-worksheet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("worksheet.json");
        let original = data();
        save(&path, original.clone()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains(&format!("\"{}\"", i128::MAX)));
        assert_eq!(read(&path).unwrap().data, original);
        assert!(save(&path, data()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn untrusted_protocol_and_number_invariants_are_rejected() {
        for text in [
            r#"{"kind":"exact","numerator":"2","denominator":"4"}"#,
            r#"{"kind":"exact","numerator":"1","denominator":"0"}"#,
            r#"{"kind":"exact","numerator":"01","denominator":"1"}"#,
            r#"{"kind":"exact","numerator":"1","denominator":"-1"}"#,
            r#"{"kind":"approx","value":1e309}"#,
            r#"{"kind":"approx","value":1,"extra":true}"#,
        ] {
            assert!(serde_json::from_str::<Value>(text).is_err(), "{text}");
        }
        let mut doc = serde_json::to_value(Document::new(data()).unwrap()).unwrap();
        doc["schema"] = 3.into();
        assert!(
            serde_json::from_value::<Document>(doc.clone())
                .unwrap()
                .validate()
                .is_err()
        );
        doc["schema"] = 1.into();
        doc["data"]["matrix"]["a"]["rows"] = 9.into();
        assert!(
            serde_json::from_value::<Document>(doc.clone())
                .unwrap()
                .validate()
                .is_err()
        );
        doc["data"]["extra"] = true.into();
        assert!(serde_json::from_value::<Document>(doc).is_err());
        let duplicate = r#"{"ans":{"kind":"exact","numerator":"1","denominator":"1"},"ans":{"kind":"exact","numerator":"2","denominator":"1"}}"#;
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "variables")]
            v: BTreeMap<String, Value>,
        }
        assert!(serde_json::from_str::<Test>(&format!("{{\"v\":{duplicate}}}")).is_err());
        let okay: Test = serde_json::from_str(r#"{"v":{}}"#).unwrap();
        assert!(okay.v.is_empty());
    }
    #[test]
    fn full_float_precision_roundtrips_even_for_subnormals_and_negative_zero() {
        for number in [
            0.0,
            -0.0,
            f64::from_bits(1),
            f64::MIN_POSITIVE,
            f64::MAX,
            std::f64::consts::PI,
            0.1,
            -1e-250,
        ] {
            let bytes = serde_json::to_vec(&Value::Approx(number)).unwrap();
            let Value::Approx(restored) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
                panic!("approximate type lost")
            };
            assert_eq!(
                number.to_bits(),
                restored.to_bits(),
                "{}",
                String::from_utf8_lossy(&bytes)
            );
        }
        assert!(serde_json::from_str::<Value>(r#"{"kind":"approx","value":"NaN"}"#).is_err());
        assert!(serde_json::from_str::<Value>(r#"{"kind":"approx","value":"1e309"}"#).is_err());
    }
    #[test]
    fn oversized_file_or_content_is_not_loaded() {
        let root =
            std::env::temp_dir().join(format!("zi-worksheet-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("large.json");
        std::fs::write(&path, vec![b' '; LIMIT + 1]).unwrap();
        assert!(read(&path).is_err());
        let mut bad = data();
        bad.history = vec![("1".into(), Value::Exact(1, 1)); 101];
        assert!(Document::new(bad).is_err());
        assert!(save(&root.join("wrong.txt"), data()).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
