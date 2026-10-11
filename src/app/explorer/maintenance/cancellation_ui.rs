use super::*;

impl Maintenance {
    pub(super) fn cancellation_ui(&mut self, ui: &mut egui::Ui, controlled: bool) {
        let busy = self.receiver.is_some();
        let mut submit = false;
        let mut cancel = false;
        if let Some(ticket) = &self.cancellation {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong("取消尚未执行的菜单维护");
                ui.add(egui::Label::new(ticket.operation_id()).wrap());
                ui.label("只取消这条未执行的菜单维护；不改菜单、程序登记、安装文件或用户数据，也不会撤销或终止安装程序。");
                ui.colored_label(ui.visuals().warn_fg_color, "请先关闭安装程序。如果安装仍在运行，后续菜单动作会被拒绝，可能导致安装失败。");
                ui.small("确认时重新核对用户、维护记录和安装登记；已经执行或状态变化时拒绝取消。");
                if controlled || ticket.preview_only() { ui.small("受控界面预览 · 禁止实际取消"); }
                ui.checkbox(&mut self.agreed, "安装程序已关闭，同意取消这条未执行的菜单维护");
                ui.horizontal(|ui| {
                    submit = ui.add_enabled(self.agreed && !busy && !controlled && !ticket.preview_only(), egui::Button::new("确认取消菜单维护")).clicked();
                    cancel = ui.add_enabled(!busy, egui::Button::new("返回维护记录")).clicked();
                });
            });
        }
        if cancel {
            self.cancellation = None;
            self.agreed = false;
        }
        if submit {
            let ticket = *self
                .cancellation
                .take()
                .expect("confirmed cancellation ticket");
            let id = ticket.operation_id().to_owned();
            self.run(move || {
                let ledger = recovery::cancellation::apply(ticket)?;
                Ok(Work::Completed {
                    ledger,
                    id,
                    status: Status::Canceled,
                    refreshed: recovery::list().map_err(|error| error.to_string()),
                })
            });
        }
    }
}
