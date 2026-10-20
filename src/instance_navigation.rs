//! Metadata only: no material reads, task execution or snapshot restoration.
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Data,
    Image,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub kind: Kind,
    pub id: String,
}
impl Key {
    pub fn token(&self) -> String {
        format!("instance:{:?}:{}", self.kind, self.id)
    }
}
#[derive(Clone)]
pub(crate) struct Entry {
    pub key: Key,
    pub name: String,
    pub tool: String,
    pub busy: bool,
    pub pending: bool,
}
fn normalized(text: &str) -> String {
    text.nfkc().collect::<String>().to_lowercase()
}
pub(crate) fn search(mut entries: Vec<Entry>, query: &str, recent: &[Key]) -> Vec<Entry> {
    let query = normalized(query.trim());
    entries.retain(|entry| {
        let kind = match entry.key.kind {
            Kind::Data => "数据 表格 data",
            Kind::Image => "图片 图像 image",
        };
        let text = normalized(&format!("{} {} {kind} 工作实例", entry.name, entry.tool));
        query.split_whitespace().all(|word| text.contains(word))
    });
    entries.sort_by_key(|entry| {
        let name = normalized(&entry.name);
        let score = if query.is_empty() || name == query {
            0
        } else if name.starts_with(&query) {
            1
        } else if name.contains(&query) {
            2
        } else {
            3
        };
        (
            score,
            recent
                .iter()
                .position(|key| key == &entry.key)
                .unwrap_or(usize::MAX),
        )
    });
    entries
}

/// Preserve the selected identity across live result changes. A disappeared row
/// requires an explicit selection gesture before Enter can open another target.
#[derive(Default)]
pub(crate) struct Selection {
    context: String,
    keys: Vec<String>,
    pub armed: bool,
}
impl Selection {
    pub fn reconcile(&mut self, context: String, keys: Vec<String>, index: &mut usize) {
        if self.context != context {
            self.context = context;
            *index = 0;
            self.armed = true;
        } else if self.armed
            && let Some(old) = self.keys.get(*index)
        {
            if let Some(position) = keys.iter().position(|key| key == old) {
                *index = position;
            } else {
                self.armed = false;
            }
        }
        *index = (*index).min(keys.len().saturating_sub(1));
        self.keys = keys;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(n: usize) -> Entry {
        Entry {
            key: Key {
                kind: Kind::Data,
                id: n.to_string(),
            },
            name: format!("材料 {n}"),
            tool: "data".into(),
            busy: false,
            pending: false,
        }
    }
    #[test]
    fn thousand_metadata_rows_keep_exact_ranking_stable_identity_and_recent_order() {
        let entries: Vec<_> = (0..1000).map(entry).collect();
        let recent = [entries[999].key.clone(), entries[7].key.clone()];
        let all = search(entries.clone(), "", &recent);
        assert_eq!(all.len(), 1000);
        assert_eq!(all[0].key, recent[0]);
        assert_eq!(all[1].key, recent[1]);
        assert_eq!(all[2].key.id, "0");
        let found = search(entries, "材料 7", &recent);
        assert_eq!(found[0].key.id, "7");
        assert_eq!(found[1].key.id, "70");
        assert!(search(vec![entry(7)], "材料 nonexistent", &[]).is_empty());
    }
    #[test]
    fn same_name_across_kinds_stays_distinct_and_unicode_names_match() {
        let a = entry(1);
        let mut b = a.clone();
        b.key.kind = Kind::Image;
        b.name = "ＪＳＯＮ".into();
        let rows = search(vec![a, b.clone()], "json", &[]);
        assert_eq!(rows[0].key, b.key);
        assert_ne!(entry(1).key.token(), b.key.token());
    }
    #[test]
    fn live_reorder_keeps_selection_and_removed_target_needs_explicit_reselection() {
        let mut state = Selection::default();
        let mut index = 0;
        state.reconcile("main:test".into(), vec!["a".into(), "b".into()], &mut index);
        index = 1;
        state.reconcile("main:test".into(), vec!["b".into(), "a".into()], &mut index);
        assert_eq!(index, 0);
        assert!(state.armed);
        state.reconcile("main:test".into(), vec!["a".into()], &mut index);
        assert!(!state.armed);
        state.reconcile("main:test".into(), vec!["a".into()], &mut index);
        assert!(!state.armed);
        state.reconcile("main:new-query".into(), vec!["a".into()], &mut index);
        assert!(state.armed);
    }
}
