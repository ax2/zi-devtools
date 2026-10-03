use super::*;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ListSort {
    #[default]
    Default,
    Updated,
    Title,
}
impl ListSort {
    pub(super) fn label(self, calendar: bool) -> &'static str {
        match self {
            Self::Default if calendar => "时间顺序",
            Self::Default | Self::Updated => "最近更新",
            Self::Title => "标题顺序",
        }
    }
}
impl State {
    pub(super) fn listed_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        let mut rows: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                i.trash == self.trash
                    && i.schedule.is_some() == self.calendar
                    && (!self.list_pinned || i.pinned)
                    && (query.is_empty()
                        || format!("{} {}", i.title, i.body)
                            .to_lowercase()
                            .contains(&query))
                    && (!self.calendar
                        || self.trash
                        || !query.is_empty()
                        || i.schedule
                            .as_ref()
                            .is_some_and(|s| s.covering(self.selected).is_some()))
            })
            .map(|(index, _)| index)
            .collect();
        rows.sort_by(|&a, &b| {
            let a = &self.items[a];
            let b = &self.items[b];
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| match self.list_sort {
                    ListSort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                    ListSort::Default if self.calendar => a
                        .schedule
                        .as_ref()
                        .unwrap()
                        .start
                        .cmp(&b.schedule.as_ref().unwrap().start),
                    _ => b.updated.cmp(&a.updated),
                })
                .then_with(|| a.id.cmp(&b.id))
        });
        rows
    }
}

pub(super) fn record_row(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 54.0), egui::Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(12.0, 6.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 2.0;
    child.add(egui::Label::new(title).truncate());
    child.add(egui::Label::new(egui::RichText::new(detail).small().weak()).truncate());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    response.on_hover_text(format!("{title}\n{detail}"))
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_listing(&mut self, pinned: bool) {
        self.preview(false, false);
        self.items = (0..1500)
            .map(|n| {
                let mut i = Item::new(None);
                i.title = format!("{n:04} · 项目备忘与本地工具资料整理记录 · 待查看详情");
                i.body = format!("资料编号{n}，完整正文保持在本机。");
                i.updated = 1_790_899_200 + n;
                i.revision = 1;
                i.pinned = n % 500 == 0;
                i
            })
            .collect();
        self.draft = None;
        self.original = None;
        self.list_pinned = pinned;
    }
    pub fn preview_listing_smoke(&mut self, phase: u8) {
        match phase {
            0 => self.preview_listing(false),
            1 => {
                assert!(self.list_pinned);
                assert_eq!(self.listed_indices().len(), 3);
                assert!(self.items.iter().all(|i| i.revision == 1));
            }
            2 => {
                assert!(!self.list_pinned);
                assert!(self.list_sort == ListSort::Title);
                let rows = self.listed_indices();
                assert_eq!(rows.len(), 1500);
                assert_eq!(&rows[..3], &[0, 500, 1000]);
                assert!(self.preview_list_rows <= 20, "large list painted all rows");
                assert!(self.pending.is_none() && self.draft.is_none());
            }
            3 => {
                assert_eq!(self.draft.as_ref().unwrap().id, self.items[0].id);
                assert!(!self.has_unsaved() && self.pending.is_none());
            }
            4 => {
                assert!(
                    self.preview_list_start > 0 && self.preview_list_rows <= 20,
                    "actual scroll did not virtualize later rows"
                );
                assert_eq!(self.draft.as_ref().unwrap().id, self.items[0].id);
                println!(
                    "PASS listing UI: actual filter, title sort, exact record click and list scroll; 1500 retained, visible rows only, no writes"
                );
            }
            _ => unreachable!(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{event, fixture};
    #[test]
    fn list_filters_searches_and_orders_without_touching_unsaved_draft() {
        let mut s = State::new(fixture());
        s.pending = None;
        s.loaded = true;
        let mut a = Item::new(None);
        a.title = "Zulu".into();
        a.body = "First body".into();
        a.updated = 20;
        let mut b = a.clone();
        b.id = uuid::Uuid::new_v4().to_string();
        b.title = "alpha".into();
        b.updated = 30;
        let mut pinned = b.clone();
        pinned.id = uuid::Uuid::new_v4().to_string();
        pinned.title = "Pinned".into();
        pinned.updated = 1;
        pinned.pinned = true;
        let mut trash = a.clone();
        trash.id = uuid::Uuid::new_v4().to_string();
        trash.trash = true;
        s.items = vec![a, b, pinned, trash, event()];
        s.edit(s.items[0].clone());
        s.draft.as_mut().unwrap().body = "未保存".into();
        assert_eq!(s.listed_indices(), vec![2, 1, 0]);
        s.list_sort = ListSort::Title;
        assert_eq!(s.listed_indices(), vec![2, 1, 0]);
        s.query = "ZULU FIRST".into();
        assert_eq!(s.listed_indices(), vec![0]);
        s.query.clear();
        s.list_pinned = true;
        assert_eq!(s.listed_indices(), vec![2]);
        s.trash = true;
        assert!(s.listed_indices().is_empty());
        s.list_pinned = false;
        assert_eq!(s.listed_indices(), vec![3]);
        assert_eq!(s.draft.as_ref().unwrap().body, "未保存");
    }
    #[test]
    fn calendar_default_is_chronological_and_search_crosses_dates() {
        let mut s = State::new(fixture());
        s.pending = None;
        s.loaded = true;
        s.calendar = true;
        let mut later = event();
        later.schedule.as_mut().unwrap().start += Duration::hours(3);
        let earlier = event();
        let mut other = event();
        other.schedule.as_mut().unwrap().start += Duration::days(1);
        other.title = "找下周".into();
        s.selected = earlier.schedule.as_ref().unwrap().start.date();
        s.items = vec![later, earlier, other];
        assert_eq!(s.listed_indices(), vec![1, 0]);
        s.query = "下周".into();
        assert_eq!(s.listed_indices(), vec![2]);
    }
}
