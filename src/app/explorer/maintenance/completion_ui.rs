use super::*;

impl Maintenance {
    pub(super) fn completion_ui(&mut self, ui: &mut egui::Ui, controlled: bool) {
        let busy = self.receiver.is_some();
        let mut submit = false;
        let mut cancel = false;
        if let Some(ticket) = &self.completion {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong("结束已提交安装的维护");
                ui.add(egui::Label::new(ticket.operation_id()).wrap());
                ui.label("安装事务已结束，仅释放这次维护占用；当前菜单、程序登记、安装文件和用户数据保留。");
                if let Some(active) = ticket.result().active() { path_label(ui, "保留当前入口：", &active.executable); }
                ui.small(format!("保留{}份安装登记。预览后的记录或用户身份变化时，需要重新预览。", ticket.result().entries().len()));
                if controlled || ticket.preview_only() { ui.small("受控界面预览 · 禁止实际结束维护"); }
                ui.checkbox(&mut self.agreed, "同意结束这次维护，保留当前入口");
                ui.horizontal(|ui| {
                    submit = ui.add_enabled(self.agreed && !busy && !controlled && !ticket.preview_only(), egui::Button::new("确认结束维护")).clicked();
                    cancel = ui.add_enabled(!busy, egui::Button::new("取消恢复预览")).clicked();
                });
            });
        }
        if cancel {
            self.completion = None;
            self.agreed = false;
        }
        if submit {
            let ticket = *self.completion.take().expect("confirmed completion ticket");
            let id = ticket.operation_id().to_owned();
            self.run(move || {
                let ledger = recovery::completion::apply(ticket)?;
                Ok(Work::Completed {
                    ledger,
                    id,
                    refreshed: recovery::list().map_err(|error| error.to_string()),
                })
            });
        }
    }
}
