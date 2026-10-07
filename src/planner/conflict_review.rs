//! Read-only comparison of the raw draft and the latest shared snapshot.
use super::*;

struct Side {
    title: String,
    body: String,
    revision: i64,
    fields: Vec<(&'static str, String)>,
}
struct Comparison {
    local: Side,
    remote: Option<Side>,
    changed: Vec<&'static str>,
}
fn flag(value: bool) -> String {
    if value { "是" } else { "否" }.into()
}
fn side(item: &Item) -> Side {
    let mut fields = vec![
        (
            "类型",
            if item.schedule.is_some() {
                "日程"
            } else {
                "备忘录"
            }
            .into(),
        ),
        ("置顶", flag(item.pinned)),
        ("回收站", flag(item.trash)),
    ];
    if let Some(s) = &item.schedule {
        fields.extend([
            ("开始日期", s.start.date().to_string()),
            (
                "开始 / 提醒时间",
                s.reminder_at(s.start).format("%H:%M").to_string(),
            ),
            (
                "结束日期",
                s.display_end()
                    .map(|end| end.date().to_string())
                    .unwrap_or_else(|| "未设置".into()),
            ),
            (
                "结束时间",
                s.display_end()
                    .map(|end| end.format("%H:%M").to_string())
                    .unwrap_or_else(|| "未设置".into()),
            ),
            ("全天", flag(s.all_day)),
            ("重复", s.repeat.label().into()),
            (
                "重复截止",
                s.repeat_until
                    .map(|day| day.to_string())
                    .unwrap_or_default(),
            ),
            ("缺日改到月底", flag(s.clamp_missing_day)),
            ("提醒", flag(s.remind)),
            ("提前分钟", s.minutes.to_string()),
            ("完成", flag(s.done)),
            (
                "已知晓",
                s.handled
                    .map(|at| at.to_string())
                    .unwrap_or_else(|| "无".into()),
            ),
            (
                "稍后提醒",
                s.snooze
                    .map(|(at, until)| {
                        format!(
                            "{at} → {}",
                            DateTime::from_timestamp(until, 0)
                                .map(|t| t.with_timezone(&Local).to_string())
                                .unwrap_or_else(|| "无效时间".into())
                        )
                    })
                    .unwrap_or_else(|| "无".into()),
            ),
            (
                "日历 UID",
                item.calendar_uid.clone().unwrap_or_else(|| "无".into()),
            ),
        ]);
    }
    Side {
        title: item.title.clone(),
        body: item.body.clone(),
        revision: item.revision,
        fields,
    }
}
impl State {
    pub fn conflict_review_open(&self) -> bool {
        self.conflict_review.is_some()
    }
    fn conflict_comparison(&self) -> Option<Comparison> {
        let draft = self.draft.as_ref()?;
        if self.conflict_review.as_deref() != Some(draft.id.as_str()) || !self.shared_conflict() {
            return None;
        }
        let mut local = side(draft);
        if let Some(s) = &draft.schedule {
            for (key, value) in &mut local.fields {
                match *key {
                    "开始日期" => *value = self.date_text.clone(),
                    "开始 / 提醒时间" => *value = self.time_text.clone(),
                    "结束日期" if s.end.is_some() => *value = self.end_date_text.clone(),
                    "结束时间" if s.end.is_some() && !s.all_day => {
                        *value = self.end_time_text.clone()
                    }
                    "重复截止" => *value = self.repeat_until_text.clone(),
                    _ => (),
                }
            }
        }
        let remote = self.items.iter().find(|item| item.id == draft.id).map(side);
        let mut changed = Vec::new();
        if let Some(other) = &remote {
            if local.title != other.title {
                changed.push("标题");
            }
            if local.body != other.body {
                changed.push("正文");
            }
            for (key, value) in &local.fields {
                if other
                    .fields
                    .iter()
                    .find(|(k, _)| k == key)
                    .is_none_or(|(_, v)| v != value)
                {
                    changed.push(*key);
                }
            }
            for (key, _) in &other.fields {
                if !local.fields.iter().any(|(k, _)| k == key) {
                    changed.push(*key);
                }
            }
        } else {
            changed.extend(["标题", "正文"]);
            changed.extend(local.fields.iter().map(|(key, _)| *key));
        }
        Some(Comparison {
            local,
            remote,
            changed,
        })
    }
    pub(super) fn conflict_review_ui(&mut self, ctx: &egui::Context) {
        if self.conflict_review.is_none() {
            return;
        }
        let Some(comparison) = self.conflict_comparison() else {
            self.conflict_review = None;
            return;
        };
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("planner-conflict-review"))
            .frame(egui::Frame::popup(&ctx.style()).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| {
                let width = (ctx.screen_rect().width() - 48.0).clamp(180.0, 860.0);
                ui.set_width(width);
                ui.heading("对比冲突内容");
                ui.label("右侧是最近自动同步的版本，可能继续变化；此窗口只查看，不保存或合并。");
                if !self.shared.notice.is_empty() {
                    ui.colored_label(ui.visuals().warn_fg_color, &self.shared.notice);
                    ui.small("同步异常期间，另一窗口版本可能不是最新内容。");
                }
                if comparison.remote.is_some() {
                    ui.label(if comparison.changed.is_empty() {
                        "字段内容相同，但保存版本已改变。".into()
                    } else {
                        format!("不同字段：{}", comparison.changed.join(" · "))
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("conflict-comparison-content")
                    .max_height((ctx.screen_rect().height() - 250.0).clamp(100.0, 440.0))
                    .show(ui, |ui| {
                        if width >= 650.0 {
                            ui.columns(2, |cols| {
                                comparison_side(
                                    &mut cols[0],
                                    "当前修改",
                                    &comparison.local,
                                    &comparison.changed,
                                );
                                remote_side(&mut cols[1], &comparison);
                            });
                        } else {
                            comparison_side(ui, "当前修改", &comparison.local, &comparison.changed);
                            ui.add_space(12.0);
                            remote_side(ui, &comparison);
                        }
                    });
                ui.separator();
                ui.small("返回编辑后，可将修改另存为新记录来保留双方；日期无效时请先修正。");
                let button = ui.button("返回编辑");
                close = button.clicked();
                #[cfg(feature = "ui-preview")]
                {
                    self.preview_conflict_review_rects[1] = Some((button.rect, ui.clip_rect()));
                }
            });
        if close || response.should_close() {
            self.conflict_review = None;
        }
    }
}
fn remote_side(ui: &mut egui::Ui, comparison: &Comparison) {
    if let Some(remote) = &comparison.remote {
        comparison_side(ui, "另一窗口版本", remote, &comparison.changed);
    } else {
        ui.group(|ui| {
            ui.heading("另一窗口版本");
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "最近同步结果中，此记录已被删除。",
            );
            ui.label("当前未保存修改仍保留。返回编辑后可以另存为新记录。");
        });
    }
}
fn comparison_side(ui: &mut egui::Ui, label: &str, side: &Side, changed: &[&str]) {
    ui.push_id(label, |ui| {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading(label);
            ui.small(format!(
                "{}版本 {}",
                if label == "当前修改" {
                    "基于保存"
                } else {
                    "最近同步"
                },
                side.revision
            ));
            field_label(ui, "标题", changed);
            ui.add(egui::Label::new(&side.title).selectable(true).wrap());
            field_label(ui, "正文", changed);
            let mut body = side.body.as_str();
            egui::ScrollArea::vertical()
                .id_salt("readonly-body")
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut body)
                            .desired_width(f32::INFINITY)
                            .desired_rows(5),
                    );
                });
            ui.separator();
            for (key, value) in side.fields.iter().filter(|(key, _)| changed.contains(key)) {
                field_label(ui, key, changed);
                ui.add(
                    egui::Label::new(if value.is_empty() { "（空）" } else { value })
                        .selectable(true)
                        .wrap(),
                );
            }
            let unchanged: Vec<_> = side
                .fields
                .iter()
                .filter(|(key, _)| !changed.contains(key))
                .collect();
            if !unchanged.is_empty() {
                ui.collapsing(
                    format!("其他属性（{} 项相同）", unchanged.len()),
                    |ui| {
                        for (key, value) in unchanged {
                            field_label(ui, key, changed);
                            ui.add(
                                egui::Label::new(if value.is_empty() {
                                    "（空）"
                                } else {
                                    value
                                })
                                .selectable(true)
                                .wrap(),
                            );
                        }
                    },
                );
            }
        });
    });
}
fn field_label(ui: &mut egui::Ui, key: &str, changed: &[&str]) {
    let text = egui::RichText::new(key).strong();
    ui.label(if changed.contains(&key) {
        text.color(ui.visuals().warn_fg_color)
    } else {
        text
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{event, fixture, wait_state};

    #[test]
    fn raw_invalid_fields_and_modal_shortcut_never_mutate_either_record() {
        let path = fixture();
        store::save(&path, event()).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(state.items[0].clone());
        state.date_text = "无效日期".into();
        state.time_text = "25:99".into();
        state.repeat_until_text = "2031-错误".into();
        state.draft.as_mut().unwrap().body = "保留未保存正文".into();
        let mut remote = state.items[0].clone();
        remote.body = "外部已保存正文".into();
        store::save(&path, remote).unwrap();
        wait_state(&mut state);
        let draft = state.draft.clone();
        state.editor_action = Some((
            draft.as_ref().unwrap().id.clone(),
            actions::Action::CompareConflict,
        ));
        state.finish_editor_actions(false);
        assert!(state.conflict_review_open());
        let comparison = state.conflict_comparison().unwrap();
        for (key, expected) in [
            ("开始日期", "无效日期"),
            ("开始 / 提醒时间", "25:99"),
            ("重复截止", "2031-错误"),
        ] {
            assert_eq!(
                comparison
                    .local
                    .fields
                    .iter()
                    .find(|(k, _)| *k == key)
                    .unwrap()
                    .1,
                expected
            );
            assert!(comparison.changed.contains(&key));
        }
        assert_eq!(comparison.remote.unwrap().body, "外部已保存正文");
        assert_eq!(state.draft, draft);
        state.finish_editor_actions(true);
        assert!(state.pending.is_none());
        assert!(state.save_conflict_copy().is_err());
        assert_eq!(store::load(&path).unwrap().len(), 1);
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            state.conflict_review_ui(ctx)
        });
        assert_eq!(state.draft, draft);
        assert_eq!(state.time_text, "25:99");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn open_comparison_follows_new_shared_revisions_and_deletion_without_discarding() {
        let path = fixture();
        let mut memo = Item::new(None);
        memo.title = "原始记录".into();
        store::save(&path, memo).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(state.items[0].clone());
        state.draft.as_mut().unwrap().title = "本地未保存标题".into();
        let id = state.draft.as_ref().unwrap().id.clone();
        let mut remote = state.items[0].clone();
        remote.title = "外部版本二".into();
        store::save(&path, remote).unwrap();
        wait_state(&mut state);
        state.conflict_review = Some(id.clone());
        assert_eq!(
            state
                .conflict_comparison()
                .unwrap()
                .remote
                .unwrap()
                .revision,
            2
        );
        let mut remote = state.items[0].clone();
        remote.title = "外部版本三".into();
        store::save(&path, remote).unwrap();
        wait_state(&mut state);
        assert_eq!(
            state.conflict_comparison().unwrap().remote.unwrap().title,
            "外部版本三"
        );
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("DELETE FROM records WHERE id=?1", [&id])
            .unwrap();
        drop(conn);
        wait_state(&mut state);
        assert!(state.conflict_comparison().unwrap().remote.is_none());
        assert_eq!(state.draft.as_ref().unwrap().title, "本地未保存标题");
        assert_eq!(state.draft.as_ref().unwrap().revision, 1);
        state.conflict_review = None;
        state.save_conflict_copy().unwrap();
        wait_state(&mut state);
        assert_eq!(store::load(&path).unwrap().len(), 1);
        assert!(!state.has_unsaved());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn switching_record_clears_comparison_and_equal_fields_do_not_fake_a_text_diff() {
        let path = fixture();
        let mut memo = Item::new(None);
        memo.title = "相同文本".into();
        store::save(&path, memo).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(state.items[0].clone());
        state.draft.as_mut().unwrap().body = "双方相同内容".into();
        let mut remote = state.items[0].clone();
        remote.body = "双方相同内容".into();
        store::save(&path, remote).unwrap();
        wait_state(&mut state);
        state.conflict_review = Some(state.draft.as_ref().unwrap().id.clone());
        assert!(state.conflict_comparison().unwrap().changed.is_empty());
        state.edit(state.items[0].clone());
        assert!(!state.conflict_review_open());
        assert!(!state.has_unsaved());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn failed_shared_read_keeps_comparison_and_raw_draft_with_a_visible_stale_warning() {
        let path = fixture();
        let mut memo = Item::new(None);
        memo.title = "同步错误夹具".into();
        store::save(&path, memo).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(state.items[0].clone());
        state.draft.as_mut().unwrap().body = "未保存的正文".into();
        let mut remote = state.items[0].clone();
        remote.body = "外部最后成功同步内容".into();
        store::save(&path, remote).unwrap();
        wait_state(&mut state);
        state.conflict_review = Some(state.draft.as_ref().unwrap().id.clone());
        std::fs::write(&path, b"corrupt disposable sqlite fixture").unwrap();
        wait_state(&mut state);
        assert!(!state.shared.notice.is_empty());
        assert_eq!(
            state.conflict_comparison().unwrap().remote.unwrap().body,
            "外部最后成功同步内容"
        );
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            state.conflict_review_ui(ctx)
        });
        assert!(state.conflict_review_open());
        assert_eq!(state.draft.as_ref().unwrap().body, "未保存的正文");
        assert!(state.pending.is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
