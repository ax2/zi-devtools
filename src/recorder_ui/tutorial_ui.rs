use super::*;
use crate::recorder::tutorial::{
    Compositor, Corner, DrawingColor, Pointer, Settings, ShortcutHint, ZoomMode,
};

const TELEPROMPTER_TEXT_LIMIT: usize = 12_000;
const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;

pub(super) struct TeleprompterState {
    text: String,
    open: bool,
    editing: bool,
    playing: bool,
    scroll_y: f32,
    speed: f32,
    font_size: f32,
    last_tick: Instant,
    capture_excluded: bool,
    #[cfg(feature = "ui-preview")]
    capture_requested: bool,
}

impl Default for TeleprompterState {
    fn default() -> Self {
        Self {
            text: String::new(),
            open: false,
            editing: true,
            playing: false,
            scroll_y: 0.0,
            speed: 36.0,
            font_size: 30.0,
            last_tick: Instant::now(),
            capture_excluded: false,
            #[cfg(feature = "ui-preview")]
            capture_requested: false,
        }
    }
}

fn teleprompter_title() -> String {
    format!("Zi DevTools · 提词器 · {}", std::process::id())
}

fn advance_teleprompter_scroll(
    offset: f32,
    speed: f32,
    elapsed: Duration,
    max_offset: f32,
) -> (f32, bool) {
    let max_offset = max_offset.max(0.0);
    let next = (offset.max(0.0) + speed.max(0.0) * elapsed.as_secs_f32().clamp(0.0, 0.1))
        .clamp(0.0, max_offset);
    (next, max_offset > 0.0 && next < max_offset)
}

#[cfg(windows)]
fn exclude_teleprompter_from_capture(title: &str) -> anyhow::Result<()> {
    use windows_sys::Win32::{
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            FindWindowW, GetWindowDisplayAffinity, GetWindowThreadProcessId,
            SetWindowDisplayAffinity,
        },
    };
    let title = format!("{title}\0").encode_utf16().collect::<Vec<_>>();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        anyhow::bail!("提词窗口正在创建，请稍候");
    }
    let mut owner = 0;
    if unsafe { GetWindowThreadProcessId(hwnd, &mut owner) } == 0
        || owner != unsafe { GetCurrentProcessId() }
    {
        anyhow::bail!("无法确认提词窗口属于当前程序");
    }
    if unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } == 0 {
        anyhow::bail!(
            "Windows 无法将提词窗口从录屏中排除：{}",
            std::io::Error::last_os_error()
        );
    }
    let mut affinity = 0;
    if unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) } == 0
        || affinity != WDA_EXCLUDEFROMCAPTURE
    {
        anyhow::bail!("Windows 未确认提词窗口的录屏排除状态");
    }
    Ok(())
}

