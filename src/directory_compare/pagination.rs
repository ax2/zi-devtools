//! Page windows shared by directory results and the reviewed copy plan.
use eframe::egui;
use std::ops::Range;

pub(super) const PAGE_SIZE: usize = 100;

pub(super) fn window(page: &mut usize, total: usize) -> Range<usize> {
    *page = (*page).min(total.saturating_sub(1) / PAGE_SIZE);
    let start = *page * PAGE_SIZE;
    start..start.saturating_add(PAGE_SIZE).min(total)
}

pub(super) fn controls(
    ui: &mut egui::Ui,
    page: &mut usize,
    total: usize,
    id: &str,
    label: &str,
) -> Range<usize> {
    window(page, total);
    let pages = total.div_ceil(PAGE_SIZE).max(1);
    if total > PAGE_SIZE {
        ui.push_id(id, |ui| {
            ui.horizontal_wrapped(|ui| {
                let previous = ui.add_enabled(*page > 0, egui::Button::new("上一页"));
                let next = ui.add_enabled(*page + 1 < pages, egui::Button::new("下一页"));
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(format!("{id}-previous")), previous.rect);
                    d.insert_temp(egui::Id::new(format!("{id}-next")), next.rect);
                });
                if previous.clicked() {
                    *page -= 1;
                }
                if next.clicked() {
                    *page += 1;
                }
                let mut number = *page + 1;
                ui.label(format!("{label} · 第"));
                if ui
                    .add(egui::DragValue::new(&mut number).range(1..=pages).speed(1))
                    .changed()
                {
                    *page = number.saturating_sub(1).min(pages - 1);
                }
                ui.label(format!("/ {pages} 页"));
            });
        });
    }
    let range = window(page, total);
    if total > 0 {
        ui.weak(format!(
            "{label}：第 {}–{} 项 / 共 {total} 项",
            range.start + 1,
            range.end
        ));
    }
    #[cfg(feature = "ui-preview")]
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::new(format!("{id}-range")),
            (range.start, range.end, total),
        );
    });
    range
}

#[cfg(feature = "ui-preview")]
impl super::State {
    pub(crate) fn preview_pagination_check(&self, step: u8) {
        let (page, selected) = match step {
            0 => (0, 0),
            1 => (2, 105),
            2 => (1, 105),
            3 => (0, 105),
            _ => panic!("unknown pagination step"),
        };
        assert_eq!(self.result_page, page);
        assert_eq!(self.selected.len(), selected);
        assert!(!self.background_active());
        assert!(self.copy_result.is_none());
        if step >= 2 {
            let plan = self.copy_plan.as_ref().unwrap();
            assert_eq!(plan.items.len(), 105);
            assert_eq!(
                plan.items.first().unwrap().relative,
                std::path::Path::new("group-0100/file-0100.txt")
            );
            assert_eq!(
                plan.items.last().unwrap().relative,
                std::path::Path::new("group-0204/file-0204.txt")
            );
            assert_eq!(plan.new_directories.len(), 105);
            assert_eq!(self.copy_preview_page, 1);
            assert_eq!(self.directory_preview_page, 1);
        } else {
            assert!(self.copy_plan.is_none());
        }
        if step == 3 {
            assert_eq!(self.filter, "file-0204.txt");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_including_the_last_partial_page_is_reachable_once() {
        for total in [0usize, 1, 99, 100, 101, 205, 1000] {
            let mut visited = Vec::new();
            for mut page in 0..total.div_ceil(PAGE_SIZE).max(1) {
                let range = window(&mut page, total);
                assert!(range.len() <= PAGE_SIZE);
                visited.extend(range);
            }
            assert_eq!(visited, (0..total).collect::<Vec<_>>());
        }
    }

    #[test]
    fn shrinking_results_and_empty_filters_cannot_leave_an_unreachable_page() {
        let mut page = 9;
        assert_eq!(window(&mut page, 205), 200..205);
        assert_eq!(page, 2);
        assert_eq!(window(&mut page, 101), 100..101);
        assert_eq!(page, 1);
        assert_eq!(window(&mut page, 0), 0..0);
        assert_eq!(page, 0);
        page = usize::MAX;
        let range = window(&mut page, usize::MAX);
        assert_eq!(range.end, usize::MAX);
    }
}
