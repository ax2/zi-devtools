use crate::{Adapter, MANIFEST_LIMIT, Manifest, validate_manifest};
use serde::Serialize;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::collections::BTreeSet;
use std::fmt;

const PAGE_SIZE: usize = 8;
const MAX_OUTSIDE_STRING_BYTES: usize = 32 * 1024;
const MAX_JSON_WORK: usize = 4096;
const MAX_JSON_WORK_BYTES: usize = 288 * 1024;
const PROPERTY_WORK_BYTES: usize = 128;
const MAX_DECLARED_URL_BYTES: usize = 2048;

#[derive(Debug, PartialEq, Eq)]
pub enum InspectError {
    TooLarge,
    TooComplex,
    InvalidJson,
    InvalidManifest,
    InvalidPage,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub total_tools: usize,
    pub builtin_tools: usize,
    pub http_tools: usize,
    pub tool_start: usize,
    pub next_tool_start: Option<usize>,
    pub tools: Vec<ToolPreview>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPreview {
    pub ordinal: usize,
    pub id: String,
    pub name: String,
    pub category: String,
    pub version: Option<String>,
    pub alias_count: usize,
    pub history_count: usize,
    pub adapter: AdapterPreview,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AdapterPreview {
    Builtin { tool: String, action: usize },
    Http { method: String },
}

fn display(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

pub fn preview(bytes: &[u8], tool_start: usize) -> Result<Preview, InspectError> {
    if bytes.len() > MANIFEST_LIMIT {
        return Err(InspectError::TooLarge);
    }
    let bytes = json_value_bytes(bytes);
    check_work(bytes)?;
    serde_json::from_slice::<Distinct>(bytes).map_err(|_| InspectError::InvalidJson)?;
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|_| InspectError::InvalidManifest)?;
    if manifest.tools.iter().any(|tool| matches!(&tool.adapter, Adapter::Http { url, .. } if url.len() > MAX_DECLARED_URL_BYTES)) {
        return Err(InspectError::TooComplex);
    }
    validate_manifest(&manifest).map_err(|_| InspectError::InvalidManifest)?;
    let total = manifest.tools.len();
    if !(1..=total).contains(&tool_start) {
        return Err(InspectError::InvalidPage);
    }
    let http = manifest
        .tools
        .iter()
        .filter(|tool| matches!(tool.adapter, Adapter::Http { .. }))
        .count();
    let tools = manifest
        .tools
        .into_iter()
        .enumerate()
        .skip(tool_start - 1)
        .take(PAGE_SIZE)
        .map(|(index, tool)| {
            let adapter = match tool.adapter {
                Adapter::Builtin { tool, action, .. } => AdapterPreview::Builtin { tool, action },
                Adapter::Http { method, .. } => AdapterPreview::Http { method },
            };
            ToolPreview {
                ordinal: index + 1,
                id: tool.id,
                name: display(&tool.name),
                category: tool.category,
                version: tool.version,
                alias_count: tool.aliases.len(),
                history_count: tool.version_history.len(),
                adapter,
            }
        })
        .collect();
    Ok(Preview {
        schema_version: manifest.schema_version,
        id: manifest.id,
        name: display(&manifest.name),
        version: display(&manifest.version),
        total_tools: total,
        builtin_tools: total - http,
        http_tools: http,
        tool_start,
        next_tool_start: (tool_start + PAGE_SIZE <= total).then_some(tool_start + PAGE_SIZE),
        tools,
    })
}

fn json_value_bytes(bytes: &[u8]) -> &[u8] {
    let content = |byte: &u8| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
    let Some(start) = bytes.iter().position(content) else {
        return &[];
    };
    let end = bytes
        .iter()
        .rposition(content)
        .expect("nonempty JSON extent");
    &bytes[start..=end]
}

fn check_work(mut bytes: &[u8]) -> Result<(), InspectError> {
    let mut outside = 0;
    let mut work = 0;
    let mut work_bytes = bytes.len();
    while let Some((&byte, rest)) = bytes.split_first() {
        outside += 1;
        if outside > MAX_OUTSIDE_STRING_BYTES {
            return Err(InspectError::TooComplex);
        }
        bytes = rest;
        if byte == b'"' {
            loop {
                let Some(index) = memchr::memchr2(b'"', b'\\', bytes) else {
                    return Ok(());
                };
                let escaped = bytes[index] == b'\\';
                if escaped {
                    work += 1;
                    if work > MAX_JSON_WORK {
                        return Err(InspectError::TooComplex);
                    }
                }
                bytes = bytes
                    .get(index + 1 + usize::from(escaped)..)
                    .unwrap_or_default();
                if !escaped {
                    break;
                }
            }
        } else if matches!(byte, b'{' | b'}' | b'[' | b']' | b':' | b',') {
            // Object keys are indexed by the duplicate audit and decoded again by the typed parser.
            if byte == b':' {
                work += 3;
                work_bytes += PROPERTY_WORK_BYTES;
            } else {
                work += 1;
            }
            if work > MAX_JSON_WORK || work_bytes > MAX_JSON_WORK_BYTES {
                return Err(InspectError::TooComplex);
            }
        }
    }
    Ok(())
}

// A declarative body is free-form JSON. Validate object keys before typed deserialization,
// which would otherwise collapse repeated keys inside a serde_json::Value.
struct Distinct;

impl<'de> Deserialize<'de> for Distinct {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        de.deserialize_any(DistinctVisitor)
    }
}

struct DistinctVisitor;

impl<'de> Visitor<'de> for DistinctVisitor {
    type Value = Distinct;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object keys")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Distinct, E> {
        Ok(Distinct)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Distinct, E> {
        Ok(Distinct)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Distinct, E> {
        Ok(Distinct)
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Distinct, E> {
        Ok(Distinct)
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Distinct, E> {
        Ok(Distinct)
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Distinct, E> {
        Ok(Distinct)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Distinct, A::Error> {
        while sequence.next_element::<Distinct>()?.is_some() {}
        Ok(Distinct)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Distinct, A::Error> {
        let mut seen = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key) {
                return Err(de::Error::custom("duplicate key"));
            }
            map.next_value::<Distinct>()?;
        }
        Ok(Distinct)
    }
}
