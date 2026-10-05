//! Built-in discovery comes from the versioned catalog; routes stay executable Rust.
use super::ToolKind;
use serde::Deserialize;
use std::sync::{Arc, OnceLock};
use unicode_normalization::UnicodeNormalization;

#[derive(Deserialize)]
struct CatalogDocument {
    tools: Vec<Definition>,
}

#[derive(Deserialize)]
struct Definition {
    id: String,
    tool_version: String,
    status: String,
    discovery: Option<Discovery>,
}

#[derive(Deserialize)]
struct Discovery {
    label: String,
    summary: String,
    category: String,
    keywords: String,
    aliases: Vec<String>,
}

#[derive(Clone)]
pub(super) struct ToolEntry {
    pub id: String,
    pub version: Option<String>,
    pub title: String,
    pub description: String,
    pub category: String,
    pub page: Page,
    pub kind: Option<ToolKind>,
    pub in_progress: bool,
    search: Arc<SearchText>,
}

struct SearchText {
    id: String,
    title: String,
    aliases: Vec<String>,
    text: String,
    words: Vec<String>,
    name_words: Vec<String>,
}

pub(super) fn normalized(text: &str) -> String {
    text.nfkc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl ToolEntry {
    fn new(
        id: String,
        discovery: Discovery,
        page: Page,
        kind: Option<ToolKind>,
        in_progress: bool,
    ) -> Self {
        let title = normalized(&discovery.label);
        let aliases: Vec<_> = discovery.aliases.iter().map(|s| normalized(s)).collect();
        let text = normalized(&format!(
            "{} {} {} {} {} {}",
            id,
            discovery.label,
            discovery.summary,
            discovery.category,
            discovery.keywords,
            aliases.join(" ")
        ));
        let words = text
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .filter(|word| word.len() >= 4)
            .map(String::from)
            .collect();
        let name_words = format!("{id} {title} {}", aliases.join(" "))
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .filter(|word| word.len() >= 4)
            .map(normalized)
            .collect();
        Self {
            version: None,
            search: Arc::new(SearchText {
                id: normalized(&id),
                title,
                aliases,
                text,
                words,
                name_words,
            }),
            id,
            title: discovery.label,
            description: discovery.summary,
            category: discovery.category,
            page,
            kind,
            in_progress,
        }
    }

    pub fn plugin(id: String, tool: &crate::plugins::PluginTool) -> Self {
        let mut entry = Self::new(
            id,
            Discovery {
                label: tool.name.clone(),
                summary: tool.description.clone(),
                category: tool.category.clone(),
                keywords: tool.keywords.join(" "),
                aliases: Vec::new(),
            },
            Page::Plugins,
            None,
            false,
        );
        entry.version.clone_from(&tool.version);
        entry
    }

    pub fn score(&self, query: &str) -> Option<u32> {
        self.score_normalized(&normalized(query))
    }

    pub fn score_normalized(&self, query: &str) -> Option<u32> {
        if query.is_empty() {
            return Some(0);
        }
        let search = &self.search;
        if search.id == query || search.title == query {
            return Some(1000);
        }
        if search.aliases.iter().any(|a| a == query) {
            return Some(900);
        }
        if search.title.starts_with(query) {
            return Some(700);
        }
        if query
            .split_whitespace()
            .all(|word| search.text.contains(word))
        {
            return Some(if search.title.contains(query) {
                500
            } else {
                300
            });
        }
        // Only a single short ASCII word may use one-edit correction. Never loosen
        // multiple-word filtering or infer an action from arbitrary pasted content.
        if (4..=32).contains(&query.len())
            && query
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && search
                .words
                .iter()
                .any(|word| one_edit(query.as_bytes(), word.as_bytes()))
        {
            return Some(
                if one_edit(query.as_bytes(), search.id.as_bytes())
                    || one_edit(query.as_bytes(), search.title.as_bytes())
                {
                    230
                } else if search
                    .name_words
                    .iter()
                    .any(|word| one_edit(query.as_bytes(), word.as_bytes()))
                {
                    180
                } else {
                    100
                },
            );
        }
        None
    }

    pub fn match_hint(&self, query: &str) -> &'static str {
        match self.score(query) {
            Some(1000) => "名称匹配",
            Some(900) => "用途 / 别名匹配",
            Some(700 | 500) => "名称包含",
            Some(100 | 180 | 230) => "近似拼写",
            Some(300) => "说明 / 关键词匹配",
            _ => "",
        }
    }

    pub fn badge(&self) -> &'static str {
        if self.in_progress {
            "开发中 · 部分能力可用"
        } else if self.id.starts_with("plugin:") {
            "插件"
        } else {
            "内置"
        }
    }
}

