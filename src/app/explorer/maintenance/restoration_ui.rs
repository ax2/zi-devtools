use super::*;

impl Maintenance {
    pub(super) fn restoration_ui(&mut self, ui: &mut egui::Ui, controlled: bool) {
        let busy = self.receiver.is_some();
        let mut submit = false;
        let mut cancel = false;
        if let Some(ticket) = &self.restoration {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong("恢复上次菜单登记");
                ui.add(egui::Label::new(ticket.operation_id()).wrap());
                ui.label("仅恢复这次维护之前的菜单登记，不替换安装文件、不更改Windows Installer产品登记，也不终止安装程序。");
                if let Some(previous) = ticket.previous() {
                    ui.label(format!("恢复MSI菜单登记版本：{}", previous.version));
                    path_label(ui, "已核验旧程序：", &previous.executable);
                } else { ui.label("取消首次安装产生的菜单登记；已核验程序文件不存在且没有对应MSI组件归属。"); }
                if let Some(active) = ticket.result().active() { path_label(ui, "恢复后的当前入口：", &active.executable); }
                else { ui.label("恢复后不启用右键入口。"); }
                ui.small("保留后来添加的便携登记和用户选择。确认时重新检查文件、安装器归属、维护记录和用户身份。");
                ui.colored_label(ui.visuals().warn_fg_color, "请先关闭安装程序，并通过安装器完成文件修复或回滚；此操作只恢复菜单登记。");
                if controlled || ticket.preview_only() { ui.small("受控界面预览 · 禁止实际恢复"); }
                ui.checkbox(&mut self.agreed, "安装程序已关闭，同意仅恢复菜单登记");
                ui.horizontal(|ui| {
                    submit = ui.add_enabled(self.agreed && !busy && !controlled && !ticket.preview_only(), egui::Button::new("确认恢复菜单")).clicked();
                    cancel = ui.add_enabled(!busy, egui::Button::new("返回维护记录")).clicked();
                });
            });
        }
        if cancel {
            self.restoration = None;
            self.agreed = false;
        }
        if submit {
            let ticket = *self
                .restoration
                .take()
                .expect("confirmed restoration ticket");
            let id = ticket.operation_id().to_owned();
            self.run(move || {
                let ledger = recovery::restoration::apply(ticket)?;
                Ok(Work::Completed {
                    ledger,
                    id,
                    status: Status::Reverted,
                    refreshed: recovery::list().map_err(|error| error.to_string()),
                })
            });
        }
    }
}
