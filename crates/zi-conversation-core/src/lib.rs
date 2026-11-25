//! Portable ZiDevTools conversation validation and inert Markdown export.
use anyhow::{Result, anyhow, ensure};
use serde::Deserialize;

pub const MAX_TURNS: usize = 16;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportFile {
    schema: String,
    version: u32,
    messages: Vec<ExportMessage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportMessage {
    role: String,
    content: String,
}

pub fn validate_question(turns: usize, bytes: usize, input: &str) -> Result<()> {
    ensure!(!input.trim().is_empty(), "请输入消息");
    ensure!(turns < MAX_TURNS, "会话已达 16 轮，请清空会话后继续");
    ensure!(
        bytes.saturating_add(input.len()) <= MAX_BYTES,
        "会话文本超过 256 KiB，请缩短输入或清空会话"
    );
    Ok(())
}

pub fn validate_answer(bytes_with_question: usize, output: &str) -> Result<()> {
    ensure!(!output.trim().is_empty(), "响应为空，未加入上下文");
    ensure!(
        bytes_with_question.saturating_add(output.len()) <= MAX_BYTES,
        "响应超出会话容量，结果保留但未加入上下文；请清空会话后继续"
    );
    Ok(())
}

pub fn parse_transcript(text: &str) -> Result<Vec<(String, String)>> {
    ensure!(text.len() <= MAX_IMPORT_BYTES, "会话文件超过 2 MiB");
    let file: ExportFile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| anyhow!("会话 JSON 格式无效或包含未知字段"))?;
    ensure!(
        file.schema == "zi-devtools-conversation" && file.version == 1,
        "不支持的会话格式或版本"
    );
    ensure!(
        !file.messages.is_empty()
            && file.messages.len().is_multiple_of(2)
            && file.messages.len() <= MAX_TURNS * 2,
        "会话必须包含 1 至 16 轮完整问答"
    );
    let mut turns = Vec::with_capacity(file.messages.len() / 2);
    let mut bytes = 0;
    let mut messages = file.messages.into_iter();
    while let Some(question) = messages.next() {
        let answer = messages.next().ok_or_else(|| anyhow!("会话缺少回答"))?;
        ensure!(
            question.role == "user" && answer.role == "assistant",
            "仅支持按顺序排列的 user/assistant 问答，不支持工具或系统消息"
        );
        validate_question(turns.len(), bytes, &question.content)?;
        bytes += question.content.len();
        validate_answer(bytes, &answer.content)?;
        bytes += answer.content.len();
        turns.push((question.content, answer.content));
    }
    Ok(turns)
}

pub fn render_markdown(turns: &[(String, String)]) -> Result<String> {
    ensure!(!turns.is_empty(), "没有可导出的成功会话");
    let mut text = String::from("# Zi DevTools 会话\n\n仅包含成功完成的问答。\n");
    for (index, (question, answer)) in turns.iter().enumerate() {
        for (role, content) in [("你", question), ("模型", answer)] {
            // A longer fence keeps even original HTML and Markdown inside literal text.
            let longest = longest_backtick_run(content);
            let fence = "`".repeat(3.max(longest + 1));
            text.push_str(&format!("\n## 第 {} 轮 · {role}\n\n", index + 1));
            text.push_str(&fence);
            text.push_str("text\n");
            text.push_str(content);
            text.push('\n');
            text.push_str(&fence);
            text.push('\n');
        }
    }
    Ok(text)
}

fn longest_backtick_run(content: &str) -> usize {
    let Some(start) = content.find('`') else {
        return 0;
    };
    let mut longest = 0;
    let mut current = 0;
    for &byte in &content.as_bytes()[start..] {
        if byte == b'`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}
