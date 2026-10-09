//! Fixed compatibility fixture, not a plugin adapter or Host request protocol.
use serde_json::{Value, json};
use zi_json_core::{json_diff, json_path};
fn outcome(result: anyhow::Result<String>) -> Value {
    match result {
        Ok(text) => json!({"text":text}),
        Err(error) => json!({"error":error.to_string()}),
    }
}
fn main() {
    let cases = [
        (
            "path-unicode",
            json_path(r#"{"项":[{"a.b":1},{"a.b":2}]}"#, r#"$.项[*]["a.b"]"#),
        ),
        ("path-u32-plus", json_path("[1]", "$[4294967296]")),
        ("path-u64-max", json_path("[1]", "$[18446744073709551615]")),
        (
            "path-u64-overflow",
            json_path("[1]", "$[18446744073709551616]"),
        ),
        ("path-duplicate-key", json_path(r#"{"x":1,"x":2}"#, "$.x")),
        ("path-wide-integer", json_path("[9007199254740993]", "$[0]")),
        ("diff-ordered", json_diff("[1,2]", "[2,1]", false)),
        ("diff-unordered", json_diff("[1,2]", "[2,1]", true)),
        ("diff-duplicates", json_diff("[1,1,2]", "[1,2,2]", true)),
        (
            "diff-pointer",
            json_diff(r#"{"a/b~":1}"#, r#"{"a/b~":null}"#, false),
        ),
    ]
    .into_iter()
    .map(|(id, result)| json!({"id":id,"outcome":outcome(result)}))
    .collect::<Vec<_>>();
    println!("{}", json!({"pointerWidth":usize::BITS,"cases":cases}));
}
