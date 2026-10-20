use super::*;
use std::{collections::BTreeSet, sync::OnceLock};

pub(super) struct Recommendation {
    pub target: Target,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Scope {
    #[default]
    All,
    Favorites,
    Recent,
    Frequent,
}

impl Scope {
    pub const ALL: [Self; 4] = [Self::All, Self::Favorites, Self::Recent, Self::Frequent];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Favorites => "收藏",
            Self::Recent => "最近",
            Self::Frequent => "常用",
        }
    }
}

/// Snapshot only tool IDs and counters once per dialog; never retain source data.
pub(super) struct Access {
    favorites: Vec<String>,
    recent: Vec<String>,
    usage: std::collections::BTreeMap<String, u32>,
}

impl Access {
    pub fn from_preferences(preferences: &crate::preferences::Preferences) -> Self {
        Self {
            favorites: preferences.favorites.clone(),
            recent: preferences.recent.clone(),
            usage: preferences.usage.clone(),
        }
    }

    pub fn filter(&self, targets: &mut Vec<Target>, scope: Scope, has_query: bool) {
        let position = |ids: &[String], target: Target| {
            ids.iter()
                .position(|id| id == &target.entry().id)
                .unwrap_or(usize::MAX)
        };
        targets.retain(|target| match scope {
            Scope::All => true,
            Scope::Favorites => self.favorites.contains(&target.entry().id),
            Scope::Recent => self.recent.contains(&target.entry().id),
            Scope::Frequent => self
                .usage
                .get(&target.entry().id)
                .is_some_and(|count| *count > 0),
        });
        // The incoming list is already relevance-ranked. Personal order only
        // applies when there is no query, including among format-specific routes.
        if has_query {
            return;
        }
        match scope {
            Scope::All => {}
            Scope::Favorites => targets.sort_by_key(|target| position(&self.favorites, *target)),
            Scope::Recent => targets.sort_by_key(|target| position(&self.recent, *target)),
            Scope::Frequent => targets.sort_by_key(|target| {
                (
                    std::cmp::Reverse(self.usage.get(&target.entry().id).copied().unwrap_or(0)),
                    position(&self.recent, *target),
                )
            }),
        }
    }
}

impl Target {
    pub(super) fn entry(self) -> &'static ToolEntry {
        let id = match self {
            Self::Tool(kind) => kind.id(),
            Self::Diagnostic(tool) => tool.id(),
            Self::Csv | Self::Tsv | Self::JsonData => "data",
            Self::TextFlow => "text-flow",
            Self::Before | Self::After => "diff",
            Self::Memo => "memos",
            Self::Event => "calendar-planner",
            Self::ArtifactHub => "artifact-hub",
            Self::KnowledgeCapture => "browser-clipper",
            Self::Calculator => "advanced-calculator",
            Self::Agent => "agent-runner",
            Self::FileEncoding => "encoding-detect",
        };
        catalog()
            .iter()
            .find(|entry| entry.id == id)
            .expect("handoff route discovery metadata")
    }
    pub(super) fn category(self) -> &'static str {
        &self.entry().category
    }
    fn score(self, query: &str) -> Option<u32> {
        let label = registry::normalized(self.label());
        let format = match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::JsonData => "json",
            Self::Before => "左侧",
            Self::After => "右侧",
            _ => "",
        };
        let local = if !query.is_empty() && (query == label || query == format) {
            Some(1100)
        } else if !query.is_empty() && query.split_whitespace().all(|word| label.contains(word)) {
            Some(500)
        } else {
            None
        };
        local
            .into_iter()
            .chain(self.entry().score_normalized(query))
            .max()
    }
}

pub(super) fn categories() -> &'static [String] {
    static CATEGORIES: OnceLock<Vec<String>> = OnceLock::new();
    CATEGORIES.get_or_init(|| {
        Target::all()
            .into_iter()
            .map(|target| target.category().to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    })
}

pub(super) fn search(query: &str, category: &str) -> Vec<Target> {
    let query = registry::normalized(query);
    let mut scored: Vec<_> = Target::all()
        .into_iter()
        .filter(|target| category.is_empty() || target.category() == category)
        .filter_map(|target| target.score(&query).map(|score| (score, target)))
        .collect();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.label().cmp(right.label()))
    });
    scored.into_iter().map(|(_, target)| target).collect()
}

