use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

const MAX_FILE: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 8 * MAX_FILE;
type Failure = &'static str;

enum Operation {
    Path(String),
    Diff { unordered: bool },
}

impl Operation {
    fn parse(id: &str, input: Value) -> Result<Self, Failure> {
        let input = input.as_object().ok_or("INVALID_REQUEST")?;
        match id {
            "devtools.compare.json-file.path" if input.len() == 1 => {
                let query = input
                    .get("query")
                    .and_then(Value::as_str)
                    .filter(|query| !query.is_empty() && query.len() <= 4096)
                    .ok_or("INVALID_REQUEST")?;
                Ok(Self::Path(query.to_owned()))
            }
            "devtools.compare.json-file.diff.ordered"
            | "devtools.compare.json-file.diff.unordered"
                if input.is_empty() =>
            {
                Ok(Self::Diff {
                    unordered: id.ends_with(".unordered"),
                })
            }
            _ => Err("INVALID_REQUEST"),
        }
    }

    fn count(&self) -> usize {
        match self {
            Self::Path(_) => 1,
            Self::Diff { .. } => 2,
        }
    }

    fn run(&self, files: &[Vec<u8>]) -> Result<(Value, String), Failure> {
        if files.len() != self.count() {
            return Err("INVALID_REQUEST");
        }
        if files.iter().any(|file| file.len() > MAX_FILE) {
            return Err("INPUT_TOO_LARGE");
        }
        let text = |index: usize| std::str::from_utf8(&files[index]).map_err(|_| "INVALID_UTF8");
        let result = match self {
            Self::Path(query) => zi_json_core::json_path(text(0)?, query),
            Self::Diff { unordered } => zi_json_core::json_diff_exact_with_result_budget(
                text(0)?,
                text(1)?,
                *unordered,
                MAX_OUTPUT,
            ),
        };
        let output = result.map_err(|error| {
            if error.is::<zi_json_core::ReportTooLarge>() {
                "RESULT_TOO_LARGE"
            } else {
                "INVALID_JSON_OR_QUERY"
            }
        })?;
        let source_byte_counts: Vec<_> = files.iter().map(Vec::len).collect();
        let report = json!({"ok":true,"schemaVersion":1,"material":"json-file-result",
            "sourceByteCounts":source_byte_counts,"outputByteCount":output.len()});
        Ok((report, output))
    }
}

fn request<T: for<'de> Deserialize<'de>>(limit: usize) -> Result<T, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "INVALID_REQUEST")?;
    if bytes.len() > limit {
        return Err("INVALID_REQUEST");
    }
    serde_json::from_slice(&bytes).map_err(|_| "INVALID_REQUEST")
}

fn main() {
    let reply =
        transport::execute().unwrap_or_else(|code| json!({"ok":false,"error":{"code":code}}));
    serde_json::to_writer(std::io::stdout().lock(), &reply).expect("write result");
}

