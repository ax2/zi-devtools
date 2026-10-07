//! Bounded tray workflow summaries; opening uses the ordinary read/review path.
use super::*;
pub(super) struct Shortcuts {
    pub entries: Vec<(crate::preferences::SavedWorkflow, &'static str)>,
    pub total: usize,
}
pub(super) fn shortcuts(prefs: &Preferences, tab: &str, query: &str) -> Shortcuts {
    let mut entries = Vec::new();
    let mut total = 0;
    if matches!(tab, "收藏" | "全部" | "流程") {
        let matching: Vec<_> = prefs
            .workflow_favorites
            .iter()
            .filter(|entry| entry.valid() && entry.matches(query))
            .collect();
        total += matching.len();
        entries.extend(
            matching
                .into_iter()
                .take(6)
                .map(|entry| (entry.clone(), "收藏流程")),
        );
    }
    if matches!(tab, "最近" | "全部" | "流程") {
        let matching: Vec<_> = prefs
            .workflow_recent
            .iter()
            .filter(|entry| {
                entry.valid()
                    && entry.matches_recent(query)
                    && (tab == "最近"
                        || !prefs
                            .workflow_favorites
                            .iter()
                            .any(|favorite| favorite.path == entry.path))
            })
            .collect();
        total += matching.len();
        entries.extend(
            matching
                .into_iter()
                .take(6)
                .map(|entry| (entry.clone(), "最近载入")),
        );
    }
    Shortcuts { entries, total }
}
impl DevToolsApp {
    pub(super) fn open_tray_workflow(
        &mut self,
        ctx: &egui::Context,
        entry: crate::preferences::SavedWorkflow,
    ) {
        self.quick_open = false;
        restore_main_window(self.window_handle, ctx);
        match self
            .preferences
            .workflow_shortcut_current(&self.preferences_path, &entry)
        {
            Ok(true) => self.open_search_choice(Choice::SavedWorkflow(entry)),
            Ok(false) => {
                let _ = self.preferences.refresh_discovery(&self.preferences_path);
                self.toast = Some((
                    "该流程入口已移除或变化，请重新打开托盘面板。".into(),
                    Instant::now(),
                ));
            }
            Err(error) => {
                self.toast = Some((format!("无法复核流程入口：{error:#}"), Instant::now()))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_workflow_shortcuts_bound_deduplicate_and_keep_recent_semantics() {
        let mut prefs = Preferences::default();
        for i in 0..100 {
            let entry = crate::preferences::SavedWorkflow {
                path: std::env::temp_dir().join(format!("flow-{i}.json")),
                name: format!("流程{i}"),
                steps: 2,
            };
            prefs.workflow_favorites.push(entry.clone());
            if i < 20 {
                prefs.workflow_recent.push(entry);
            }
        }
        assert_eq!(shortcuts(&prefs, "收藏", "").entries.len(), 6);
        assert_eq!(shortcuts(&prefs, "收藏", "").total, 100);
        let all = shortcuts(&prefs, "流程", "");
        assert_eq!(all.entries.len(), 6);
        assert_eq!(all.total, 100);
        let recent = shortcuts(&prefs, "最近", "");
        assert_eq!(recent.entries.len(), 6);
        assert_eq!(recent.total, 20);
        assert!(recent.entries.iter().all(|(_, kind)| *kind == "最近载入"));
        assert!(shortcuts(&prefs, "常用", "").entries.is_empty());
        assert!(shortcuts(&prefs, "服务", "").entries.is_empty());
        assert_eq!(
            shortcuts(&prefs, "收藏", "流程99").entries[0].0.name,
            "流程99"
        );
    }
}