fn one_edit(a: &[u8], b: &[u8]) -> bool {
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let mismatch = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    if mismatch == a.len().min(b.len()) {
        return true;
    }
    if a.len() == b.len() {
        a[mismatch + 1..] == b[mismatch + 1..]
            || (mismatch + 1 < a.len()
                && a[mismatch] == b[mismatch + 1]
                && a[mismatch + 1] == b[mismatch]
                && a[mismatch + 2..] == b[mismatch + 2..])
    } else if a.len() > b.len() {
        a[mismatch + 1..] == b[mismatch..]
    } else {
        a[mismatch..] == b[mismatch + 1..]
    }
}

pub(super) fn catalog() -> &'static [ToolEntry] {
    static ENTRIES: OnceLock<Vec<ToolEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let document: CatalogDocument = serde_json::from_str(include_str!("../../docs/tools.json"))
            .expect("validated embedded tool catalog");
        document
            .tools
            .into_iter()
            .filter_map(|definition| {
                let (page, kind) = route(&definition.id)?;
                assert_ne!(
                    definition.status, "planned",
                    "planned tools cannot be routed"
                );
                let discovery = definition
                    .discovery
                    .expect("visible tool discovery metadata");
                let mut entry = ToolEntry::new(
                    definition.id,
                    discovery,
                    page,
                    kind,
                    definition.status == "in-progress",
                );
                entry.version = Some(definition.tool_version);
                Some(entry)
            })
            .collect()
    })
}