#[cfg(not(windows))]
fn exclude_teleprompter_from_capture(_: &str) -> anyhow::Result<()> {
    anyhow::bail!("当前系统不支持安全显示提词窗口")
}

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
    pub fn focus_tutorial_at_cursor(&mut self) {
        if self.countdown_deadline.is_some() {
            return;
        }
        let (Some(display), Some(region)) = (self.display.as_ref(), self.region) else {
            self.status = "请先选择显示器并框选录制区域，再使用当前位置聚焦".into();
            self.error = true;
            return;
        };
        let mut cursor = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetPhysicalCursorPos(&mut cursor) }
            == 0
        {
            self.status = format!("读取鼠标物理坐标失败：{}", std::io::Error::last_os_error());
            self.error = true;
            return;
        }
        let local = [
            i64::from(cursor.x) - i64::from(display.x) - i64::from(region.x),
            i64::from(cursor.y) - i64::from(display.y) - i64::from(region.y),
        ];
        let Some(settings) = focus_settings(self.tutorial, local, region) else {
            self.status = "当前位置不在录制区域内；焦点保持不变".into();
            self.error = true;
            return;
        };
        self.set_tutorial(settings);
        if self.tutorial == settings {
            self.status = "已将固定放大焦点移至鼠标当前位置".into();
            self.error = false;
        }
    }
    pub fn disable_tutorial_effects(&mut self) {
        self.set_tutorial(Settings {
            draw_clear_epoch: self.tutorial.draw_clear_epoch.wrapping_add(1),
            ..Settings::default()
        });
    }
    pub fn show_tutorial(&mut self) {
        self.tutorial_open_requested = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_tutorial_fixture(&mut self, scroll: bool) {
        self.preview_fixture();
        self.entry_id = "recorder-tutorial".into();
        self.preview_tutorial_scroll = scroll;
        self.tutorial_shortcut_text = "CTRL+SHIFT+S".into();
        self.tutorial = Settings {
            mode: ZoomMode::Fixed,
            scale: 2.5,
            focus: [0.65, 0.45],
            highlight: true,
            clicks: true,
            pointer_arrow: true,
            step_number: Some(2),
            step_corner: Corner::BottomRight,
            shortcut_hint: Some(
                ShortcutHint::parse("CTRL+SHIFT+S", Corner::TopLeft)
                    .expect("fixture shortcut is valid"),
            ),
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
                    if ui.button("关闭所有效果").clicked() {
                        self.tutorial = Settings {
                            draw_clear_epoch: self.tutorial.draw_clear_epoch.wrapping_add(1),
                            ..Settings::default()
                        };
                    }
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
                    ui.checkbox(&mut self.tutorial.pointer_arrow,"方向箭头指针");
                    ui.checkbox(&mut self.tutorial.spotlight,"鼠标聚光灯");
                });
                if self.tutorial.pointer_arrow {
                    ui.small("高对比箭头跟随鼠标并写入预览与导出视频；默认关闭，不采集键盘内容。");
                }
                if self.tutorial.spotlight {
                    ui.add(egui::Slider::new(&mut self.tutorial.spotlight_radius,0.03..=0.45).text("聚光半径（画面短边比例）"));
                    ui.add(egui::Slider::new(&mut self.tutorial.spotlight_dim,0.0..=0.9).text("周围压暗程度"));
                    ui.small("鼠标离开选区时取消压暗；放大窗保持明亮。聚光灯用于强调，不用于隐藏敏感信息。");
                }
                ui.horizontal(|ui| {
                    let label = self.tutorial.step_number.map_or_else(
                        || "显示步骤编号".to_owned(),
                        |number| format!("步骤 {number}"),
                    );
                    if ui.selectable_label(self.tutorial.step_number.is_some(), label).clicked() {
                        self.tutorial.step_number = if self.tutorial.step_number.is_some() {
                            None
                        } else {
                            Some(1)
                        };
                    }
                    if let Some(number) = self.tutorial.step_number {
                        if ui.add_enabled(number < 99, egui::Button::new("下一步")).clicked() {
                            self.tutorial.step_number = Some(number + 1);
                        }
                        if ui.button("从 1 重新开始").clicked() {
                            self.tutorial.step_number = Some(1);
                        }
                        egui::ComboBox::from_id_salt("tutorial-step-corner")
                            .selected_text(self.tutorial.step_corner.label())
                            .show_ui(ui, |ui| {
                                for corner in [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight] {
                                    ui.selectable_value(&mut self.tutorial.step_corner, corner, corner.label());
                                }
                            });
                    }
                });
                if self.tutorial.step_number.is_some() {
                    ui.small("步骤编号会写入导出视频；只显示数字，不读取或显示键盘输入。99 后停止递增。");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("手动按键组合");
                    ui.add(egui::TextEdit::singleline(&mut self.tutorial_shortcut_text).hint_text("例如 CTRL+SHIFT+S").char_limit(24));
                    let active_hint = self.tutorial.shortcut_hint;
                    let active = active_hint.is_some();
                    let parsed = ShortcutHint::parse(
                        &self.tutorial_shortcut_text,
                        active_hint.map_or(Corner::TopLeft, |hint| hint.corner),
                    );
                    let changed = active_hint.is_some_and(|hint| {
                        parsed.as_ref().is_ok_and(|candidate| candidate.text() != hint.text())
                    });
                    let action = if !active {
                        "添加到视频"
                    } else if changed {
                        "应用更改"
                    } else {
                        "关闭提示"
                    };
                    if ui.button(action).clicked() {
                        if active && !changed {
                            self.tutorial.shortcut_hint = None;
                        } else {
                            match parsed {
                                Ok(hint) => self.tutorial.shortcut_hint = Some(hint),
                                Err(error) => {
                                    self.status = error.to_string();
                                    self.error = true;
                                }
                            }
                        }
                        if self.tutorial.shortcut_hint != active_hint {
                            self.status = if self.tutorial.shortcut_hint.is_some() {
                                "手动按键组合提示已更新，并写入预览与录制帧".into()
                            } else {
                                "手动按键组合提示已关闭".into()
                            };
                            self.error = false;
                        }
                    }
                    if let Some(mut hint) = self.tutorial.shortcut_hint {
                        egui::ComboBox::from_id_salt("tutorial-shortcut-corner")
                            .selected_text(hint.corner.label())
                            .show_ui(ui, |ui| {
                                for corner in [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight] {
                                    ui.selectable_value(&mut hint.corner, corner, corner.label());
                                }
                            });
                        self.tutorial.shortcut_hint = Some(hint);
                    }
                });
                if self.tutorial.shortcut_hint.is_some() {
                    ui.small("仅显示你手动填写的组合，并写入预览和导出视频；不监听键盘，也不读取实际输入。最多24个ASCII字母、数字、空格及 + - /。");
                } else {
                    ui.small("提示默认关闭；添加前请检查组合中不含账号、密码或其他敏感内容。");
                }
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self.tutorial.show_pressed_shortcuts,
                        "录制实际快捷键组合",
                    );
                    if self.tutorial.show_pressed_shortcuts {
                        egui::ComboBox::from_id_salt("tutorial-pressed-shortcut-corner")
                            .selected_text(self.tutorial.pressed_shortcut_corner.label())
                            .show_ui(ui, |ui| {
                                for corner in [
                                    Corner::TopLeft,
                                    Corner::TopRight,
                                    Corner::BottomLeft,
                                    Corner::BottomRight,
                                ] {
                                    ui.selectable_value(
                                        &mut self.tutorial.pressed_shortcut_corner,
                                        corner,
                                        corner.label(),
                                    );
                                }
                            });
                    }
                });
                if self.tutorial.show_pressed_shortcuts {
                    ui.small("仅在录制画面采样 Ctrl/Alt/Win 加一个可识别按键；不显示未修饰文字或多键输入，不建日志、不保存。暂停时停止采样；请避免在录制敏感窗口时启用。");
                } else {
                    ui.small("实际快捷键显示默认关闭；开启后只显示含 Ctrl/Alt/Win 的单键组合，可随时暂停或关闭。");
                }
                if ui.button("打开本机提词器…").clicked() {
                    self.teleprompter.open = true;
                    self.teleprompter.editing = self.teleprompter.text.is_empty();
                    self.teleprompter.last_tick = Instant::now();
                }
                ui.small("提词正文仅保存在本次运行内存中。浮窗会请求 Windows 录屏排除；无法确认时不会显示正文。适用于本程序录制路径，不是防止外部拍摄的安全功能。Windows 10 2004 及以上支持完整排除。 ");
                ui.horizontal(|ui| {
                    if ui.button(if self.tutorial.draw_enabled {
                        "结束画笔"
                    } else {
                        "开启画笔"
                    }).clicked() {
                        self.tutorial.draw_enabled = !self.tutorial.draw_enabled;
                    }
                    egui::ComboBox::from_id_salt("tutorial-draw-color")
                        .selected_text(self.tutorial.draw_color.label())
                        .show_ui(ui, |ui| {
                            for color in [DrawingColor::Red, DrawingColor::Yellow, DrawingColor::Cyan, DrawingColor::White] {
                                ui.selectable_value(&mut self.tutorial.draw_color, color, color.label());
                            }
                        });
                    if ui.button("清除标注").clicked() {
                        self.tutorial.draw_clear_epoch = self.tutorial.draw_clear_epoch.wrapping_add(1);
                    }
                });
                ui.small("开启后在预览或录制选区按住鼠标左键拖动绘制；笔迹会保留到本次录制结束或手动清除，并直接写入视频。暂停期间不继续绘制。");
            });
            if !allowed { ui.small("教程效果支持最多1600万像素；请缩小录制区域。"); }
            if before != self.tutorial {
                let requested=self.tutorial;
                self.tutorial=before;
                self.set_tutorial(requested);
            }
            ui.small("放大保持输出尺寸；焦点限制在录制范围。鼠标离开选区时保留焦点、不画高亮。点击按状态采样，极快点击可能遗漏。录制中可调整；暂停清除波纹。");
            ui.small("统一前缀后：R T打开本页 · R Z开关鼠标跟随放大 · R F固定聚焦当前位置 · R X关闭效果。当前位置需在录制选区内。");
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
    pub fn preview_teleprompter_fixture(&mut self) {
        self.output = r"C:\Users\Example\Videos\tutorial-demo.mp4".into();
        self.teleprompter.text = "欢迎使用本地教程录屏。这里的稿件只保留在当前运行内存中，不会写入视频、设置或日志。\n\n点击开始滚动，按自己的语速讲解；滚动到结尾会自动暂停，也可以随时手动暂停或回到开头。\n\n录制前请确认提词浮窗已成功排除，并确认画面没有显示不该录制的信息。".into();
        self.teleprompter.open = true;
        self.teleprompter.editing = false;
        self.teleprompter.playing = false;
        self.teleprompter.scroll_y = 0.0;
        self.teleprompter.last_tick = Instant::now();
        self.teleprompter.capture_requested = false;
        self.tutorial_open_requested = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_spotlight_fixture(&mut self) {
        self.preview_tutorial_fixture(true);
        self.tutorial.mode = ZoomMode::Inset;
        self.tutorial.spotlight = true;
        self.tutorial.smooth = false;
        self.tutorial.step_number = Some(2);
        self.tutorial.step_corner = Corner::BottomRight;
        self.tutorial_preview.label = "合成示例：全景＋局部放大窗与聚光灯，未录制真实桌面".into();
        self.tutorial_preview.fixture_pointer = Some(Pointer {
            position: Some([400.0, 120.0]),
            buttons: 0,
        });
    }

    pub(super) fn teleprompter_ui(&mut self, ctx: &egui::Context) {
        if !self.teleprompter.open {
            return;
        }
        let title = teleprompter_title();
        let viewport_id =
            egui::ViewportId::from_hash_of(("recorder-teleprompter", std::process::id()));
        let builder = egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([620.0, 500.0])
            .with_min_inner_size([380.0, 300.0])
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        ctx.show_viewport_immediate(viewport_id, builder, |panel, _| {
            if panel.input(|input| input.viewport().close_requested()) {
                self.teleprompter.open = false;
                self.teleprompter.capture_excluded = false;
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            let exclusion = if self.teleprompter.capture_excluded {
                Ok(())
            } else {
                let result = exclude_teleprompter_from_capture(&title);
                self.teleprompter.capture_excluded = result.is_ok();
                result
            };
            #[cfg(feature = "ui-preview")]
            let capture_fixture = std::env::args().nth(3).as_deref()
                == Some("teleprompter-affinity-smoke");
            #[cfg(feature = "ui-preview")]
            if capture_fixture && exclusion.is_ok() && !self.teleprompter.capture_requested {
                self.teleprompter.capture_requested = true;
                let folder = std::path::PathBuf::from(
                    std::env::args().nth(1).expect("capture output directory"),
                );
                std::fs::write(
                    folder.join("teleprompter-affinity-proof.txt"),
                    "SetWindowDisplayAffinity and GetWindowDisplayAffinity returned WDA_EXCLUDEFROMCAPTURE (0x11).",
                )
                .expect("write teleprompter affinity proof");
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::from_rgb(20, 27, 39)).inner_margin(18.0))
                .show(panel, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading("录屏提词器");
                        ui.label(RichText::new("仅本机临时文本").color(Color32::from_rgb(150, 205, 255)));
                        if ui.button(if self.teleprompter.editing { "预览稿件" } else { "编辑稿件" }).clicked() {
                            self.teleprompter.editing = !self.teleprompter.editing;
                            self.teleprompter.playing = false;
                            if !self.teleprompter.editing { self.teleprompter.scroll_y = 0.0; }
                        }
                    });
                    ui.separator();
                    if let Err(error) = &exclusion {
                        ui.colored_label(Color32::from_rgb(255, 185, 95), "录屏排除尚未确认，提词正文已隐藏");
                        ui.small(error.to_string());
                        ui.small("请等待浮窗完成初始化；如果持续失败，请关闭提词器，勿将明文稿件留在未排除的窗口中。");
                        panel.request_repaint_after(Duration::from_millis(100));
                        return;
                    }
                    if self.teleprompter.editing {
                        ui.label("在此粘贴讲解稿。内容不会写入录制画面、设置或日志。");
                        let edit = egui::TextEdit::multiline(&mut self.teleprompter.text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(12)
                            .char_limit(TELEPROMPTER_TEXT_LIMIT);
                        ui.add(edit);
                        ui.small(format!("{} / {} 字符", self.teleprompter.text.chars().count(), TELEPROMPTER_TEXT_LIMIT));
                    } else if self.teleprompter.text.trim().is_empty() {
                        ui.label("稿件为空。点击“编辑稿件”添加讲解内容。");
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(if self.teleprompter.playing { "暂停滚动" } else { "开始滚动" }).clicked() {
                                self.teleprompter.playing = !self.teleprompter.playing;
                                self.teleprompter.last_tick = Instant::now();
                            }
                            if ui.button("回到开头").clicked() {
                                self.teleprompter.scroll_y = 0.0;
                                self.teleprompter.playing = false;
                            }
                            ui.add(egui::Slider::new(&mut self.teleprompter.speed, 12.0..=100.0).suffix(" 像素/秒").text("速度"));
                            ui.add(egui::Slider::new(&mut self.teleprompter.font_size, 20.0..=48.0).suffix(" px").text("字号"));
                        });
                        let now = Instant::now();
                        let delta = now.saturating_duration_since(self.teleprompter.last_tick).as_secs_f32().min(0.1);
                        self.teleprompter.last_tick = now;
                        let output = egui::ScrollArea::vertical()
                            .id_salt("recorder-teleprompter-text")
                            .vertical_scroll_offset(self.teleprompter.scroll_y)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&self.teleprompter.text)
                                            .size(self.teleprompter.font_size)
                                            .color(Color32::from_rgb(242, 245, 250)),
                                    )
                                    .wrap(),
                                );
                            });
                        let max_scroll = (output.content_size.y - output.inner_rect.height()).max(0.0);
                        if self.teleprompter.playing {
                            let (next, keep_playing) = advance_teleprompter_scroll(
                                output.state.offset.y,
                                self.teleprompter.speed,
                                Duration::from_secs_f32(delta),
                                max_scroll,
                            );
                            self.teleprompter.scroll_y = next;
                            self.teleprompter.playing = keep_playing;
                            if keep_playing {
                                panel.request_repaint_after(Duration::from_millis(16));
                            }
                        } else {
                            self.teleprompter.scroll_y = output.state.offset.y;
                        }
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.small("此浮窗仅对启用 Windows 显示亲和性的录屏接口隐藏；系统版本、其他采集软件或外部拍摄可能不遵循该设置。开始录制前请用测试画面确认。");
                    });
                });
        });
    }
}

