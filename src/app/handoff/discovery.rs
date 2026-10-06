use super::*;
use std::{collections::BTreeSet, sync::OnceLock};

pub(super) struct Recommendation {
    pub target: Target,
    pub reason: &'static str,
}

impl Target {
    fn entry(self) -> &'static ToolEntry {
        let id = match self {
            Self::Tool(kind) => kind.id(),
            Self::Csv | Self::Tsv | Self::JsonData => "data",
            Self::Before | Self::After => "diff",
            Self::Memo => "memos",
            Self::Event => "calendar-planner",
            Self::Calculator => "advanced-calculator",
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

/// Inspect only a bounded snapshot once, not the UI loop. Never run target actions.
pub(super) fn recommendations(text: &str) -> Vec<Recommendation> {
    let mut suggestions = Vec::new();
    if NumericTable::read_json(text).is_ok() {
        suggestions.push(Recommendation {
            target: Target::Calculator,
            reason: "完整规范数值类型表格，可确认接收为矩阵A，不执行计算",
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
    fn aliases_normalization_and_format_specific_ranking_share_catalog_metadata() {
        assert_eq!(search("reminder", "")[0], Target::Event);
        assert_eq!(
            search("ｂａｓｅ６４", "")[0],
            Target::Tool(ToolKind::Base64)
        );
        assert_eq!(search("CSV", "")[0], Target::Csv);
        assert_eq!(search("TSV", "")[0], Target::Tsv);
        assert_eq!(search("右侧", "")[0], Target::After);
        assert!(search("not-a-real-target-7654321", "").is_empty());
        for target in Target::all() {
            assert!(!target.category().is_empty());
        }
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
}
