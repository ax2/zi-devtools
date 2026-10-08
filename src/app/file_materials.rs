use super::*;
impl DevToolsApp {
    pub(super) fn file_material_dialog(&mut self, ctx: &egui::Context) {
        let Some(material) = self.file_state.file_transfer.as_ref() else {
            return;
        };
        let check = material.bytes() <= 8 * 1024 * 1024;
        let mut decision = None;
        egui::Modal::new(egui::Id::new("file-material-transfer")).show(ctx, |ui| {
            ui.heading("将原文件送到编码检查？");
            ui.label(material.path().display().to_string());
            ui.label(format!("{}字节。确认后替换编码检查的现有草稿，清除旧检测/转换预览；不会自动读取、转换或写入文件。", material.bytes()));
            ui.small("接收时重新核对原文件身份；文件变更会拒绝接收。编码检查最多8 MiB。");
            if !check { ui.colored_label(ui.visuals().error_fg_color, "原文件超过编码检查8 MiB上限"); }
            ui.horizontal(|ui| {
                if ui.button("返回").clicked() { decision = Some(false); }
                let confirm = ui.add_enabled(check, egui::Button::new("确认接收"));
                #[cfg(feature="ui-preview")]
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("file-material-confirm"), confirm.rect));
                if confirm.clicked() { decision = Some(true); }
            });
        });
        if let Some(confirmed) = decision {
            let material = self.file_state.file_transfer.take().unwrap();
            if confirmed {
                match self.file_encoding.receive_material(material) {
                    Ok(()) => {
                        self.page = Page::FileEncoding;
                        self.visit("encoding-detect");
                    }
                    Err(error) => self.toast = Some((error.to_string(), Instant::now())),
                }
            }
        }
    }
}

#[cfg(feature = "ui-preview")]
impl DevToolsApp {
    pub fn preview_file_material_prepare(
        &mut self,
        ctx: &egui::Context,
        light: bool,
        path: PathBuf,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Files;
        self.file_state.preview(path);
    }
    pub fn preview_file_material_check(&self, phase: usize) {
        if phase == 0 {
            assert_eq!(self.file_state.job.phase, crate::tasks::Phase::Done);
            let report: serde_json::Value = serde_json::from_str(self.file_state.report()).unwrap();
            assert_eq!(report["bytes"], 3);
            assert!(!self.file_state.report().contains("Users"));
            assert!(self.handoff_source().is_some());
        } else {
            assert!(self.page == Page::FileEncoding);
            self.file_encoding.preview_material_check(phase == 2);
            assert!(self.file_state.file_transfer.is_none());
        }
    }
}