pub(super) fn ranked<'a>(
    entries: impl Iterator<Item = &'a ToolEntry>,
    query: &str,
    preferences: &crate::preferences::Preferences,
) -> Vec<ToolEntry> {
    let query = normalized(query);
    let mut scored: Vec<_> = entries
        .filter_map(|e| {
            e.score_normalized(&query).map(|score| {
                let rank = score * 100
                    + if preferences.favorites.contains(&e.id) {
                        30
                    } else {
                        0
                    }
                    + preferences
                        .recent
                        .iter()
                        .position(|id| id == &e.id)
                        .map(|i| 20_u32.saturating_sub(i as u32))
                        .unwrap_or(0)
                    + preferences.usage.get(&e.id).copied().unwrap_or(0).min(10);
                (rank, e)
            })
        })
        .collect();
    scored.sort_by(|(left_rank, left), (right_rank, right)| {
        right_rank
            .cmp(left_rank)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
    scored.into_iter().map(|(_, e)| e.clone()).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Home,
    Library,
    Intake,
    Tasks,
    Notes,
    Calendar,
    Calculator,
    Clock,
    Commands,
    Services,
    SmallTools,
    EncodingTools,
    Http,
    Diff,
    Network,
    Data,
    Files,
    Images,
    Markdown,
    FileEncoding,
    ChecksumManifest,
    DiskInspector,
    DuplicateFinder,
    DirectoryCompare,
    SqliteBrowser,
    AsciiCodes,
    Symbols,
    AsciiArt,
    KnowledgeSources,
    DocumentIngestion,
    KnowledgeIndex,
    VectorIndex,
    KnowledgeSearch,
    HybridSearch,
    KnowledgeAnswer,
    Embedding,
    KnowledgeCapture,
    KnowledgeMcp,
    KnowledgeEval,
    Settings,
    Plugins,
    Mcp,
    Agent,
    AgentRecords,
    Recorder,
    Integrations,
    Java,
    Django,
}

fn route(id: &str) -> Option<(Page, Option<ToolKind>)> {
    if let Some(kind) = ToolKind::ALL.into_iter().find(|kind| kind.id() == id) {
        return Some((
            if kind.is_encoding() {
                Page::EncodingTools
            } else {
                Page::SmallTools
            },
            Some(kind),
        ));
    }
    if let Some(tool) = crate::framework::Tool::from_id(id) {
        return Some((
            if tool.category() == "Java 与 JVM" {
                Page::Java
            } else {
                Page::Django
            },
            None,
        ));
    }
    Some((
        match id {
            "task-center" => Page::Tasks,
            "memos" => Page::Notes,
            "calendar-planner" => Page::Calendar,
            "advanced-calculator" => Page::Calculator,
            "clock-workbench" => Page::Clock,
            "screenshot-workbench" => Page::Images,
            "unified-shortcuts" => Page::Commands,
            "global-launcher" => Page::Settings,
            "file-intake" => Page::Intake,
            "data" => Page::Data,
            "data-sqlite-export" => Page::Data,
            "pipeline" => Page::Data,
            "workspace-sessions" => Page::Data,
            "data-transform" => Page::Data,
            "csv-merge" => Page::Data,
            "files" => Page::Files,
            "http" => Page::Http,
            "diff" => Page::Diff,
            "network" => Page::Network,
            "plugins" => Page::Plugins,
            "integrations" => Page::Integrations,
            "mcp-inspector" => Page::Mcp,
            "agent-runner" => Page::Agent,
            "agent-replay" => Page::AgentRecords,
            "image-tools" => Page::Images,
            "image-batch" => Page::Images,
            "image-metadata" => Page::Images,
            "image-crop-annotate" => Page::Images,
            "markdown" => Page::Markdown,
            "encoding-detect" => Page::FileEncoding,
            "checksum-manifest" => Page::ChecksumManifest,
            "disk-inspector" => Page::DiskInspector,
            "duplicate-files" => Page::DuplicateFinder,
            "file-compare" => Page::DirectoryCompare,
            "sqlite" => Page::SqliteBrowser,
            "ascii-codes" => Page::AsciiCodes,
            "symbol-library" => Page::Symbols,
            "ascii-art" => Page::AsciiArt,
            "rag-sources" => Page::KnowledgeSources,
            "rag-ingestion" => Page::DocumentIngestion,
            "rag-index" => Page::KnowledgeIndex,
            "rag-keyword-search" => Page::KnowledgeSearch,
            "rag-vectors" => Page::VectorIndex,
            "rag-search" => Page::HybridSearch,
            "rag-answer" => Page::KnowledgeAnswer,
            "embedding-playground" => Page::Embedding,
            "rag-eval" => Page::KnowledgeEval,
            "browser-clipper" => Page::KnowledgeCapture,
            "mcp-server" => Page::KnowledgeMcp,
            "mcp-export" => Page::KnowledgeMcp,
            "screen-recorder" => Page::Recorder,
            "screen-recorder-multimonitor" => Page::Recorder,
            "screen-recorder-audio-mix" => Page::Recorder,
            "screen-recorder-audio-gain" => Page::Recorder,
            "screen-recorder-audio-meter" => Page::Recorder,
            "services" => Page::Services,
            _ => return None,
        },
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::Preferences;

    #[test]
    fn every_visible_definition_has_an_executable_route_and_valid_discovery() {
        let document: CatalogDocument =
            serde_json::from_str(include_str!("../../docs/tools.json")).unwrap();
        let mut ids = std::collections::HashSet::new();
        for definition in document.tools {
            assert!(ids.insert(definition.id.clone()));
            assert!(crate::plugins::valid_tool_version(&definition.tool_version));
            if definition.status == "implemented" {
                assert!(route(&definition.id).is_some(), "{}", definition.id);
            }
            if route(&definition.id).is_some() {
                assert!(matches!(
                    definition.status.as_str(),
                    "implemented" | "in-progress"
                ));
                let d = definition.discovery.unwrap();
                assert!(!d.label.trim().is_empty() && !d.summary.trim().is_empty());
                assert!(
                    crate::plugins::CATEGORIES.contains(&d.category.as_str()),
                    "{}",
                    definition.id
                );
                assert!(d.aliases.iter().all(|s| !s.trim().is_empty()));
            } else {
                assert!(definition.discovery.is_none(), "unrouted {}", definition.id);
            }
        }
        for kind in ToolKind::ALL {
            assert!(
                catalog()
                    .iter()
                    .any(|e| e.id == kind.id() && e.kind == Some(kind))
            );
        }
        assert!(catalog().iter().all(|entry| entry.version.is_some()));
        for tool in crate::framework::Tool::ALL {
            assert!(catalog().iter().any(|e| e.id == tool.id()));
        }
        assert_eq!(
            catalog()
                .iter()
                .find(|e| e.id == "mcp-export")
                .unwrap()
                .category,
            "MCP 与 Agent"
        );
        assert_eq!(
            catalog()
                .iter()
                .find(|e| e.id == "embedding-playground")
                .unwrap()
                .category,
            "AI 与模型"
        );
        assert!(
            catalog()
                .iter()
                .find(|e| e.id == "mcp-inspector")
                .unwrap()
                .in_progress
        );
    }

    #[test]
    fn common_task_queries_find_expected_tools_in_top_three() {
        // Product-level examples include spacing, width/case, purpose, abbreviation,
        // typo, and mixed Chinese/English; they do not just compare alias strings.
        let queries = [
            ("把图片变小", "image-tools"),
            ("压缩 图片", "image-tools"),
            ("批量转 WEBP", "image-batch"),
            ("裁剪图片", "image-crop-annotate"),
            ("ＪＳＯＮ", "json"),
            ("json格式化", "json"),
            ("JSON FORMAT", "json"),
            ("jsno", "json"),
            ("提取json字段", "json-path"),
            ("比较两个json", "json-diff"),
            ("合并表格", "csv-merge"),
            ("CSV JOIN", "csv-merge"),
            ("清洗数据", "data-transform"),
            ("json转csv", "data"),
            ("删除重复行", "lines"),
            ("比较两段文字", "diff"),
            ("时间戳转换", "timestamp"),
            ("sjc", "timestamp"),
            ("base64 解码", "base64"),
            ("base6", "base64"),
            ("生成二维码", "qr"),
            ("检查隐藏字符", "unicode"),
            ("luping", "screen-recorder"),
            ("查找重复文件", "duplicate-files"),
            ("查看大文件", "disk-inspector"),
            ("比较两个文件夹", "file-compare"),
            ("打开数据库", "sqlite"),
            ("文件乱码", "encoding-detect"),
            ("调试MCP", "mcp-inspector"),
            ("向文档提问", "rag-answer"),
            ("保存网页", "browser-clipper"),
            ("django报错", "django-trace"),
            ("颜文字", "symbol-library"),
            ("生成字符画", "ascii-art"),
            ("回收站", "memos"),
            ("延后", "calendar-planner"),
            ("每月", "calendar-planner"),
        ];
        let preferences = Preferences::default();
        for (query, expected) in queries {
            let found = ranked(catalog().iter(), query, &preferences);
            assert!(
                found.iter().take(3).any(|e| e.id == expected),
                "{query}: {:?}",
                found.iter().take(3).map(|e| &e.id).collect::<Vec<_>>()
            );
        }
        assert!(ranked(catalog().iter(), "json definitely-unmatched", &preferences).is_empty());
        assert!(
            ranked(
                catalog().iter(),
                "这是一个没有对应能力的完整任务句子",
                &preferences
            )
            .is_empty()
        );
    }

    #[test]
    fn ranking_keeps_exact_matches_above_favorites_and_filters_plugins() {
        let plugin = crate::plugins::PluginTool {
            id: "helper".into(),
            version: Some("1.2.3".into()),
            name: "JSON Helper".into(),
            description: "处理 JSON 文本".into(),
            category: "数据与格式".into(),
            keywords: vec!["json".into()],
            sample: String::new(),
            model: String::new(),
            adapter: crate::plugins::Adapter::Builtin {
                tool: "json".into(),
                action: 0,
                pattern: String::new(),
            },
        };
        let entry = ToolEntry::plugin("plugin:test/helper".into(), &plugin);
        assert_eq!(entry.version.as_deref(), Some("1.2.3"));
        let mut preferences = Preferences::default();
        preferences.favorites.push(entry.id.clone());
        preferences.visit(&entry.id);
        let found = ranked(
            catalog().iter().chain(std::iter::once(&entry)),
            "json",
            &preferences,
        );
        assert_eq!(found[0].id, "json");
        assert!(found.iter().any(|e| e.id == entry.id));
        assert!(
            !ranked(catalog().iter(), "json", &preferences)
                .iter()
                .any(|e| e.id == entry.id)
        );
        assert_eq!(entry.match_hint("JSON"), "名称包含");
    }

    #[test]
    fn large_registry_reaches_last_entry_without_losing_exact_id() {
        let entries: Vec<_> = (0..300)
            .map(|i| {
                ToolEntry::new(
                    format!("fixture-{i:03}"),
                    Discovery {
                        label: format!("合成工作台 {i:03}"),
                        summary: "用于目录规模验证".into(),
                        category: "数据与格式".into(),
                        keywords: "fixture".into(),
                        aliases: Vec::new(),
                    },
                    Page::Data,
                    None,
                    false,
                )
            })
            .collect();
        assert_eq!(
            ranked(entries.iter(), "", &Preferences::default()).len(),
            300
        );
        let found = ranked(entries.iter(), "fixture-299", &Preferences::default());
        assert_eq!(found[0].id, "fixture-299");
        assert_eq!(found[0].page, Page::Data);
    }
}
