//! Bounded session navigation. A candidate is committed only after restoration.
use super::Location;
const CAPACITY: usize = 64;
#[derive(Default)]
pub(super) struct History {
    entries: Vec<Location>,
    cursor: usize,
}
impl History {
    pub fn observe(&mut self, location: Location) {
        if let Some(current) = self.entries.get_mut(self.cursor) {
            if current.same_place(&location) {
                *current = location;
                return;
            }
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(location);
        if self.entries.len() > CAPACITY {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }
    pub fn candidate(
        &self,
        backwards: bool,
        valid: impl Fn(&Location) -> bool,
    ) -> Option<(usize, Location, usize)> {
        let indices: Box<dyn Iterator<Item = usize>> = if backwards {
            Box::new((0..self.cursor).rev())
        } else {
            Box::new(self.cursor.saturating_add(1)..self.entries.len())
        };
        let current = self.entries.get(self.cursor)?;
        let mut skipped = 0;
        for index in indices {
            let location = &self.entries[index];
            if current.same_place(location) {
                continue;
            }
            if valid(location) {
                return Some((index, location.clone(), skipped));
            }
            skipped += 1;
        }
        None
    }
    pub fn commit(&mut self, index: usize) {
        assert!(index < self.entries.len());
        self.cursor = index;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::registry::Page;
    fn location(id: &str) -> Location {
        Location {
            page: Page::Data,
            tool: Some("data".into()),
            instance: Some(id.into()),
            title: id.into(),
            library: None,
        }
    }
    #[test]
    fn independent_instances_and_new_branch_keep_exact_positions() {
        let mut history = History::default();
        for id in ["a", "b", "c"] {
            history.observe(location(id));
        }
        let (index, target, _) = history.candidate(true, |_| true).unwrap();
        assert_eq!(target.instance.as_deref(), Some("b"));
        history.commit(index);
        history.observe(target);
        assert_eq!(
            history
                .candidate(false, |_| true)
                .unwrap()
                .1
                .instance
                .as_deref(),
            Some("c")
        );
        history.observe(location("d"));
        assert!(history.candidate(false, |_| true).is_none());
        assert_eq!(
            history
                .candidate(true, |_| true)
                .unwrap()
                .1
                .instance
                .as_deref(),
            Some("b")
        );
    }
    #[test]
    fn inspection_and_failed_restore_never_move_history_cursor() {
        let mut history = History::default();
        for id in ["a", "closed", "c"] {
            history.observe(location(id));
        }
        let candidate = history
            .candidate(true, |l| l.instance.as_deref() != Some("closed"))
            .unwrap();
        assert_eq!(candidate.1.instance.as_deref(), Some("a"));
        assert_eq!(candidate.2, 1);
        assert_eq!(history.cursor, 2);
        assert_eq!(history.candidate(true, |_| true).unwrap().0, 1);
        assert!(history.candidate(true, |_| false).is_none());
        assert_eq!(history.cursor, 2);
    }
    #[test]
    fn typing_updates_context_without_history_spam_and_capacity_is_bounded() {
        let mut history = History::default();
        let mut first = location("a");
        history.observe(first.clone());
        first.title = "renamed instance".into();
        history.observe(first);
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].title, "renamed instance");
        for n in 0..100 {
            history.observe(location(&n.to_string()));
        }
        assert_eq!(history.entries.len(), 64);
        assert_eq!(history.entries[0].instance.as_deref(), Some("36"));
        assert_eq!(history.cursor, 63);
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use crate::app::history::Library;
    use crate::app::registry::Page;
    fn entry(id: &str) -> Location {
        Location {
            page: Page::DocumentIngestion,
            tool: Some(id.into()),
            instance: None,
            title: id.into(),
            library: None,
        }
    }
    #[test]
    fn aliases_on_one_page_remain_distinct_and_repeated_current_is_not_unavailable() {
        let mut history = History::default();
        for id in ["rag-ingestion", "ocr-documents", "rag-ingestion"] {
            history.observe(entry(id));
        }
        assert_eq!(history.entries.len(), 3);
        let (_, location, skipped) = history.candidate(true, |_| true).unwrap();
        assert_eq!(location.tool.as_deref(), Some("ocr-documents"));
        assert_eq!(skipped, 0);
        assert!(
            history
                .candidate(true, |l| l.tool.as_deref() != Some("ocr-documents"))
                .is_none()
        );
    }
    #[test]
    fn separate_library_visits_keep_their_query_filter_and_scroll_snapshots() {
        let mut history = History::default();
        let mut library = Location {
            page: Page::Library,
            tool: None,
            instance: None,
            title: "工具库".into(),
            library: Some(Library {
                query: "json".into(),
                filter: "收藏".into(),
                category: "全部分类".into(),
                scroll: 480.0,
            }),
        };
        history.observe(library.clone());
        history.observe(entry("ocr-documents"));
        library.library.as_mut().unwrap().query = "uuid".into();
        library.library.as_mut().unwrap().scroll = 0.0;
        history.observe(library);
        let (index, _, _) = history.candidate(true, |_| true).unwrap();
        history.commit(index);
        let (_, target, _) = history.candidate(true, |_| true).unwrap();
        let old = target.library.unwrap();
        assert_eq!(old.query, "json");
        assert_eq!(old.filter, "收藏");
        assert_eq!(old.scroll, 480.0);
    }
}