/// Use the destination parser, but only for a small snapshot. A single column,
/// header-only input, or ambiguous delimiter is insufficient evidence to suggest.
fn delimited_target(text: &str) -> Option<Target> {
    if text.len() > 128 * 1024 {
        return None;
    }
    let accepts = |delimiter| {
        text.as_bytes().contains(&delimiter)
            && crate::workbench::Dataset::parse(text, crate::workbench::DataFormat::Csv, delimiter)
                .is_ok_and(|table| table.headers.len() >= 2 && !table.rows.is_empty())
    };
    match (accepts(b','), accepts(b'\t')) {
        (true, false) => Some(Target::Csv),
        (false, true) => Some(Target::Tsv),
        _ => None,
    }
}

/// Inspect only a bounded snapshot once, not the UI loop. Never run target actions.
pub(super) fn recommendations(text: &str) -> Vec<Recommendation> {
    let mut suggestions = Vec::new();
    if text.contains("Exception in thread") || text.contains("Caused by:") {
        suggestions.push(Recommendation {
            target: Target::Tool(ToolKind::JavaTrace),
            reason: "可能包含 Java 异常堆栈，可选择本机结构解析；不自动运行",
        });
    }
    if text.contains("Traceback (most recent call last):") {
        suggestions.push(Recommendation {
            target: Target::Tool(ToolKind::DjangoTrace),
            reason: "可能包含 Python traceback，可选择 Django 异常结构解析；不自动运行",
        });
    }
    if text.contains("[gc") {
        suggestions.push(Recommendation {
            target: Target::Diagnostic(crate::framework::Tool::Gc),
            reason: "可能包含 JVM GC 日志，可选择 GC 诊断；需主动分析",
        });
    }
    if NumericTable::read_json(text).is_ok() {
        suggestions.push(Recommendation {
            target: Target::Calculator,
            reason: "完整规范数值类型表格，可选择接收为矩阵A或B，不执行计算",
        });
    }
    if text.len() <= 128 * 1024 {
        if let Ok(value) =
            serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
        {
            if let Some(rows) = value
                .as_array()
                .filter(|rows| !rows.is_empty() && rows.len() <= 10000)
            {
                let mut keys = BTreeSet::new();
                let objects = rows.iter().all(|row| {
                    if let Some(object) = row.as_object() {
                        keys.extend(object.keys());
                        keys.len() <= 128
                    } else {
                        false
                    }
                });
                if objects && !keys.is_empty() {
                    suggestions.push(Recommendation {
                        target: Target::JsonData,
                        reason: "非空 JSON 对象数组，可在新数据实例中预览表格",
                    });
                }
            }
            suggestions.push(Recommendation {
                target: Target::Tool(ToolKind::Json),
                reason: "已识别为 JSON，可继续格式化或校验",
            });
        } else if let Some(target) = delimited_target(text) {
            suggestions.push(Recommendation {
                target,
                reason: if target == Target::Tsv {
                    "可能是 TSV 表格，已按数据工作台规则检查；确认后在新实例中解析预览"
                } else {
                    "可能是 CSV 表格，已按数据工作台规则检查；确认后在新实例中解析预览"
                },
            });
        }
        suggestions.push(Recommendation {
            target: Target::Memo,
            reason: "创建本地备忘草稿，明确保存后才写入",
        });
        if suggestions.len() < 3 {
            suggestions.push(Recommendation {
                target: Target::Event,
                reason: "创建日程草稿，日期需确认，提醒默认关闭",
            });
        }
    }
    if suggestions.len() < 3 {
        suggestions.push(Recommendation {
            target: Target::Before,
            reason: "放到对比左侧，再选择另一份文字；需主动运行对比",
        });
    }
    suggestions.truncate(3);
    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_wheel_smoothing_is_consumed_until_a_new_wheel_event() {
        let ctx = egui::Context::default();
        let mut transfer = Transfer::new("fixture".into(), "text").unwrap();
        for remaining in [300.0, 50.0, 6.0, 0.0] {
            ctx.input_mut(|input| {
                input.raw_scroll_delta = egui::Vec2::ZERO;
                input.smooth_scroll_delta = egui::vec2(0.0, -remaining);
            });
            transfer.discard_old_wheel(&ctx, false);
            assert_eq!(
                ctx.input(|input| input.smooth_scroll_delta),
                egui::Vec2::ZERO
            );
        }
        ctx.input_mut(|input| {
            input.raw_scroll_delta = egui::vec2(0.0, -120.0);
            input.smooth_scroll_delta = egui::vec2(0.0, -20.0);
        });
        transfer.discard_old_wheel(&ctx, false);
        assert_eq!(ctx.input(|input| input.smooth_scroll_delta.y), -20.0);
        ctx.input_mut(|input| {
            input.raw_scroll_delta = egui::Vec2::ZERO;
            input.smooth_scroll_delta = egui::vec2(0.0, -10.0);
        });
        transfer.discard_old_wheel(&ctx, false);
        assert_eq!(ctx.input(|input| input.smooth_scroll_delta.y), -10.0);
        transfer.discard_old_wheel(&ctx, true);
        assert_eq!(
            ctx.input(|input| input.smooth_scroll_delta),
            egui::Vec2::ZERO
        );
        assert_eq!(transfer.text, "text");
    }

    #[test]
    fn personal_scopes_follow_tool_order_but_queries_keep_format_relevance() {
        let mut preferences = crate::preferences::Preferences::default();
        preferences.favorites = vec!["memos".into(), "data".into(), "missing-tool".into()];
        preferences.recent = vec!["data".into(), "memos".into(), "recorder".into()];
        preferences.usage.insert("memos".into(), 8);
        preferences.usage.insert("data".into(), 8);
        preferences.usage.insert("json".into(), 0);
        let access = Access::from_preferences(&preferences);
        let mut targets = search("", "");
        access.filter(&mut targets, Scope::Favorites, false);
        assert_eq!(targets[0], Target::Memo);
        assert_eq!(targets.len(), 4);
        assert!(
            targets.contains(&Target::Csv)
                && targets.contains(&Target::Tsv)
                && targets.contains(&Target::JsonData)
        );
        for scope in [Scope::Recent, Scope::Frequent] {
            let mut targets = search("", "");
            access.filter(&mut targets, scope, false);
            assert_eq!(targets.len(), 4);
            assert!(matches!(
                targets[0],
                Target::Csv | Target::Tsv | Target::JsonData
            ));
            assert_eq!(targets.last(), Some(&Target::Memo));
        }
        for scope in [Scope::Favorites, Scope::Recent, Scope::Frequent] {
            let mut targets = search("tsv", "");
            access.filter(&mut targets, scope, true);
            assert_eq!(targets[0], Target::Tsv);
        }
        assert_eq!(preferences.usage["memos"], 8);
        assert_eq!(preferences.recent[0], "data");
    }

    #[test]
    fn personal_scope_changes_reset_once_and_preserve_selection_and_source() {
        let mut preferences = crate::preferences::Preferences::default();
        preferences.favorites = vec!["data".into()];
        let mut transfer = Transfer::new("fixture".into(), "name,count\nZi,2").unwrap();
        transfer.access = Some(Access::from_preferences(&preferences));
        transfer.matching_targets();
        transfer.reset_target_scroll = false;
        transfer.scope = Scope::Favorites;
        assert_eq!(transfer.matching_targets().len(), 3);
        assert!(std::mem::take(&mut transfer.reset_target_scroll));
        transfer.matching_targets();
        assert!(!transfer.reset_target_scroll);
        transfer.scope = Scope::Recent;
        assert!(transfer.matching_targets().is_empty());
        assert_eq!(transfer.target, Target::Tool(ToolKind::Json));
        assert_eq!(transfer.text, "name,count\nZi,2");
        assert!(transfer.new_data_instance);
    }

    #[test]
    fn favorites_never_expand_material_compatibility_or_change_during_dialog() {
        let mut preferences = crate::preferences::Preferences::default();
        preferences.favorites = vec!["data".into(), "json".into()];
        let table = crate::workbench::Dataset::parse(
            "name,count\nZi,2",
            crate::workbench::DataFormat::Csv,
            b',',
        )
        .unwrap();
        let mut transfer = Transfer::material("fixture".into(), Material::Table(table)).unwrap();
        transfer.access = Some(Access::from_preferences(&preferences));
        transfer.scope = Scope::Favorites;
        preferences.favorites.clear();
        assert_eq!(transfer.matching_targets(), [Target::JsonData]);
        assert!(transfer.compatible());
    }

    #[test]
    fn aliases_normalization_and_format_specific_ranking_share_catalog_metadata() {
        assert_eq!(search("reminder", "")[0], Target::Event);
        assert_eq!(
            search("ｂａｓｅ６４", "")[0],
            Target::Tool(ToolKind::Base64)
        );
        assert_eq!(search("CSV", "")[0], Target::Csv);
        assert_eq!(search("TSV", "")[0], Target::Tsv);
        assert_eq!(search("右侧", "")[0], Target::After);
        assert!(search("本机 Agent", "").contains(&Target::Agent));
        assert!(search("收集对话", "").contains(&Target::KnowledgeCapture));
        assert!(search("not-a-real-target-7654321", "").is_empty());
        for target in Target::all() {
            assert!(!target.category().is_empty());
        }
    }

    #[test]
    fn knowledge_capture_handoff_is_text_only_and_obeys_its_document_limit() {
        let mut transfer = Transfer::new("agent-runner@1.2.3".into(), "answer").unwrap();
        transfer.target = Target::KnowledgeCapture;
        assert!(transfer.compatible());
        transfer.text = "x".repeat(crate::knowledge_capture::MAX_HANDOFF_BODY + 1);
        assert!(!transfer.compatible());
        assert!(Target::all().contains(&Target::KnowledgeCapture));
    }

    #[test]
    fn category_and_query_changes_invalidate_cached_matches_without_changing_selection() {
        let mut transfer = Transfer::new("report".into(), "untouched").unwrap();
        transfer.query = "reminder".into();
        assert_eq!(transfer.matching_targets()[0], Target::Event);
        transfer.category = Target::Csv.category().into();
        assert!(transfer.matching_targets().is_empty());
        transfer.query = "tsv".into();
        assert_eq!(transfer.matching_targets()[0], Target::Tsv);
        assert_eq!(transfer.text, "untouched");
        assert_eq!(transfer.target, Target::Tool(ToolKind::Json));
    }

    #[test]
    fn recommends_tables_only_for_valid_nonempty_bounded_object_arrays() {
        assert_eq!(
            recommendations(r#"[{"name":"Zi"},{"id":2}]"#)[0].target,
            Target::JsonData
        );
        for text in ["[]", "[{}]", "[1,2]", "[{},1]", "{invalid", "ordinary text"] {
            assert!(
                !recommendations(text)
                    .iter()
                    .any(|item| item.target == Target::JsonData)
            );
        }
        let object: serde_json::Map<_, _> = (0..129)
            .map(|i| (format!("k{i}"), serde_json::Value::Null))
            .collect();
        let text = serde_json::to_string(&vec![object]).unwrap();
        assert!(
            !recommendations(&text)
                .iter()
                .any(|item| item.target == Target::JsonData)
        );
        let large = "x".repeat(128 * 1024 + 1);
        assert!(
            !recommendations(&large)
                .iter()
                .any(|item| matches!(item.target, Target::Memo | Target::Event | Target::JsonData))
        );
    }

    #[test]
    fn recommends_delimited_tables_using_destination_parser() {
        for (text, target) in [
            ("name,count\nZi,2\n工具,3", Target::Csv),
            ("\u{feff}name,count\r\nZi,2\r\n", Target::Csv),
            ("name,note\nZi,\"a,b\nnext line\"", Target::Csv),
            ("name\tcount\nZi\t2\n工具\t3", Target::Tsv),
            ("name\tnote\nZi\t\"a\tb\"", Target::Tsv),
        ] {
            assert_eq!(recommendations(text)[0].target, target);
            let delimiter = if target == Target::Csv { b',' } else { b'\t' };
            let parsed = crate::workbench::Dataset::parse(
                text,
                crate::workbench::DataFormat::Csv,
                delimiter,
            )
            .unwrap();
            assert!(parsed.headers.len() >= 2 && !parsed.rows.is_empty());
        }
    }

    #[test]
    fn uncertain_delimited_input_does_not_override_manual_selection() {
        for text in [
            "name,count",
            "name\nZi",
            "name,name\n1,2",
            "name,\nZi,2",
            "name,count\nZi,2,3",
            "name,count\nZi",
            "name\tcount\nZi",
            "a,b\tc\n1,2\t3",
            "ordinary text",
            "{\n\"a\":1,\n\"b\":2\n}",
        ] {
            assert!(
                !recommendations(text)
                    .iter()
                    .any(|r| matches!(r.target, Target::Csv | Target::Tsv)),
                "{text}"
            );
        }
        let large = format!("name,note\nZi,{}", "x".repeat(128 * 1024));
        assert_eq!(delimited_target(&large), None);
        let many_rows = format!("name,count\n{}", "Zi,2\n".repeat(10001));
        assert_eq!(delimited_target(&many_rows), None);
        let text = "name,count\nZi,2";
        let transfer = Transfer::new("fixture".into(), text).unwrap();
        assert_eq!(transfer.target, Target::Tool(ToolKind::Json));
        assert_eq!(transfer.text, text);
        assert!(transfer.new_data_instance);
    }

    #[test]
    fn scroll_reset_is_consumed_once_per_new_transfer_or_distinct_filter() {
        let mut transfer = Transfer::new("fixture".into(), "unchanged source").unwrap();
        assert!(std::mem::take(&mut transfer.reset_body_scroll));
        assert!(!std::mem::take(&mut transfer.reset_body_scroll));
        transfer.matching_targets();
        assert!(std::mem::take(&mut transfer.reset_target_scroll));
        transfer.matching_targets();
        assert!(!std::mem::take(&mut transfer.reset_target_scroll));
        transfer.query = "csv".into();
        transfer.matching_targets();
        assert!(std::mem::take(&mut transfer.reset_target_scroll));
        transfer.query = " ＣＳＶ ".into();
        transfer.matching_targets();
        assert!(!std::mem::take(&mut transfer.reset_target_scroll));
        transfer.target = Target::Csv;
        transfer.matching_targets();
        assert!(!std::mem::take(&mut transfer.reset_target_scroll));
        transfer.category = Target::Csv.category().into();
        transfer.matching_targets();
        assert!(std::mem::take(&mut transfer.reset_target_scroll));
        assert!(!transfer.reset_body_scroll);
        assert_eq!(transfer.text, "unchanged source");
        assert_eq!(transfer.target, Target::Csv);
        let fresh = Transfer::new("next".into(), "another source").unwrap();
        assert!(fresh.reset_body_scroll && fresh.reset_target_scroll);
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn offline_diagnostics_and_trace_hints_are_discoverable_without_running() {
        assert!(search("java gc", "").contains(&Target::Diagnostic(crate::framework::Tool::Gc)));
        assert!(
            search("django sql", "").contains(&Target::Diagnostic(crate::framework::Tool::Sql))
        );
        assert_eq!(
            Target::all()
                .iter()
                .filter(|target| matches!(target, Target::Diagnostic(_)))
                .count(),
            11
        );
        for tool in crate::framework::Tool::ALL {
            assert_eq!(
                Target::all().contains(&Target::Diagnostic(tool)),
                tool.accepts_text_handoff()
            );
        }
        assert_eq!(
            recommendations("Exception in thread demo")[0].target,
            Target::Tool(ToolKind::JavaTrace)
        );
        assert_eq!(
            recommendations("Traceback (most recent call last):")[0].target,
            Target::Tool(ToolKind::DjangoTrace)
        );
        assert_eq!(
            recommendations("[0.1s][gc] GC(0)")[0].target,
            Target::Diagnostic(crate::framework::Tool::Gc)
        );
    }
}
