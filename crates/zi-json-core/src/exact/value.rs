use super::number::Number;
use anyhow::{Context, Result, ensure};
use serde::{
    Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use std::{collections::BTreeMap, fmt, slice};

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(super) enum Value<'a> {
    Null,
    Bool(bool),
    Number(Number<'a>),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Canonical<'v, 's> {
    Null,
    Bool(bool),
    Number(Number<'s>),
    String(&'v str),
    Array(Vec<Self>),
    Object(Vec<(&'v str, Self)>),
}

impl<'s> Value<'s> {
    pub(super) fn canonical(&self) -> Canonical<'_, 's> {
        match self {
            Self::Null => Canonical::Null,
            Self::Bool(value) => Canonical::Bool(*value),
            Self::Number(value) => Canonical::Number(*value),
            Self::String(value) => Canonical::String(value),
            Self::Array(values) => {
                let mut values: Vec<_> = values.iter().map(Value::canonical).collect();
                values.sort_unstable();
                Canonical::Array(values)
            }
            Self::Object(values) => Canonical::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.canonical()))
                    .collect(),
            ),
        }
    }
}

struct ReadValue<'iter, 'tokens, 'input>(&'iter mut slice::Iter<'tokens, &'input str>);

impl<'input, 'de> DeserializeSeed<'de> for ReadValue<'_, '_, 'input> {
    type Value = Value<'input>;

    fn deserialize<D: de::Deserializer<'de>>(self, source: D) -> Result<Self::Value, D::Error> {
        source.deserialize_any(self)
    }
}

impl<'input, 'de> Visitor<'de> for ReadValue<'_, '_, 'input> {
    type Value = Value<'input>;

    fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("a JSON value and its original number tokens")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_u64<E: de::Error>(self, marker: u64) -> Result<Self::Value, E> {
        if marker != 0 {
            return Err(E::custom("unexpected number in the validation projection"));
        }
        let raw = self
            .0
            .next()
            .ok_or_else(|| E::custom("missing number lexeme"))?;
        Ok(Value::Number(Number(raw)))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = input.next_element_seed(ReadValue(&mut *self.0))? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        let mut values = BTreeMap::new();
        while let Some(key) = input.next_key::<String>()? {
            let value = input.next_value_seed(ReadValue(&mut *self.0))?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

fn digits(bytes: &[u8], mut offset: usize) -> usize {
    while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
        offset += 1;
    }
    offset
}

fn number_end(bytes: &[u8], mut offset: usize) -> Option<usize> {
    if bytes.get(offset) == Some(&b'-') {
        offset += 1;
    }
    offset = match bytes.get(offset) {
        Some(b'0') => offset + 1,
        Some(b'1'..=b'9') => digits(bytes, offset + 1),
        _ => return None,
    };
    if bytes.get(offset) == Some(&b'.') {
        let start = offset + 1;
        offset = digits(bytes, start);
        if offset == start {
            return None;
        }
    }
    if matches!(bytes.get(offset), Some(b'e' | b'E')) {
        let mut start = offset + 1;
        if matches!(bytes.get(start), Some(b'+' | b'-')) {
            start += 1;
        }
        offset = digits(bytes, start);
        if offset == start {
            return None;
        }
    }
    if bytes
        .get(offset)
        .is_some_and(|next| !matches!(next, b' ' | b'\t' | b'\n' | b'\r' | b',' | b'}' | b']'))
    {
        return None;
    }
    Some(offset)
}

pub(super) fn parse(source: &str) -> Result<Value<'_>> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut projection = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    let mut unmasked_start = 0;
    while offset < bytes.len() {
        match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    let next = bytes[offset];
                    offset += 1;
                    if next == b'\\' {
                        offset += 1;
                    } else if next == b'"' {
                        break;
                    }
                }
            }
            b'-' | b'0'..=b'9' => {
                let end = number_end(bytes, offset).context("JSON 数字词法无效")?;
                projection.extend_from_slice(&bytes[unmasked_start..offset]);
                projection.push(b'0');
                tokens.push(&source[offset..end]);
                offset = end;
                unmasked_start = end;
            }
            _ => offset += 1,
        }
    }
    // Parsing still checks complete JSON syntax, depth, strings and UTF-8.
    // The numeric projection avoids binary floating-point and the visitor
    // attaches the corresponding original token in traversal order.
    projection.extend_from_slice(&bytes[unmasked_start..]);
    let mut tokens = tokens.iter();
    let mut parser = serde_json::Deserializer::from_slice(&projection);
    let value = ReadValue(&mut tokens).deserialize(&mut parser)?;
    parser.end()?;
    ensure!(tokens.next().is_none(), "JSON 数字词法与结构不一致");
    Ok(value)
}
