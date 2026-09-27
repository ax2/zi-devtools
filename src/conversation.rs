//! Ephemeral, bounded user/assistant context. No files, tools, or automatic execution.
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
