//! Bounded in-memory context with explicit text export; no automatic persistence.
use crate::plugins::{Adapter, PluginTool};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
pub use zi_conversation_core::{MAX_BYTES, MAX_TURNS};
use zi_conversation_core::{
    MAX_IMPORT_BYTES, parse_transcript, render_markdown, validate_answer, validate_question,
};
#[derive(Default)]
pub struct Conversation {
    binding: String,
    turns: Vec<(String, String)>,
}
pub fn supported(tool: &PluginTool) -> bool {
    match &tool.adapter {
        Adapter::Http {
            method,
            body,
            response_pointer,
            ..
        } => {
            method == "POST"
                && body.get("model") == Some(&json!("$model"))
                && body.get("messages") == Some(&json!([{"role":"user","content":"$input"}]))
                && body.get("stream") == Some(&json!(false))
                && matches!(
                    response_pointer.as_deref(),
                    Some("/choices/0/message/content" | "/message/content")
                )
                && body.get("tools").is_none()
                && body.get("functions").is_none()
        }
        _ => false,
    }
}
impl Conversation {
    pub fn import_file(path: &str, binding: String) -> Result<Self> {
        use std::io::Read;
        let path = path.trim();
        ensure!(std::fs::metadata(path)?.is_file(), "请选择普通 JSON 文件");
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_IMPORT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_IMPORT_BYTES, "会话文件超过 2 MiB");
        Self::import(std::str::from_utf8(&bytes)?, binding)
    }
    pub(crate) fn import(text: &str, binding: String) -> Result<Self> {
        Ok(Self {
            binding,
            turns: parse_transcript(text)?,
        })
    }

    pub fn bind(&mut self, binding: String) -> bool {
        if self.binding == binding {
            return false;
        }
        let had_context = !self.turns.is_empty();
        self.turns.clear();
        self.binding = binding;
        had_context
    }
    pub fn clear(&mut self) {
        self.turns.clear();
    }
    pub fn binding(&self) -> &str {
        &self.binding
    }
    pub fn turns(&self) -> &[(String, String)] {
        &self.turns
    }
    pub fn bytes(&self) -> usize {
        self.turns.iter().map(|(q, a)| q.len() + a.len()).sum()
    }
    pub fn export(&self, json_format: bool) -> Result<String> {
        ensure!(!self.turns.is_empty(), "没有可导出的成功会话");
        if json_format {
            let messages: Vec<Value> = self
                .turns
                .iter()
                .flat_map(|(q, a)| {
                    [
                        json!({"role":"user","content":q}),
                        json!({"role":"assistant","content":a}),
                    ]
                })
                .collect();
            return Ok(serde_json::to_string_pretty(&json!({
                "schema":"zi-devtools-conversation", "version":1, "messages":messages
            }))?);
        }
        render_markdown(&self.turns)
    }

    pub fn prepare(&self, input: &str) -> Result<Vec<Value>> {
        validate_question(self.turns.len(), self.bytes(), input)?;
        let mut messages = Vec::with_capacity(self.turns.len() * 2 + 1);
        for (user, assistant) in &self.turns {
            messages.push(json!({"role":"user","content":user}));
            messages.push(json!({"role":"assistant","content":assistant}));
        }
        messages.push(json!({"role":"user","content":input}));
        Ok(messages)
    }
    pub fn complete(&mut self, input: String, output: String) -> Result<()> {
        let bytes = self.bytes();
        validate_question(self.turns.len(), bytes, &input)?;
        validate_answer(bytes + input.len(), &output)?;
        self.turns.push((input, output));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_is_bounded_strict_and_preserves_exported_context() {
        let mut original = Conversation::default();
        original
            .complete("原始🙂$model".into(), "答案\n```".into())
            .unwrap();
        let exported = original.export(true).unwrap();
        let restored = Conversation::import(&exported, "current-binding".into()).unwrap();
        assert_eq!(restored.turns(), original.turns());
        assert_eq!(restored.binding(), "current-binding");
        assert_eq!(
            restored.prepare("继续").unwrap(),
            original.prepare("继续").unwrap()
        );
        let base: Value = serde_json::from_str(&exported).unwrap();
        let mut bad = Vec::new();
        let mut v = base.clone();
        v["messages"] = json!([]);
        bad.push(v);
        let mut v = base.clone();
        v["messages"] = Value::Array(
            base["messages"]
                .as_array()
                .unwrap()
                .iter()
                .cycle()
                .take(34)
                .cloned()
                .collect(),
        );
        bad.push(v);
        let mut v = base.clone();
        v["version"] = json!(2);
        bad.push(v);
        let mut v = base.clone();
        v["endpoint"] = json!("ignored?");
        bad.push(v);
        let mut v = base.clone();
        v["messages"][0]["role"] = json!("system");
        bad.push(v);
        let mut v = base.clone();
        v["messages"][0]["tool_calls"] = json!([]);
        bad.push(v);
        let mut v = base.clone();
        v["messages"].as_array_mut().unwrap().pop();
        bad.push(v);
        let mut v = base.clone();
        v["messages"][1]["content"] = json!(" ");
        bad.push(v);
        let mut v = base.clone();
        v["messages"][1]["content"] = json!("x".repeat(MAX_BYTES));
        bad.push(v);
        for value in bad {
            assert!(Conversation::import(&value.to_string(), String::new()).is_err());
        }
        assert!(Conversation::import(&" ".repeat(MAX_IMPORT_BYTES + 1), String::new()).is_err());
        assert_eq!(original.turns().len(), 1);
        let path = std::env::temp_dir().join(format!("zi-chat-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, format!("\u{feff}{exported}")).unwrap();
        let loaded = Conversation::import_file(path.to_str().unwrap(), "current".into()).unwrap();
        assert_eq!(loaded.turns(), original.turns());
        std::fs::write(&path, vec![b' '; MAX_IMPORT_BYTES + 1]).unwrap();
        assert!(Conversation::import_file(path.to_str().unwrap(), String::new()).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn export_preserves_text_without_connection_binding_or_incomplete_turns() {
        let mut chat = Conversation::default();
        chat.bind("private-connection-binding".into());
        assert!(chat.export(false).is_err());
        let input = "中文\n````\n<script>example</script>";
        chat.complete(input.into(), "$input🙂".into()).unwrap();
        let _pending = chat.prepare("未完成消息").unwrap();
        let data: Value = serde_json::from_str(&chat.export(true).unwrap()).unwrap();
        assert_eq!(data["messages"].as_array().unwrap().len(), 2);
        assert_eq!(data["messages"][0]["content"], input);
        let markdown = chat.export(false).unwrap();
        assert!(markdown.contains("`````text\n"));
        assert!(markdown.contains(input));
        for exported in [markdown, chat.export(true).unwrap()] {
            assert!(!exported.contains("private-connection-binding"));
            assert!(!exported.contains("未完成消息"));
        }
    }
    #[test]
    fn context_is_ordered_bounded_and_destination_changes_clear_it() {
        let mut chat = Conversation::default();
        chat.bind("first".into());
        chat.complete("$model".into(), "$input".into()).unwrap();
        let messages = chat.prepare("继续").unwrap();
        assert_eq!(messages, json!([{"role":"user","content":"$model"},{"role":"assistant","content":"$input"},{"role":"user","content":"继续"}]).as_array().unwrap().clone());
        assert!(chat.complete("q".into(), "x".repeat(MAX_BYTES)).is_err());
        assert_eq!(chat.turns().len(), 1);
        assert!(!chat.bind("first".into()));
        assert!(chat.bind("second".into()));
        assert!(chat.turns().is_empty());
        for _ in 0..MAX_TURNS {
            chat.complete("q".into(), "a".into()).unwrap();
        }
        assert!(chat.prepare("next").is_err());
        chat.clear();
        assert!(chat.prepare("next").is_ok());
    }
}
