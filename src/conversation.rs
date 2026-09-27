//! Bounded in-memory context with explicit text export; no automatic persistence.
use crate::plugins::{Adapter, PluginTool};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub const MAX_TURNS: usize = 16;
pub const MAX_BYTES: usize = 256 * 1024;
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
        let mut text = String::from("# Zi DevTools 会话\n\n仅包含成功完成的问答。\n");
        for (index, (question, answer)) in self.turns.iter().enumerate() {
            for (role, content) in [("你", question), ("模型", answer)] {
                // Fence original text, including HTML and Markdown, without altering it.
                let longest = content.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(3.max(longest + 1));
                text.push_str(&format!(
                    "\n## 第 {} 轮 · {role}\n\n{fence}text\n{content}\n{fence}\n",
                    index + 1
                ));
            }
        }
        Ok(text)
    }
    pub fn prepare(&self, input: &str) -> Result<Vec<Value>> {
        ensure!(!input.trim().is_empty(), "请输入消息");
        ensure!(
            self.turns.len() < MAX_TURNS,
            "会话已达 16 轮，请清空会话后继续"
        );
        ensure!(
            self.bytes().saturating_add(input.len()) <= MAX_BYTES,
            "会话文本超过 256 KiB，请缩短输入或清空会话"
        );
        let mut messages = Vec::with_capacity(self.turns.len() * 2 + 1);
        for (user, assistant) in &self.turns {
            messages.push(json!({"role":"user","content":user}));
            messages.push(json!({"role":"assistant","content":assistant}));
        }
        messages.push(json!({"role":"user","content":input}));
        Ok(messages)
    }
    pub fn complete(&mut self, input: String, output: String) -> Result<()> {
        self.prepare(&input)?;
        ensure!(!output.trim().is_empty(), "响应为空，未加入上下文");
        ensure!(
            self.bytes()
                .saturating_add(input.len())
                .saturating_add(output.len())
                <= MAX_BYTES,
            "响应超出会话容量，结果保留但未加入上下文；请清空会话后继续"
        );
        self.turns.push((input, output));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
