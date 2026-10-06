//! Opt-in, bounded clipboard history. No persistence or network access.
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod storage;
mod ui;
pub use ui::State;

const ITEM_LIMIT: usize = 500;
const TEXT_LIMIT: usize = 1024 * 1024;
const BYTE_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub text: String,
    pub source: String,
    pub time: String,
    pub pinned: bool,
}
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub entries: Vec<Entry>,
    next: u64,
}
impl History {
    pub fn insert(&mut self, text: String, source: String) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        if text.len() > TEXT_LIMIT {
            return Err("单条文本超过1 MiB，未采集".into());
        }
        let existing = self.entries.iter().position(|e| e.text == text);
        let mut entry = if let Some(i) = existing {
            self.entries.remove(i)
        } else {
            self.next = self.next.checked_add(1).ok_or("历史标识已耗尽")?;
            Entry {
                id: self.next,
                text,
                source: String::new(),
                time: String::new(),
                pinned: false,
            }
        };
        entry.source = source;
        entry.time = chrono::Local::now().format("%m-%d %H:%M:%S").to_string();
        // Preflight eviction before mutating history: pinned content is never discarded.
        let mut bytes = self.entries.iter().map(|e| e.text.len()).sum::<usize>() + entry.text.len();
        let mut count = self.entries.len() + 1;
        let mut evict = Vec::new();
        for old in self.entries.iter().rev().filter(|e| !e.pinned) {
            if count <= ITEM_LIMIT && bytes <= BYTE_LIMIT {
                break;
            }
            evict.push(old.id);
            count -= 1;
            bytes -= old.text.len();
        }
        if count > ITEM_LIMIT || bytes > BYTE_LIMIT {
            if let Some(index) = existing {
                self.entries.insert(index, entry);
            }
            return Err("置顶内容已占满容量，请取消部分置顶或删除条目".into());
        }
        self.entries.retain(|e| !evict.contains(&e.id));
        self.entries.insert(0, entry);
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.text.len()).sum()
    }
    pub fn combined(&self, selected: &[u64], format: usize) -> Result<String, String> {
        let parts: Vec<&str> = selected
            .iter()
            .map(|id| {
                self.entries
                    .iter()
                    .find(|e| e.id == *id)
                    .map(|e| e.text.as_str())
                    .ok_or("选择的历史已失效，请重新选择".to_string())
            })
            .collect::<Result<_, _>>()?;
        if parts.is_empty() {
            return Err("先选择历史条目".into());
        }
        if parts.iter().map(|s| s.len()).sum::<usize>() > TEXT_LIMIT {
            return Err("组合原文超过1 MiB，请减少选择".into());
        }
        let value = match format {
            1 => parts
                .iter()
                .enumerate()
                .map(|(i, t)| format!("{}. {}", i + 1, t))
                .collect::<Vec<_>>()
                .join("\n"),
            2 => serde_json::to_string_pretty(&parts).map_err(|_| "JSON组合失败")?,
            3 => {
                let mut writer = csv::Writer::from_writer(Vec::new());
                writer.write_record(["text"]).map_err(|_| "CSV组合失败")?;
                for text in parts {
                    let safe = if text.trim_start().starts_with(['=', '+', '-', '@'])
                        || text.starts_with(['\t', '\r'])
                    {
                        format!("'{text}")
                    } else {
                        text.to_string()
                    };
                    writer.write_record([safe]).map_err(|_| "CSV组合失败")?;
                }
                String::from_utf8(writer.into_inner().map_err(|_| "CSV组合失败")?)
                    .map_err(|_| "CSV编码失败")?
            }
            _ => parts.join("\n\n"),
        };
        if value.len() > TEXT_LIMIT {
            return Err("组合结果超过1 MiB，请减少选择".into());
        }
        Ok(value)
    }
}

#[cfg(all(windows, feature = "ui-preview"))]
pub use native::isolated_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_eviction_and_formula_export_preserve_original_text() {
        let mut h = History::default();
        for i in 0..33 {
            h.insert(
                format!("{i:02}{}", "x".repeat(TEXT_LIMIT - 2)),
                "fixture".into(),
            )
            .unwrap();
        }
        assert_eq!(h.bytes(), BYTE_LIMIT);
        assert_eq!(h.entries.len(), 32);
        assert!(!h.entries.iter().any(|e| e.text.starts_with("00")));
        let ids = h.entries.iter().take(2).map(|e| e.id).collect::<Vec<_>>();
        assert!(h.combined(&ids, 2).is_err());
        h.entries.clear();
        h.insert("=1+2".into(), "fixture".into()).unwrap();
        let csv = h.combined(&[h.entries[0].id], 3).unwrap();
        let row = csv::Reader::from_reader(csv.as_bytes())
            .records()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(&row[0], "'=1+2");
        assert_eq!(h.entries[0].text, "=1+2");
    }
    #[test]
    fn dedup_retains_pin_and_id_updates_source_and_recency() {
        let mut h = History::default();
        h.insert("甲".into(), "A".into()).unwrap();
        h.entries[0].pinned = true;
        let id = h.entries[0].id;
        h.insert("乙".into(), "B".into()).unwrap();
        h.insert("甲".into(), "C".into()).unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].id, id);
        assert!(h.entries[0].pinned);
        assert_eq!(h.entries[0].source, "C");
    }
    #[test]
    fn limits_preserve_pins_and_reject_without_losing_content() {
        let mut h = History::default();
        for i in 0..ITEM_LIMIT {
            h.insert(i.to_string(), "fixture".into()).unwrap();
        }
        h.entries.last_mut().unwrap().pinned = true;
        h.insert("new".into(), "fixture".into()).unwrap();
        assert_eq!(h.entries.len(), ITEM_LIMIT);
        assert!(h.entries.iter().any(|e| e.text == "0"));
        for e in &mut h.entries {
            e.pinned = true;
        }
        let before = h.entries.iter().map(|e| e.id).collect::<Vec<_>>();
        assert!(h.insert("full".into(), "fixture".into()).is_err());
        assert_eq!(before, h.entries.iter().map(|e| e.id).collect::<Vec<_>>());
        assert!(
            h.insert("x".repeat(TEXT_LIMIT + 1), "fixture".into())
                .is_err()
        );
        h.insert("new".into(), "other".into()).unwrap();
        assert_eq!(h.entries.len(), ITEM_LIMIT);
    }
    #[test]
    fn combination_preserves_selection_order_and_roundtrips_multiline() {
        let mut h = History::default();
        h.insert("a,\"b\"\n甲".into(), "fixture".into()).unwrap();
        let a = h.entries[0].id;
        h.insert("乙".into(), "fixture".into()).unwrap();
        let b = h.entries[0].id;
        let json: Vec<String> = serde_json::from_str(&h.combined(&[a, b], 2).unwrap()).unwrap();
        assert_eq!(json, vec!["a,\"b\"\n甲", "乙"]);
        let csv = h.combined(&[a, b], 3).unwrap();
        let rows = csv::Reader::from_reader(csv.as_bytes())
            .records()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(&rows[0][0], "a,\"b\"\n甲");
        assert!(h.combined(&[999], 0).is_err());
        assert!(h.combined(&[], 0).is_err());
    }
}
