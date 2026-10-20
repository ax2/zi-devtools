use super::*;
use chrono::Datelike;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Target {
    World(Tz),
    Stopwatch,
    Timer(u64),
    Focus,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedSize {
    target: String,
    width: u16,
    height: u16,
}
impl SavedSize {
    fn from_target_size(target: Target, size: egui::Vec2) -> Self {
        let target = match target {
            Target::World(zone) => format!("world:{}", zone.name()),
            Target::Stopwatch => "stopwatch".into(),
            Target::Timer(id) => format!("timer:{id}"),
            Target::Focus => "focus".into(),
        };
        Self {
            target,
            width: size.x.round() as u16,
            height: size.y.round() as u16,
        }
    }
    pub(super) fn validate(&self) -> Result<Target, String> {
        let target = match self.target.as_str() {
            "stopwatch" => Target::Stopwatch,
            "focus" => Target::Focus,
            value if value.starts_with("world:") => value[6..]
                .parse::<Tz>()
                .map(Target::World)
                .map_err(|_| "无效的小窗时区".to_owned())?,
            value if value.starts_with("timer:") => {
                let id = value[6..]
                    .parse::<u64>()
                    .map_err(|_| "无效的小窗计时器".to_owned())?;
                if id == 0 || id == u64::MAX {
                    return Err("无效的小窗计时器".into());
                }
                Target::Timer(id)
            }
            _ => return Err("无效的小窗目标".into()),
        };
        if !(320..=1600).contains(&self.width) || !(260..=1200).contains(&self.height) {
            return Err("小窗尺寸越界".into());
        }
        Ok(target)
    }
}
#[derive(Debug, Clone)]
struct Pane {
    id: u64,
    target: Target,
    pinned: bool,
    fullscreen: bool,
    reset_confirmation: bool,
    size: egui::Vec2,
}
impl Pane {
    fn viewport(&self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("zi-clock-window", self.id))
    }
    fn level(&self) -> egui::WindowLevel {
        if self.pinned && !self.fullscreen {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }
    }
}
#[cfg_attr(not(feature = "ui-preview"), derive(Default))]
pub(super) struct Windows {
    panes: Vec<Pane>,
    next_id: u64,
    recent_sizes: Vec<(Target, egui::Vec2)>,
    #[cfg(feature = "ui-preview")]
    rects: [egui::Rect; 5],
    #[cfg(feature = "ui-preview")]
    actual_fullscreen: bool,
    #[cfg(feature = "ui-preview")]
    scale: f32,
    #[cfg(feature = "ui-preview")]
    preview_size_before_reopen: Option<egui::Vec2>,
}
#[cfg(feature = "ui-preview")]
impl Default for Windows {
    fn default() -> Self {
        Self {
            panes: Vec::new(),
            next_id: 0,
            recent_sizes: Vec::new(),
            #[cfg(feature = "ui-preview")]
            rects: [egui::Rect::NOTHING; 5],
            #[cfg(feature = "ui-preview")]
            actual_fullscreen: false,
            #[cfg(feature = "ui-preview")]
            scale: 1.0,
            #[cfg(feature = "ui-preview")]
            preview_size_before_reopen: None,
        }
    }
}
impl Windows {
    pub fn active(&self) -> bool {
        !self.panes.is_empty()
    }
    pub fn clear(&mut self) {
        // Preserve the monotonically increasing viewport ID across restoration.
        self.panes.clear();
    }
    fn open(&mut self, target: Target, ctx: &egui::Context) -> Result<(), String> {
        if let Some(pane) = self.panes.iter().find(|p| p.target == target) {
            ctx.send_viewport_cmd_to(pane.viewport(), egui::ViewportCommand::Focus);
            return Ok(());
        }
        if self.panes.len() >= 8 {
            return Err("最多同时打开8个时钟小窗；请先关闭一个。".into());
        }
        self.next_id = self.next_id.saturating_add(1);
        let size = self
            .recent_sizes
            .iter()
            .find_map(|(recent_target, size)| (*recent_target == target).then_some(*size))
            .unwrap_or_else(|| egui::vec2(440.0, 350.0));
        self.panes.push(Pane {
            id: self.next_id,
            target,
            pinned: false,
            fullscreen: false,
            reset_confirmation: false,
            size,
        });
        ctx.request_repaint();
        Ok(())
    }
    pub(super) fn remember_size(&mut self, target: Target, size: egui::Vec2) -> bool {
        let size = egui::vec2(
            if size.x.is_finite() {
                size.x.clamp(320.0, 1600.0)
            } else {
                440.0
            },
            if size.y.is_finite() {
                size.y.clamp(260.0, 1200.0)
            } else {
                350.0
            },
        );
        if let Some(index) = self
            .recent_sizes
            .iter()
            .position(|(recent_target, _)| *recent_target == target)
        {
            if SavedSize::from_target_size(target, self.recent_sizes[index].1)
                == SavedSize::from_target_size(target, size)
            {
                return false;
            }
            self.recent_sizes.remove(index);
        } else if self.recent_sizes.len() == 48 {
            self.recent_sizes.remove(0);
        }
        self.recent_sizes.push((target, size));
        true
    }
    pub(super) fn saved_sizes(&self) -> Vec<SavedSize> {
        self.recent_sizes
            .iter()
            .map(|(target, size)| SavedSize::from_target_size(*target, *size))
            .collect()
    }
    pub(super) fn restore_sizes(&mut self, sizes: &[SavedSize]) {
        self.recent_sizes = sizes
            .iter()
            .map(|size| {
                let target = size.validate().expect("validated saved window size");
                (
                    target,
                    egui::vec2(f32::from(size.width), f32::from(size.height)),
                )
            })
            .collect();
    }
}
#[derive(Clone, Copy)]
enum Action {
    Toggle,
    Lap,
    Reset,
    Next,
}
impl State {
    pub(super) fn open_window(&mut self, target: Target, ctx: &egui::Context) {
        self.message = self.windows.open(target, ctx).err().unwrap_or_default();
    }
    fn window_action(&mut self, target: Target, action: Action, now: Instant) -> bool {
        match target {
            Target::World(_) => return false,
            Target::Stopwatch => match action {
                Action::Toggle => self.stopwatch.toggle(now),
                Action::Lap if self.stopwatch.running() && self.stopwatch.laps.len() < 1024 => {
                    self.stopwatch.lap(now);
                }
                Action::Reset => self.stopwatch.reset(),
                _ => return false,
            },
            Target::Timer(id) => {
                let Some(timer) = self.timers.iter_mut().find(|t| t.id == id) else {
                    return false;
                };
                match action {
                    Action::Toggle if !timer.finished => timer.toggle(now),
                    Action::Reset => {
                        timer.restart();
                        self.notices.retain(|n| n.source != Source::Timer(id));
                    }
                    _ => return false,
                }
            }
            Target::Focus => match action {
                Action::Toggle if !self.focus.timer.finished => self.focus.timer.toggle(now),
                Action::Reset => {
                    self.focus.reset();
                    self.notices.retain(|n| n.source != Source::Focus);
                }
                Action::Next if self.focus.timer.finished => {
                    self.focus.next();
                    self.notices.retain(|n| n.source != Source::Focus);
                }
                _ => return false,
            },
        }
        self.changed();
        true
    }
    /// Render before the main workbench's hidden-window early return.
    /// All immediate viewports use this same State; none owns a scheduler.
    pub fn window_ui(&mut self, ctx: &egui::Context) -> bool {
        let panes = std::mem::take(&mut self.windows.panes);
        let mut show_main = false;
        for mut pane in panes {
            let mut open = true;
            let id = pane.viewport();
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Zi DevTools · 时钟小窗 · {}", pane.id))
                .with_min_inner_size([320.0, 260.0])
                .with_taskbar(true)
                .with_window_level(pane.level())
                .with_fullscreen(pane.fullscreen);
            let builder = if pane.fullscreen {
                builder
            } else {
                builder.with_inner_size(pane.size)
            };
            ctx.show_viewport_immediate(id, builder, |panel, _| {
                #[cfg(feature = "ui-preview")]
                {
                    self.windows.actual_fullscreen =
                        panel.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    self.windows.scale = panel.pixels_per_point();
                }
                if !pane.fullscreen
                    && !panel.input(|i| i.viewport().maximized.unwrap_or(false))
                    && let Some(rect) = panel.input(|i| i.viewport().inner_rect)
                {
                    pane.size = rect.size();
                }
                if panel.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                if pane.fullscreen
                    && panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    pane.fullscreen = false;
                    panel.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    panel.send_viewport_cmd(egui::ViewportCommand::WindowLevel(pane.level()));
                }
                if !pane.reset_confirmation && !matches!(pane.target, Target::World(_)) {
                    if panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                        self.window_action(pane.target, Action::Toggle, Instant::now());
                    }
                    if pane.target == Target::Stopwatch
                        && panel.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::L))
                    {
                        self.window_action(pane.target, Action::Lap, Instant::now());
                    }
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::central_panel(&panel.style()).inner_margin(18.0))
                    .show(panel, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let pinned = ui.checkbox(&mut pane.pinned, "置顶");
                            #[cfg(feature = "ui-preview")]
                            {
                                self.windows.rects[0] = pinned.rect;
                            }
                            if pinned.changed() {
                                panel.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                                    pane.level(),
                                ));
                            }
                            let fullscreen = ui.button(if pane.fullscreen {
                                "退出全屏 · Esc"
                            } else {
                                "全屏"
                            });
                            #[cfg(feature = "ui-preview")]
                            {
                                self.windows.rects[1] = fullscreen.rect;
                            }
                            if fullscreen.clicked() {
                                pane.fullscreen = !pane.fullscreen;
                                panel.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                    pane.fullscreen,
                                ));
                                panel.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                                    pane.level(),
                                ));
                            }
                            let main = ui.button("工作台 ↗");
                            #[cfg(feature = "ui-preview")]
                            {
                                self.windows.rects[2] = main.rect;
                            }
                            if main.clicked() {
                                show_main = true;
                            }
                            let close = ui.button("关闭小窗");
                            #[cfg(feature = "ui-preview")]
                            {
                                self.windows.rects[3] = close.rect;
                            }
                            if close.clicked() {
                                open = false;
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if pane.fullscreen {
                                ui.add_space((panel.screen_rect().height() * 0.2).min(240.0));
                                ui.vertical_centered(|ui| {
                                    self.pane_ui(ui, &mut pane);
                                });
                            } else {
                                self.pane_ui(ui, &mut pane);
                            }
                        });
                    });
                panel.request_repaint_after(Duration::from_millis(if self.stopwatch.running() {
                    50
                } else {
                    250
                }));
            });
            if self.windows.remember_size(pane.target, pane.size) {
                self.changed();
            }
            if open {
                self.windows.panes.push(pane);
            }
        }
        show_main
    }
    fn pane_ui(&mut self, ui: &mut egui::Ui, pane: &mut Pane) {
        let now = Instant::now();
        let mut action = None;
        let (name, time, running, finished) = match pane.target {
            Target::World(zone) => {
                let local = Utc::now().with_timezone(&zone);
                ui.heading(super::ui::zone_label(zone));
                ui.label(format!(
                    "{} · {} · UTC{}",
                    local.format("%Y-%m-%d"),
                    ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                        [local.weekday().num_days_from_monday() as usize],
                    local.format("%:z")
                ));
                let size = (ui.available_width() / 7.0).clamp(32.0, 144.0);
                ui.label(
                    egui::RichText::new(local.format("%H:%M:%S").to_string())
                        .monospace()
                        .size(size),
                );
                ui.small("本机时间 · 关闭小窗不影响其他时钟或提醒");
                return;
            }
            Target::Stopwatch => (
                "秒表".into(),
                format!(
                    "{}.{:03}",
                    duration_text(self.stopwatch.elapsed(now)),
                    self.stopwatch.elapsed(now).subsec_millis()
                ),
                self.stopwatch.running(),
                false,
            ),
            Target::Timer(id) => {
                let Some(timer) = self.timers.iter().find(|t| t.id == id) else {
                    ui.heading("原计时器已不存在");
                    ui.label("计时器可能已在工作台删除。此窗不会控制其他计时器，请关闭或返回工作台重新打开。");
                    return;
                };
                (
                    timer.name.clone(),
                    countdown_text(timer.remaining(now)),
                    timer.running(),
                    timer.finished,
                )
            }
            Target::Focus => (
                format!("{} · 已完成{}轮", self.focus.label(), self.focus.completed),
                countdown_text(self.focus.timer.remaining(now)),
                self.focus.timer.running(),
                self.focus.timer.finished,
            ),
        };
        ui.heading(name);
        let size = (ui.available_width()
            / if pane.target == Target::Stopwatch {
                9.0
            } else {
                7.0
            })
        .clamp(28.0, 144.0);
        ui.label(egui::RichText::new(time).monospace().size(size));
        ui.label(if finished {
            "已结束 · 提醒保留在工作台"
        } else if running {
            "计时中"
        } else {
            "已暂停 / 待开始"
        });
        ui.horizontal_wrapped(|ui| {
            let toggle = ui.add_enabled(
                !finished,
                egui::Button::new(if running {
                    "暂停 · Space"
                } else {
                    "开始 / 继续 · Space"
                }),
            );
            #[cfg(feature = "ui-preview")]
            {
                self.windows.rects[4] = toggle.rect;
            }
            if toggle.clicked() {
                action = Some(Action::Toggle);
            }
            if pane.target == Target::Stopwatch
                && ui
                    .add_enabled(
                        running && self.stopwatch.laps.len() < 1024,
                        egui::Button::new("分段 · L"),
                    )
                    .clicked()
            {
                action = Some(Action::Lap);
            }
            if pane.target == Target::Focus
                && ui
                    .add_enabled(finished, egui::Button::new("下一阶段（待开始）"))
                    .clicked()
            {
                action = Some(Action::Next);
            }
            if ui.button("重置…").clicked() {
                pane.reset_confirmation = true;
            }
        });
        if pane.reset_confirmation {
            ui.group(|ui| {
                ui.label(if pane.target == Target::Stopwatch {
                    "重置会清空累计与全部分段。"
                } else if pane.target == Target::Focus {
                    "重置会清空本轮专注计数和提醒。"
                } else {
                    "重置回原时长并暂停，清除此计时器提醒。"
                });
                ui.horizontal(|ui| {
                    if ui.button("确认重置").clicked() {
                        action = Some(Action::Reset);
                        pane.reset_confirmation = false;
                    }
                    if ui.button("取消").clicked() {
                        pane.reset_confirmation = false;
                    }
                });
            });
        }
        if let Some(action) = action {
            self.window_action(pane.target, action, now);
        }
        if pane.target == Target::Stopwatch {
            ui.small(format!(
                "已记录{}段；完整分段在工作台查看",
                self.stopwatch.laps.len()
            ));
            if let Some(lap) = self.stopwatch.laps.last() {
                ui.monospace(format!(
                    "最近分段总计 {}.{:03}",
                    duration_text(*lap),
                    lap.subsec_millis()
                ));
            }
        }
        ui.small("与工作台同步 · 关闭此窗继续计时 · 完全退出程序后停止提醒");
    }
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;

#[cfg(feature = "ui-preview")]
#[path = "windows_preview.rs"]
mod preview;
