//! Built-in discovery comes from the versioned catalog; routes stay executable Rust.
use super::ToolKind;
use crate::plugins::PluginToolVersionRecord;
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
    #[serde(default)]
    version_history: Vec<PluginToolVersionRecord>,
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
    pub version_history: Arc<[PluginToolVersionRecord]>,
    pub title: String,
    pub description: String,
    pub category: String,
    pub page: Page,
    pub kind: Option<ToolKind>,
    pub in_progress: bool,
    pub exact_match_alternatives: usize,
    search: Arc<SearchText>,
}

#[derive(Clone)]
struct SearchText {
    id: String,
    title: String,
    aliases: Vec<String>,
    text: String,
    version: Option<String>,
    historical_versions: Vec<String>,
    history_text: String,
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
            version_history: Arc::from([]),
            search: Arc::new(SearchText {
                id: normalized(&id),
                title,
                aliases,
                text,
                version: None,
                historical_versions: Vec::new(),
                history_text: String::new(),
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
            exact_match_alternatives: 0,
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
                aliases: tool.aliases.clone(),
            },
            Page::Plugins,
            None,
            false,
        );
        entry.set_version(tool.version.clone());
        entry.set_version_history(&tool.version_history);
        entry
    }

    fn set_version(&mut self, version: Option<String>) {
        self.version.clone_from(&version);
        let Some(version) = version else {
            return;
        };
        let version = normalized(&version);
        let search = Arc::make_mut(&mut self.search);
        search.text.push(' ');
        search.text.push_str(&version);
        search.version = Some(version);
    }

    fn set_version_history(&mut self, history: &[PluginToolVersionRecord]) {
        self.version_history = Arc::from(history.to_vec());
        let historical_versions = history
            .iter()
            .map(|record| normalized(&record.version))
            .collect::<Vec<_>>();
        let history_text = normalized(
            &history
                .iter()
                .map(|record| format!("{} {}", record.version, record.changes))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let search = Arc::make_mut(&mut self.search);
        if !history_text.is_empty() {
            search.text.push(' ');
            search.text.push_str(&history_text);
        }
        search.historical_versions = historical_versions;
        search.history_text = history_text;
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
        if search.version.as_deref() == Some(query) {
            return Some(850);
        }
        if search
            .historical_versions
            .iter()
            .any(|version| version == query)
        {
            return Some(820);
        }
        if query.contains('.')
            && search
                .version
                .as_deref()
                .is_some_and(|version| version.starts_with(query))
        {
            return Some(650);
        }
        if query.contains('.')
            && search
                .historical_versions
                .iter()
                .any(|version| version.starts_with(query))
        {
            return Some(630);
        }
        if search.title.starts_with(query) {
            return Some(700);
        }
        let all_terms_match = query
            .split_whitespace()
            .all(|word| search.text.contains(word));
        // Keep every term as a filter. Once they all match, distinguish a named
        // tool plus context from incidental mentions in descriptions/history.
        // Single-word, exact alias and version ranking retain their old rules.
        if all_terms_match && query.contains(' ') {
            if query
                .split_whitespace()
                .any(|word| word == search.id || word == search.title)
            {
                return Some(550);
            }
            if query
                .split_whitespace()
                .all(|word| search.title.contains(word))
            {
                return Some(500);
            }
            if query
                .split_whitespace()
                .any(|word| search.title.contains(word) || search.id.contains(word))
            {
                return Some(450);
            }
            if query
                .split_whitespace()
                .any(|word| search.aliases.iter().any(|alias| alias.contains(word)))
            {
                return Some(425);
            }
        }
        if !search.history_text.is_empty()
            && query
                .split_whitespace()
                .all(|word| search.history_text.contains(word))
        {
            return Some(400);
        }
        if all_terms_match {
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
            Some(850) => "版本匹配",
            Some(820) => "历史版本匹配",
            Some(650) => "版本前缀匹配",
            Some(630) => "历史版本前缀匹配",
            Some(400) => "版本历史匹配",
            Some(700 | 500) => "名称包含",
            Some(550 | 450) => "名称与关键词匹配",
            Some(425) => "别名与关键词匹配",
            Some(100 | 180 | 230) => "近似拼写",
            Some(300) => "说明 / 关键词匹配",
            _ => "",
        }
    }

    fn is_exact_name_or_alias(&self, query: &str) -> bool {
        self.search.id == query
            || self.search.title == query
            || self.search.aliases.iter().any(|alias| alias == query)
    }

    pub fn match_detail(&self, query: &str) -> String {
        let hint = self.match_hint(query);
        if self.exact_match_alternatives == 0 {
            hint.to_owned()
        } else {
            format!("{hint} · 同词还有 {} 个入口", self.exact_match_alternatives)
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
                entry.set_version(Some(definition.tool_version));
                entry.set_version_history(&definition.version_history);
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
    let entries: Vec<_> = entries.collect();
    let exact_count = entries
        .iter()
        .filter(|entry| entry.is_exact_name_or_alias(&query))
        .count();
    let alternatives = exact_count.saturating_sub(1);
    let mut scored: Vec<_> = entries
        .iter()
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
                let mut entry = (**e).clone();
                if alternatives > 0 && entry.is_exact_name_or_alias(&query) {
                    entry.exact_match_alternatives = alternatives;
                }
                (rank, entry)
            })
        })
        .collect();
    scored.sort_by(|(left_rank, left), (right_rank, right)| {
        right_rank
            .cmp(left_rank)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
    scored.into_iter().map(|(_, entry)| entry).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Home,
    Library,
    Artifacts,
    Intake,
    Tasks,
    Notes,
    Calendar,
    Calculator,
    Clipboard,
    Clock,
    TimeZone,
    Updates,
    DeltaUpdate,
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
    FileRename,
    LogViewer,
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
            "artifact-hub" => Page::Artifacts,
            "memos" => Page::Notes,
            "calendar-planner" => Page::Calendar,
            "advanced-calculator" => Page::Calculator,
            "super-clipboard" => Page::Clipboard,
            "clock-workbench" => Page::Clock,
            "time-zone" => Page::TimeZone,
            "app-update-check" => Page::Updates,
            "app-delta-update" => Page::DeltaUpdate,
            "screenshot-workbench" => Page::Images,
            "unified-shortcuts" => Page::Commands,
            "global-launcher" => Page::Settings,
            "file-intake" => Page::Intake,
            "data" => Page::Data,
            "data-sqlite-export" => Page::Data,
            "pipeline" => Page::Data,
            "text-flow" => Page::Data,
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
            "image-workflow" => Page::Images,
            "image-metadata" => Page::Images,
            "image-crop-annotate" => Page::Images,
            "markdown" => Page::Markdown,
            "encoding-detect" => Page::FileEncoding,
            "file-rename" => Page::FileRename,
            "log-viewer" => Page::LogViewer,
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
            "ocr-documents" => Page::DocumentIngestion,
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
            "recorder-tutorial" => Page::Recorder,
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
    fn every_registered_image_entry_selects_its_exact_view() {
        let mut images = crate::image_tools::State::default();
        let entries: Vec<_> = catalog()
            .iter()
            .filter(|entry| entry.page == Page::Images)
            .collect();
        assert_eq!(entries.len(), 6);
        for source in &entries {
            for target in &entries {
                assert!(images.select_entry(&source.id));
                assert!(images.select_entry(&target.id));
                assert_eq!(images.active_tool_id(), target.id);
            }
        }
    }

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
    fn routed_aliases_are_unique_and_disambiguated_queries_have_one_exact_match() {
        let mut alias_owners = std::collections::HashMap::new();
        for entry in catalog() {
            for alias in &entry.search.aliases {
                if let Some(owner) = alias_owners.insert(alias.clone(), entry.id.as_str()) {
                    assert_eq!(
                        owner,
                        entry.id.as_str(),
                        "normalized alias {alias:?} is shared by routed tools"
                    );
                }
            }
        }

        for (query, expected) in [
            ("数据转换管线", "pipeline"),
            ("表格流程", "text-flow"),
            ("闹钟", "clock-workbench"),
            ("事件提醒", "calendar-planner"),
        ] {
            let exact_matches: Vec<_> = catalog()
                .iter()
                .filter(|entry| entry.score(query) == Some(900))
                .map(|entry| entry.id.as_str())
                .collect();
            assert_eq!(exact_matches, vec![expected], "{query}");
        }
    }

    #[test]
    fn every_routed_id_name_and_alias_searches_back_to_its_tool() {
        let entries = catalog();
        let preferences = Preferences::default();
        for entry in entries {
            let terms = std::iter::once(entry.id.as_str())
                .chain(std::iter::once(entry.title.as_str()))
                .chain(entry.search.aliases.iter().map(String::as_str));
            for term in terms {
                let found = ranked(entries.iter(), term, &preferences);
                assert_eq!(
                    found.first().map(|candidate| candidate.id.as_str()),
                    Some(entry.id.as_str()),
                    "exact catalog term {term:?} should resolve to {}",
                    entry.id
                );
            }
        }
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
            ("正则替换", "regex-replace"),
            ("批量替换文本", "regex-replace"),
            ("regex replace", "regex-replace"),
            ("批量重命名", "file-rename"),
            ("文件名替换", "file-rename"),
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
            version_history: vec![
                PluginToolVersionRecord {
                    version: "1.1.0".into(),
                    date: "2026-10-10".into(),
                    changes: "增加 JSON 校验规则".into(),
                },
                PluginToolVersionRecord {
                    version: "1.2.3".into(),
                    date: "2026-10-16".into(),
                    changes: "修复示例工具行为".into(),
                },
            ],
            name: "JSON Helper".into(),
            description: "处理 JSON 文本".into(),
            category: "数据与格式".into(),
            keywords: vec!["json".into()],
            aliases: vec!["translate text".into(), "翻译文本".into()],
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
        assert_eq!(
            entry.version_history.as_ref(),
            plugin.version_history.as_slice()
        );
        assert_eq!(entry.score("translate text"), Some(900));
        assert_eq!(entry.score("翻译文本"), Some(900));
        assert_eq!(entry.score("1.2.3"), Some(850));
        assert_eq!(entry.score("1.2"), Some(650));
        assert_eq!(entry.score("1.1.0"), Some(820));
        assert_eq!(entry.score("1.1"), Some(630));
        assert_eq!(entry.score("校验规则"), Some(400));
        assert_eq!(entry.match_hint("翻译文本"), "用途 / 别名匹配");
        assert_eq!(entry.match_hint("1.2.3"), "版本匹配");
        assert_eq!(entry.match_hint("1.2"), "版本前缀匹配");
        assert_eq!(entry.match_hint("1.1.0"), "历史版本匹配");
        assert_eq!(entry.match_hint("校验规则"), "版本历史匹配");
        assert!(
            ranked(
                catalog().iter().chain(std::iter::once(&entry)),
                "1.2.3",
                &Preferences::default()
            )
            .iter()
            .any(|candidate| candidate.id == entry.id)
        );
        assert!(
            ranked(
                catalog().iter().chain(std::iter::once(&entry)),
                "校验规则",
                &Preferences::default()
            )
            .iter()
            .any(|candidate| candidate.id == entry.id)
        );
        for alias in ["translate text", "翻译文本"] {
            let found = ranked(
                catalog().iter().chain(std::iter::once(&entry)),
                alias,
                &Preferences::default(),
            );
            assert_eq!(
                found.first().map(|tool| tool.id.as_str()),
                Some(entry.id.as_str())
            );
        }
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
    fn exact_name_or_alias_collisions_are_visible_when_plugins_extend_the_directory() {
        let duplicate = crate::plugins::PluginTool {
            id: "formatter".into(),
            version: Some("1.0.0".into()),
            version_history: Vec::new(),
            name: "JSON Formatter Plugin".into(),
            description: "格式化 JSON 内容".into(),
            category: "数据与格式".into(),
            keywords: vec!["json".into()],
            aliases: vec!["json格式化".into()],
            sample: String::new(),
            model: String::new(),
            adapter: crate::plugins::Adapter::Builtin {
                tool: "json".into(),
                action: 0,
                pattern: String::new(),
            },
        };
        let plugin = ToolEntry::plugin("plugin:test/formatter".into(), &duplicate);
        let found = ranked(
            catalog().iter().chain(std::iter::once(&plugin)),
            "json格式化",
            &Preferences::default(),
        );
        let exact: Vec<_> = found
            .iter()
            .filter(|entry| entry.exact_match_alternatives > 0)
            .collect();
        assert_eq!(exact.len(), 2);
        assert!(
            exact
                .iter()
                .all(|entry| entry.exact_match_alternatives == 1)
        );
        assert!(exact.iter().all(|entry| {
            entry
                .match_detail("json格式化")
                .contains("同词还有 1 个入口")
        }));
        assert_eq!(found[0].id, "json");

        let mut title_duplicate = duplicate;
        title_duplicate.id = "formatter-by-name".into();
        title_duplicate.name = "json格式化".into();
        title_duplicate.aliases.clear();
        let title_plugin =
            ToolEntry::plugin("plugin:test/formatter-by-name".into(), &title_duplicate);
        let title_found = ranked(
            catalog().iter().chain(std::iter::once(&title_plugin)),
            "json格式化",
            &Preferences::default(),
        );
        let exact_title: Vec<_> = title_found
            .iter()
            .filter(|entry| entry.exact_match_alternatives > 0)
            .collect();
        assert_eq!(exact_title.len(), 2);
        assert!(
            exact_title
                .iter()
                .all(|entry| entry.exact_match_alternatives == 1)
        );
        assert_eq!(title_found[0].id, "plugin:test/formatter-by-name");
    }

    #[test]
    fn built_in_catalog_exposes_append_only_tool_version_history() {
        let entry = catalog()
            .iter()
            .find(|entry| entry.id == "sql-format")
            .unwrap();
        assert_eq!(entry.version.as_deref(), Some("0.3.1"));
        assert_eq!(entry.score("0.3.1"), Some(850));
        assert_eq!(entry.score("0.3.0"), Some(820));
        assert_eq!(entry.score("0.3"), Some(650));
        assert_eq!(entry.score("0.2.0"), Some(820));
        assert_eq!(entry.score("0.2"), Some(630));
        assert_eq!(entry.score("sqlformat"), Some(400));
        assert_eq!(entry.match_hint("0.3.1"), "版本匹配");
        assert_eq!(entry.match_hint("0.3.0"), "历史版本匹配");
        assert_eq!(entry.match_hint("0.2.0"), "历史版本匹配");
        assert_eq!(entry.match_hint("sqlformat"), "版本历史匹配");
        assert!(
            ranked(catalog().iter(), "0.3.1", &Preferences::default())
                .iter()
                .any(|candidate| candidate.id == entry.id)
        );
        assert!(
            ranked(catalog().iter(), "0.3.0", &Preferences::default())
                .iter()
                .any(|candidate| candidate.id == entry.id)
        );
        assert!(
            ranked(catalog().iter(), "0.2.0", &Preferences::default())
                .iter()
                .any(|candidate| candidate.id == entry.id)
        );
        assert!(
            ranked(catalog().iter(), "sqlformat", &Preferences::default())
                .iter()
                .any(|candidate| candidate.id == entry.id)
        );
        assert_eq!(
            entry
                .version_history
                .last()
                .map(|record| record.version.as_str()),
            entry.version.as_deref()
        );
        assert!(!entry.version_history.is_empty());
    }

    #[test]
    fn multi_word_search_keeps_the_named_tool_ahead_of_history_mentions() {
        for query in ["JSON 工具", "  ＪＳＯＮ   工具  "] {
            let found = ranked(catalog().iter(), query, &Preferences::default());
            assert_eq!(found.first().map(|entry| entry.id.as_str()), Some("json"));
        }
        assert!(
            ranked(
                catalog().iter(),
                "json unmatched987654",
                &Preferences::default()
            )
            .is_empty()
        );
    }

    #[test]
    fn name_relevance_beats_favorite_history_mentions_without_dropping_terms() {
        let make = |id: &str, title: &str, summary: &str| {
            ToolEntry::new(
                id.into(),
                Discovery {
                    label: title.into(),
                    summary: summary.into(),
                    category: "fixture".into(),
                    keywords: String::new(),
                    aliases: Vec::new(),
                },
                Page::Data,
                None,
                false,
            )
        };
        let named = make("json", "JSON", "offline parse");
        let mut history = make("history", "AAA history", "unrelated");
        history.set_version_history(&[PluginToolVersionRecord {
            version: "1.0.0".into(),
            date: "2026-10-18".into(),
            changes: "json offline parse".into(),
        }]);
        let mut preferences = Preferences::default();
        preferences.favorites.push(history.id.clone());
        preferences.visit(&history.id);
        preferences.usage.insert(history.id.clone(), u32::MAX);
        let entries = [history, named];
        let found = ranked(entries.iter(), "json parse", &preferences);
        assert_eq!(found[0].id, "json");
        assert_eq!(found[0].match_hint("json parse"), "名称与关键词匹配");
        assert_eq!(found[1].match_hint("json parse"), "版本历史匹配");
        assert!(ranked(entries.iter(), "json missing-term", &preferences).is_empty());
    }

    #[test]
    fn scattered_name_words_and_alias_context_have_distinct_match_reasons() {
        let entry = ToolEntry::new(
            "formatter".into(),
            Discovery {
                label: "Structured JSON Validator".into(),
                summary: "offline conversion".into(),
                category: "fixture".into(),
                keywords: String::new(),
                aliases: vec!["prettify".into()],
            },
            Page::Data,
            None,
            false,
        );
        assert_eq!(entry.score("validator structured"), Some(500));
        assert_eq!(entry.match_hint("validator structured"), "名称包含");
        assert_eq!(entry.match_hint("json offline"), "名称与关键词匹配");
        assert_eq!(entry.match_hint("prettify offline"), "别名与关键词匹配");
        assert_eq!(entry.score("prettify missing-term"), None);
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
