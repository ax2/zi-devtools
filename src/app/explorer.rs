use super::*;
use anyhow::Context;

impl DevToolsApp {
    pub(super) fn explorer_preview(&self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let expanded;
        #[cfg(feature = "ui-preview")]
        {
            expanded = ctx.data(|data| {
                data.get_temp::<bool>(egui::Id::new("preview-explorer-only"))
                    .unwrap_or(false)
            });
        }
        #[cfg(not(feature = "ui-preview"))]
        {
            expanded = false;
        }
        egui::Frame::new().fill(self.colors.card).corner_radius(12.0).inner_margin(18.0).show(ui, |ui| {
            ui.strong("资源管理器右键入口");
            ui.label("开发中 · 先查看统一入口和可接收的文件动作");
            ui.small("当前只生成注册计划，不查询已安装状态，不改动系统。单选经典菜单的候选计划；完整多选、目录、窗口转发及安装维护仍待完成。");
            egui::CollapsingHeader::new("查看候选菜单与注册明细").default_open(expanded).show(ui, |ui| {
                let plan = std::env::current_exe().context("无法定位当前程序").and_then(|exe| crate::explorer::plan(&exe, &self.manager.config_snapshot().path));
                match plan {
                    Ok(plan) => {
                        ui.label("Zi DevTools › 选择工具处理…");
                        ui.add_space(6.0);
                        for target in crate::intake::Target::ALL {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(target.label());
                                ui.small(if target == crate::intake::Target::Files { "外部接收最多64个文件；当前菜单计划仅单选" } else { "一次一个UTF-8文件 · 最多2 MiB" });
                            });
                        }
                        ui.add_space(10.0);
                        ui.small("选择动作只预选接收工具，仍需在导入页确认，不自动处理文件。明细含本机程序及配置位置，请按需复制。");
                        let mut json = serde_json::to_string_pretty(&plan).expect("registration plan serialization");
                        if ui.button("复制注册预览（JSON）").clicked() { ctx.copy_text(json.clone()); }
                        egui::ScrollArea::vertical().id_salt("explorer-plan-json").max_height(240.0).show(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut json).code_editor().interactive(false).desired_width(f32::INFINITY));
                        });
                    }
                    Err(error) => { ui.colored_label(self.colors.red, error.to_string()); }
                }
            });
        });
    }
}