fn focus_settings(mut settings: Settings, local: [i64; 2], region: Region) -> Option<Settings> {
    if region.width == 0
        || region.height == 0
        || local[0] < 0
        || local[1] < 0
        || local[0] >= i64::from(region.width)
        || local[1] >= i64::from(region.height)
    {
        return None;
    }
    settings.mode = ZoomMode::Fixed;
    settings.focus = [
        local[0] as f32 / region.width.saturating_sub(1).max(1) as f32,
        local[1] as f32 / region.height.saturating_sub(1).max(1) as f32,
    ];
    Some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teleprompter_scroll_is_bounded_and_caps_large_frame_gaps() {
        assert_eq!(
            advance_teleprompter_scroll(10.0, 20.0, Duration::from_secs(10), 100.0),
            (12.0, true)
        );
        assert_eq!(
            advance_teleprompter_scroll(99.0, 100.0, Duration::from_millis(100), 100.0),
            (100.0, false)
        );
        assert_eq!(
            advance_teleprompter_scroll(-10.0, -2.0, Duration::from_secs(1), 0.0),
            (0.0, false)
        );
    }

    #[test]
    fn focus_shortcut_uses_region_relative_cursor_and_preserves_other_effects() {
        let current = Settings {
            mode: ZoomMode::Follow,
            scale: 3.0,
            pointer_arrow: true,
            ..Settings::default()
        };
        let focused = focus_settings(
            current,
            [499, 249],
            Region {
                x: 200,
                y: 100,
                width: 500,
                height: 250,
            },
        )
        .unwrap();
        assert_eq!(focused.mode, ZoomMode::Fixed);
        assert_eq!(focused.focus, [1.0, 1.0]);
        assert_eq!(focused.scale, 3.0);
        assert!(focused.pointer_arrow);
        assert!(
            focus_settings(
                current,
                [-1, 2],
                Region {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10
                }
            )
            .is_none()
        );
    }
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