#[cfg(target_arch = "wasm32")]
mod transport {
    use super::*;
    const PLUGIN: &str = "com.zicode.devtools.compare-json-file";
    const CHUNK: usize = 64 * 1024;

    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn output_create(mime_index: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn output_write(index: i32, source: *const u8, length: i32) -> i32;
        #[link_name = "finish"]
        fn output_finish(index: i32) -> i32;
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Envelope {
        plugin_id: String,
        scene_id: String,
        capability_id: String,
        command_id: Value,
        input: Value,
        resource_input_profile: String,
        resource_output_profile: String,
        resources: Vec<Descriptor>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Descriptor {
        index: usize,
        resource_id: String,
        content_version: String,
        mime_type: String,
        byte_length: usize,
    }

    fn read_file(index: usize, file: &Descriptor) -> Result<Vec<u8>, Failure> {
        if file.index != index
            || file.resource_id.is_empty()
            || file.content_version.len() != 64
            || !file
                .content_version
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !matches!(
                file.mime_type.as_str(),
                "text/plain" | "application/octet-stream" | "application/json"
            )
        {
            return Err("INVALID_REQUEST");
        }
        if file.byte_length > MAX_FILE {
            return Err("INPUT_TOO_LARGE");
        }
        zi_resource_input_wasi::read_prefix(index as i32, file.byte_length, file.byte_length)
    }

    pub(super) fn execute() -> Result<Value, Failure> {
        let envelope: Envelope = request(32 * 1024)?;
        if envelope.plugin_id != PLUGIN
            || envelope.scene_id.is_empty()
            || !envelope.command_id.is_null()
            || envelope.resource_input_profile != "resource.wasi.input.draft2"
            || envelope.resource_output_profile != "resource.wasi.output.draft1"
        {
            return Err("INVALID_REQUEST");
        }
        let operation = Operation::parse(&envelope.capability_id, envelope.input)?;
        if envelope.resources.len() != operation.count() {
            return Err("INVALID_REQUEST");
        }
        let files = envelope
            .resources
            .iter()
            .enumerate()
            .map(|(index, file)| read_file(index, file))
            .collect::<Result<Vec<_>, _>>()?;
        let (report, output) = operation.run(&files)?;
        // Reserved application/json index for the JSON-file compare execution profile.
        let index = unsafe { output_create(28, output.len() as i64) };
        if index != 0 {
            return Err("OUTPUT_FAILED");
        }
        for chunk in output.as_bytes().chunks(CHUNK) {
            if unsafe { output_write(index, chunk.as_ptr(), chunk.len() as i32) }
                != chunk.len() as i32
            {
                return Err("OUTPUT_FAILED");
            }
        }
        if unsafe { output_finish(index) } != 0 {
            return Err("OUTPUT_FAILED");
        }
        Ok(report)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Fixture {
        capability_id: String,
        input: Value,
        files_base64: Vec<String>,
    }

    pub(super) fn execute() -> Result<Value, Failure> {
        let fixture: Fixture = request(((MAX_FILE + 1).div_ceil(3) * 4) * 2 + 32 * 1024)?;
        let operation = Operation::parse(&fixture.capability_id, fixture.input)?;
        if fixture.files_base64.len() != operation.count() {
            return Err("INVALID_REQUEST");
        }
        let files = fixture
            .files_base64
            .iter()
            .map(|value| STANDARD.decode(value).map_err(|_| "INVALID_REQUEST"))
            .collect::<Result<Vec<_>, _>>()?;
        let (report, output) = operation.run(&files)?;
        Ok(json!({"report":report,"outputBase64":STANDARD.encode(output)}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_files_with_exact_values_and_retains_the_resource_shape() {
        for unordered in [false, true] {
            let files = vec![
                b"[0.123456789012345678901]".to_vec(),
                b"[0.123456789012345678902]".to_vec(),
            ];
            let (report, output) = Operation::Diff { unordered }.run(&files).unwrap();
            assert_eq!(
                report["sourceByteCounts"],
                json!([files[0].len(), files[1].len()])
            );
            assert_eq!(report["outputByteCount"], output.len());
            assert!(output.contains(r#""equal": false"#));
            assert!(
                output.contains("0.123456789012345678901")
                    && output.contains("0.123456789012345678902")
            );
        }
    }

    #[test]
    fn checks_query_and_refuses_malformed_or_out_of_scope_resources() {
        let path = "devtools.compare.json-file.path";
        assert!(Operation::parse(path, json!({"other":"$"})).is_err());
        let query = Operation::parse(path, json!({"query":"$.x"})).unwrap();
        let (_, output) = query.run(&[br#"{"x":[1,2]}"#.to_vec()]).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap(),
            json!([[1, 2]])
        );
        let diff = Operation::Diff { unordered: false };
        for (files, expected) in [
            (vec![b"null".to_vec()], "INVALID_REQUEST"),
            (vec![vec![0xff], b"null".to_vec()], "INVALID_UTF8"),
            (
                vec![br#""\uD800""#.to_vec(), b"1e99999".to_vec()],
                "INVALID_JSON_OR_QUERY",
            ),
            (
                vec![vec![b' '; MAX_FILE + 1], b"null".to_vec()],
                "INPUT_TOO_LARGE",
            ),
        ] {
            assert_eq!(diff.run(&files).unwrap_err(), expected);
        }
    }
}
