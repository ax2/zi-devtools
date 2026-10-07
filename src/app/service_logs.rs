use eframe::egui;
use std::ops::Range;

/// Indexes a bounded log snapshot; filtering is rebuilt only when inputs change.
#[derive(Default)]
pub(super) struct LogLines {
    lines: Vec<Range<usize>>,
    matches: Vec<usize>,
    query: String,
    dirty: bool,
}

impl LogLines {
    pub(super) fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub(super) fn update(&mut self, text: &str, query: &str) {
        let query = query.trim().to_lowercase();
        if !self.dirty && self.query == query {
            return;
        }
        if self.dirty {
            self.lines.clear();
            let mut start = 0;
            for line in text.split_inclusive('\n') {
                let body = line
                    .strip_suffix('\n')
                    .map_or(line, |body| body.strip_suffix('\r').unwrap_or(body));
                let end = start + body.len();
                self.lines.push(start..end);
                start += line.len();
            }
        }
        self.matches = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, range)| {
                query.is_empty() || text[(*range).clone()].to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.query = query;
        self.dirty = false;
    }

    pub(super) fn total(&self) -> usize {
        self.lines.len()
    }

    pub(super) fn count(&self) -> usize {
        self.matches.len()
    }

    pub(super) fn line<'a>(&self, text: &'a str, index: usize) -> &'a str {
        &text[self.lines[self.matches[index]].clone()]
    }

    pub(super) fn copy_matches(&self, text: &str) -> String {
        (0..self.count())
            .map(|index| self.line(text, index))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn display_line(line: &str) -> (&str, bool) {
    const LIMIT: usize = 16_384;
    let mut end = line.len().min(LIMIT);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    (&line[..end], end != line.len())
}

pub(super) fn show(ui: &mut egui::Ui, lines: &LogLines, text: &str, follow: bool) -> usize {
    let height = ui
        .text_style_height(&egui::TextStyle::Monospace)
        .max(ui.spacing().interact_size.y);
    let mut painted = 0;
    egui::ScrollArea::both()
        .id_salt("service-log-lines")
        .stick_to_bottom(follow)
        .max_height(480.0)
        .auto_shrink([false, false])
        .show_rows(ui, height, lines.count(), |ui, range| {
            for index in range {
                let (line, shortened) = display_line(lines.line(text, index));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(if line.is_empty() { " " } else { line })
                                .monospace(),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(true),
                    );
                    if shortened {
                        ui.label("…［超长行仅预览前 16 KiB，复制/打开文件查看完整内容］");
                    }
                });
                painted += 1;
            }
        });
    painted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_unicode_empty_lines_and_rebuilds_after_snapshot_change() {
        let mut lines = LogLines::default();
        lines.invalidate();
        let text = "INFO 开始\r\n\r\nWARN Ошибка\nwarn second\n";
        lines.update(text, " WARN ");
        assert_eq!(lines.total(), 4);
        assert_eq!(lines.copy_matches(text), "WARN Ошибка\nwarn second");
        lines.update(text, "ошибка");
        assert_eq!(lines.copy_matches(text), "WARN Ошибка");
        lines.update(text, "");
        assert_eq!(lines.line(text, 1), "");
        lines.invalidate();
        lines.update("next\n", "");
        assert_eq!(lines.total(), 1);
        assert_eq!(lines.copy_matches("next\n"), "next");
        lines.invalidate();
        lines.update("", "");
        assert_eq!(lines.count(), 0);
    }

    #[test]
    fn large_log_paints_only_visible_rows_and_preserves_long_line_copy() {
        let text = (0..5000).map(|i| format!("line {i}\n")).collect::<String>();
        let mut lines = LogLines::default();
        lines.invalidate();
        lines.update(&text, "");
        let ctx = egui::Context::default();
        let mut painted = 0;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 240.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    painted = show(ui, &lines, &text, false);
                });
            },
        );
        assert!(painted > 0 && painted < 30, "painted {painted} / 5000");
        let long = "界".repeat(20_000);
        let (preview, shortened) = display_line(&long);
        assert!(shortened && preview.len() <= 16_384 && preview.is_char_boundary(preview.len()));
        lines.invalidate();
        lines.update(&long, "");
        assert_eq!(lines.copy_matches(&long), long);
    }
}
