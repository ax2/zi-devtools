use super::*;
impl Prefix {
    pub(in crate::app) fn has_work(&self, saved: &crate::commands::Bindings) -> bool {
        self.files.pending() || self.draft.as_ref().is_some_and(|draft| draft != saved)
    }
}
impl DevToolsApp {
    pub(super) fn binding_profiles(&mut self, ui: &mut egui::Ui, defaults: &[Command]) -> bool {
        if self.prefix.files.busy() {
            ui.spinner();
            ui.label("快捷配置文件处理中…");
            return true;
        }
        if let Some(imported) = self.prefix.files.review.clone() {
            ui.heading("查看快捷配置");
            ui.small("只包含自定义附加键，不修改系统前缀。确认后放入草稿，保存成功才生效。");
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut self.prefix.merge, true, "合并：保留其他自定义键");
                let replace =
                    ui.radio_value(&mut self.prefix.merge, false, "替换：未列出的工具恢复默认");
                #[cfg(feature = "ui-preview")]
                {
                    self.prefix.profile_rects[2] = replace.rect;
                }
                #[cfg(not(feature = "ui-preview"))]
                let _ = replace;
            });
            let current = self.prefix.draft.as_ref().unwrap();
            let candidate =
                crate::commands::profile::candidate(current, &imported, self.prefix.merge);
            let (_, errors) = crate::commands::configured(defaults, &candidate);
            let ids: std::collections::BTreeSet<_> =
                current.keys().chain(candidate.keys()).cloned().collect();
            let mut changes = Vec::new();
            for id in ids {
                let default = defaults.iter().find(|command| command.id == id);
                let default_sequence = default.map(|c| c.sequence.as_str()).unwrap_or("");
                let old = current
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or(default_sequence);
                let new = candidate
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or(default_sequence);
                if old != new {
                    changes.push((
                        default
                            .map(|c| c.title.clone())
                            .unwrap_or_else(|| format!("暂不可用 · {id}")),
                        old.to_owned(),
                        new.to_owned(),
                    ));
                }
            }
            ui.label(format!(
                "文件 {} 项自定义设置；导入后 {} 项；{} 项序列变化",
                imported.len(),
                candidate.len(),
                changes.len()
            ));
            egui::ScrollArea::vertical()
                .id_salt("profile-changes")
                .max_height(180.0)
                .show(ui, |ui| {
                    if changes.is_empty() {
                        ui.label("当前可见序列没有变化；覆盖记录仍按所选方式处理。");
                    }
                    for (title, old, new) in changes {
                        ui.label(format!(
                            "{title}：{} → {}",
                            if old.is_empty() { "未绑定" } else { &old },
                            if new.is_empty() { "未绑定" } else { &new }
                        ));
                    }
                });
            for error in errors.iter().take(4) {
                ui.colored_label(self.colors.amber, error);
            }
            if !errors.is_empty() {
                ui.label("导入后需在草稿中修正冲突；未保存前不会改变当前绑定。");
            }
            ui.horizontal_wrapped(|ui| {
                let confirm =
                    ui.add_enabled(candidate.len() <= 4096, egui::Button::new("放入草稿"));
                #[cfg(feature = "ui-preview")]
                {
                    self.prefix.profile_rects[0] = confirm.rect;
                }
                if confirm.clicked() {
                    self.prefix.draft = Some(candidate);
                    self.prefix.files.review = None;
                    self.prefix.selected.clear();
                    self.prefix.files.message = "配置已放入草稿；尚未生效，请检查并保存".into();
                }
                let cancel = ui.button("取消导入");
                #[cfg(feature = "ui-preview")]
                {
                    self.prefix.profile_rects[1] = cancel.rect;
                }
                if cancel.clicked() {
                    self.prefix.files.review = None;
                    self.prefix.files.message = "已取消导入，原草稿和当前绑定保留".into();
                }
            });
            return true;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("导入快捷配置…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("选择快捷配置，预览后导入")
                    .add_filter("快捷配置 JSON", &["json"])
                    .pick_file()
                && let Err(error) = self.prefix.files.read(path)
            {
                self.prefix.files.message = format!("{error:#}");
            }
            let draft = self.prefix.draft.as_ref().unwrap();
            let valid = crate::commands::configured(defaults, draft).1.is_empty()
                && crate::commands::profile::encode(draft).is_ok();
            if ui
                .add_enabled(
                    valid && !self.prefix.files.pending(),
                    egui::Button::new("另存草稿配置…"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("另存新的快捷配置文件")
                    .add_filter("快捷配置 JSON", &["json"])
                    .set_file_name("ZiDevTools-快捷配置.json")
                    .save_file()
                && let Err(error) = self.prefix.files.save(draft.clone(), path)
            {
                self.prefix.files.message = format!("{error:#}");
            }
        });
        if !self.prefix.files.message.is_empty() {
            ui.label(&self.prefix.files.message);
        }
        self.prefix.files.busy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_drafts_and_import_reviews_are_protected_on_exit() {
        let saved = crate::commands::Bindings::new();
        let mut prefix = Prefix::default();
        assert!(!prefix.has_work(&saved));
        prefix.draft = Some(saved.clone());
        assert!(!prefix.has_work(&saved));
        prefix.files.review = Some(saved.clone());
        assert!(prefix.has_work(&saved));
        prefix.files.review = None;
        prefix
            .draft
            .as_mut()
            .unwrap()
            .insert("open:json".into(), "Q J".into());
        assert!(prefix.has_work(&saved));
        let saved = prefix.draft.clone().unwrap();
        assert!(!prefix.has_work(&saved));
    }
}
