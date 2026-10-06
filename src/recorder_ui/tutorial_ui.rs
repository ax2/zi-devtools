use super::*;
use crate::recorder::tutorial::{Compositor, Corner, Pointer, Settings, ZoomMode};

pub(super) struct Preview {
    source: Vec<u8>,
    output: Vec<u8>,
    size: [u32; 2],
    compositor: Compositor,
    texture: Option<egui::TextureHandle>,
    started: Instant,
    last: Option<Instant>,
    label: String,
    snapshot_region: Option<(String, Region)>,
    #[cfg(feature = "ui-preview")]
    fixture_pointer: Option<Pointer>,
}
impl Default for Preview {
    fn default() -> Self {
        let size = [640, 240];
        let source = (0..size[1])
            .flat_map(|y| {
                (0..size[0]).flat_map(move |x| {
                    if x % 80 < 2 || y % 60 < 2 {
                        [220, 220, 220, 255]
                    } else {
                        [
                            if x < 320 { 70 } else { 160 },
                            if y < 120 { 150 } else { 75 },
                            if x < 320 { 35 } else { 210 },
                            255,
                        ]
                    }
                })
            })
            .collect();
        Self {
            source,
            output: vec![],
            size,
            compositor: Compositor::default(),
            texture: None,
            started: Instant::now(),
            last: None,
            label: "合成网格示例；在预览上移动和点击试用效果".into(),
            snapshot_region: None,
            #[cfg(feature = "ui-preview")]
            fixture_pointer: None,
        }
    }
}
impl RecorderState {
    fn set_tutorial(&mut self, settings: Settings) {
        if self.countdown_deadline.is_some() {
            return;
        }
        if settings.active()
            && self
                .region
                .is_some_and(|r| u64::from(r.width) * u64::from(r.height) > 16_000_000)
        {
            self.status = "教程效果支持最多1600万像素；请缩小区域".into();
            self.error = true;
            return;
        }
        if let Some(session) = &self.session
            && let Err(error) = session.tutorial.set(settings)
        {
            self.status = format!("教程效果未修改：{error}");
            self.error = true;
            return;
        }
        self.tutorial = settings;
        self.tutorial_preview.last = None;
    }
    pub fn toggle_tutorial_zoom(&mut self) {
        let mut settings = self.tutorial;
        settings.mode = if settings.mode == ZoomMode::Off {
            ZoomMode::Follow
        } else {
            ZoomMode::Off
        };
        self.set_tutorial(settings);
    }
    pub fn disable_tutorial_effects(&mut self) {
        self.set_tutorial(Settings::default());
    }
    pub fn show_tutorial(&mut self) {
        self.tutorial_open_requested = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_tutorial_fixture(&mut self, scroll: bool) {
        self.preview_fixture();
        self.entry_id = "recorder-tutorial".into();
        self.preview_tutorial_scroll = scroll;
        self.tutorial = Settings {
            mode: ZoomMode::Fixed,
            scale: 2.5,
            focus: [0.65, 0.45],
            highlight: true,
            clicks: true,
            ..Settings::default()
        };
    }
    pub(super) fn tutorial_ui(&mut self, ui: &mut egui::Ui) {
        if std::mem::take(&mut self.tutorial_open_requested) {
            let id = ui.make_persistent_id("recorder-tutorial");
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                true,
            );
            state.set_open(true);
            state.store(ui.ctx());
        }
        egui::CollapsingHeader::new("教程辅助 · 效果写入视频")
            .id_salt("recorder-tutorial").default_open(self.tutorial.active()).show(ui, |ui| {
            let before = self.tutorial;
            let allowed = self.region.is_none_or(|r| u64::from(r.width)*u64::from(r.height)<=16_000_000);
            ui.add_enabled_ui(self.countdown_deadline.is_none(), |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("tutorial-mode").selected_text(self.tutorial.mode.label()).show_ui(ui, |ui| {
                        for mode in [ZoomMode::Off,ZoomMode::Fixed,ZoomMode::Follow,ZoomMode::Inset] {
                            ui.selectable_value(&mut self.tutorial.mode, mode, mode.label());
                        }
                    });
                    if ui.button("复位画面").clicked() { self.tutorial.mode = ZoomMode::Off; }
                    if ui.button("关闭所有效果").clicked() { self.tutorial = Settings::default(); }
                });
                if self.tutorial.mode != ZoomMode::Off {
                    ui.add(egui::Slider::new(&mut self.tutorial.scale,1.0..=4.0).suffix(" 倍").text("放大"));
                    ui.checkbox(&mut self.tutorial.smooth,"平滑过渡（约180ms）");
                    if self.tutorial.mode == ZoomMode::Fixed {
                        ui.horizontal(|ui| {
                            ui.label("焦点位置");
                            ui.add(egui::Slider::new(&mut self.tutorial.focus[0],0.0..=1.0).text("横向"));
                            ui.add(egui::Slider::new(&mut self.tutorial.focus[1],0.0..=1.0).text("纵向"));
                        });
                    }
                }
                if self.tutorial.mode == ZoomMode::Inset {
                    ui.horizontal(|ui| {
                        ui.label("放大窗位置");
                        egui::ComboBox::from_id_salt("tutorial-inset-corner").selected_text(self.tutorial.inset_corner.label()).show_ui(ui, |ui| {
                            for corner in [Corner::TopLeft,Corner::TopRight,Corner::BottomLeft,Corner::BottomRight] { ui.selectable_value(&mut self.tutorial.inset_corner,corner,corner.label()); }
                        });
                        ui.small("保留全景，放大窗占画面宽高约36%；选择不遮挡讲解内容的位置。");
                    });
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.tutorial.highlight,"鼠标高亮");
                    ui.checkbox(&mut self.tutorial.clicks,"左右键点击波纹");
                    ui.checkbox(&mut self.tutorial.spotlight,"鼠标聚光灯");
                });
                if self.tutorial.spotlight {
                    ui.add(egui::Slider::new(&mut self.tutorial.spotlight_radius,0.03..=0.45).text("聚光半径（画面短边比例）"));
                    ui.add(egui::Slider::new(&mut self.tutorial.spotlight_dim,0.0..=0.9).text("周围压暗程度"));
                    ui.small("鼠标离开选区时取消压暗；放大窗保持明亮。聚光灯用于强调，不用于隐藏敏感信息。");
                }
            });
            if !allowed { ui.small("教程效果支持最多1600万像素；请缩小录制区域。"); }
            if before != self.tutorial {
                let requested=self.tutorial;
                self.tutorial=before;
                self.set_tutorial(requested);
            }
            ui.small("放大保持输出尺寸；焦点限制在录制范围。鼠标离开选区时保留焦点、不画高亮。点击按状态采样，极快点击可能遗漏。录制中可调整；暂停清除波纹。");
            ui.small("统一前缀后：R T打开本页 · R Z开关鼠标聚焦放大 · R X关闭效果。按钮与设置显示当前生效状态。");
            ui.horizontal(|ui| {
                if ui.add_enabled(self.session.is_none() && self.countdown_deadline.is_none() && self.region.is_some(),egui::Button::new("预览所选区域当前画面")).clicked() {
                    let result=(|| -> anyhow::Result<()> {
                        let display=self.display.as_ref().ok_or_else(||anyhow::anyhow!("请先选择显示器"))?;
                        let region=self.region.ok_or_else(||anyhow::anyhow!("请先框选区域"))?;
                        recorder::validate_display(display)?;
                        anyhow::ensure!(region.fits(display.width,display.height),"区域已失效，请重新选择");
                        let image=capture_display_snapshot(display)?;
                        let scale=(640.0/region.width as f32).min(360.0/region.height as f32).min(1.0);
                        let size=[(region.width as f32*scale).round().max(1.0) as u32,(region.height as f32*scale).round().max(1.0) as u32];
                        let mut pixels=Vec::with_capacity((size[0]*size[1]*4) as usize);
                        for y in 0..size[1] { for x in 0..size[0] {
                            let sx=region.x+(u64::from(x)*u64::from(region.width)/u64::from(size[0])) as u32;
                            let sy=region.y+(u64::from(y)*u64::from(region.height)/u64::from(size[1])) as u32;
                            let p=image.pixels[(sy*display.width+sx) as usize];
                            pixels.extend_from_slice(&[p.b(),p.g(),p.r(),255]);
                        }}
                        self.tutorial_preview=Preview {source:pixels,size,label:"选区静态快照（仅内存）；在预览上移动、点击试用".into(),snapshot_region:Some((display.device_name.clone(),region)),..Preview::default()};
                        Ok(())
                    })();
                    if let Err(error)=result { self.status=format!("预览失败：{error:#}"); self.error=true; }
                }
                if ui.button("恢复示例 / 清除快照").clicked() { self.tutorial_preview=Preview::default(); }
            });
            if self.tutorial_preview.snapshot_region.as_ref().is_some_and(|(device,region)| self.display.as_ref().is_none_or(|d| d.device_name!=*device) || self.region!=Some(*region)) {
                self.tutorial_preview=Preview::default();
            }
            ui.small(&self.tutorial_preview.label);
            let preview=&mut self.tutorial_preview;
            let width=ui.available_width().min(preview.size[0] as f32);
            let (rect,response)=ui.allocate_exact_size(egui::vec2(width,width*preview.size[1] as f32/preview.size[0] as f32),Sense::click_and_drag());
            #[cfg(feature="ui-preview")]
            if std::mem::take(&mut self.preview_tutorial_scroll) { response.scroll_to_me(Some(egui::Align::Max)); }
            let position=ui.input(|i|i.pointer.hover_pos()).filter(|p|rect.contains(*p)).map(|p|[(p.x-rect.left())/rect.width()*preview.size[0] as f32,(p.y-rect.top())/rect.height()*preview.size[1] as f32]);
            let buttons=if response.hovered() { ui.input(|i|u8::from(i.pointer.primary_down()) | (u8::from(i.pointer.secondary_down())<<1)) } else { 0 };
            if preview.last.is_none_or(|at|at.elapsed()>=Duration::from_millis(33)) {
                let pointer = Pointer{position,buttons};
                #[cfg(feature="ui-preview")]
                let pointer = preview.fixture_pointer.unwrap_or(pointer);
                if preview.compositor.render(&preview.source,preview.size,self.tutorial,pointer,preview.started.elapsed(),&mut preview.output).is_ok() {
                    let rgba:Vec<u8>=preview.output.chunks_exact(4).flat_map(|p|[p[2],p[1],p[0],p[3]]).collect();
                    let image=egui::ColorImage::from_rgba_unmultiplied([preview.size[0] as usize,preview.size[1] as usize],&rgba);
                    if let Some(texture)=&mut preview.texture {texture.set(image,egui::TextureOptions::LINEAR);}
                    else {preview.texture=Some(ui.ctx().load_texture("tutorial-preview",image,egui::TextureOptions::LINEAR));}
                }
                preview.last=Some(Instant::now());
            }
            if let Some(texture)=&preview.texture {ui.painter().image(texture.id(),rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);}
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        });
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_spotlight_fixture(&mut self) {
        self.preview_tutorial_fixture(true);
        self.tutorial.mode = ZoomMode::Inset;
        self.tutorial.spotlight = true;
        self.tutorial.smooth = false;
        self.tutorial_preview.label = "合成示例：全景＋局部放大窗与聚光灯，未录制真实桌面".into();
        self.tutorial_preview.fixture_pointer = Some(Pointer {
            position: Some([400.0, 120.0]),
            buttons: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_region_rejects_enable_but_allows_clearing_old_effects() {
        let mut state = RecorderState::default();
        state.region = Some(Region {
            x: 0,
            y: 0,
            width: 8000,
            height: 4000,
        });
        state.toggle_tutorial_zoom();
        assert_eq!(state.tutorial.mode, ZoomMode::Off);
        assert!(state.error);
        state.tutorial = Settings {
            mode: ZoomMode::Fixed,
            highlight: true,
            ..Settings::default()
        };
        state.disable_tutorial_effects();
        assert!(!state.tutorial.active());
    }
    #[test]
    fn countdown_keeps_the_settings_frozen() {
        let mut state = RecorderState::default();
        state.countdown_deadline = Some(Instant::now() + Duration::from_secs(3));
        state.toggle_tutorial_zoom();
        assert_eq!(state.tutorial, Settings::default());
    }
}
