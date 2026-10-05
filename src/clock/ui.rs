use super::*;
use chrono::{Datelike, Timelike};
use eframe::egui;
impl State {
    pub fn ui(&mut self, ui: &mut egui::Ui, version: &str) {
        ui.heading("时钟工作台");
        ui.label(format!("v{version} · 开发中"));
        ui.small("当前保存在运行会话；隐藏托盘继续计时，完全退出清空。保存恢复、声音与独立小窗继续开发。");
        ui.horizontal_wrapped(|ui| {
            for (tab, title) in [
                (Tab::World, "世界时钟"),
                (Tab::Stopwatch, "秒表 / 分段"),
                (Tab::Timers, "多计时器"),
                (Tab::Alarms, "闹钟"),
                (Tab::Focus, "番茄专注"),
            ] {
                ui.selectable_value(&mut self.tab, tab, title);
            }
        });
        ui.separator();
        let now = Instant::now();
        let utc = Utc::now();
        match self.tab {
            Tab::World => self.world_ui(ui, utc),
            Tab::Stopwatch => {
                ui.label(
                    egui::RichText::new(format!(
                        "{}.{:03}",
                        duration_text(self.stopwatch.elapsed(now)),
                        self.stopwatch.elapsed(now).subsec_millis()
                    ))
                    .size(40.0)
                    .monospace(),
                );
                ui.horizontal_wrapped(|ui| {
                    let toggle = ui.button(if self.stopwatch.running() {
                        "暂停秒表"
                    } else {
                        "开始 / 继续"
                    });
                    #[cfg(feature = "ui-preview")]
                    {
                        self.rects[0] = toggle.rect;
                    }
                    if toggle.clicked() {
                        self.stopwatch.toggle(now);
                    }
                    let lap = ui.add_enabled(
                        self.stopwatch.running() && self.stopwatch.laps.len() < 1024,
                        egui::Button::new("记录分段"),
                    );
                    #[cfg(feature = "ui-preview")]
                    {
                        self.rects[1] = lap.rect;
                    }
                    if lap.clicked() {
                        self.stopwatch.lap(now);
                    }
                    if ui.button("重置并清空分段").clicked() {
                        self.stopwatch.reset();
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("clock-laps")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (i, lap) in self.stopwatch.laps.iter().enumerate().rev() {
                            let previous = if i == 0 {
                                Duration::ZERO
                            } else {
                                self.stopwatch.laps[i - 1]
                            };
                            ui.monospace(format!(
                                "#{:03}    分段 {}.{:03}    总计 {}.{:03}",
                                i + 1,
                                duration_text(lap.saturating_sub(previous)),
                                (lap.saturating_sub(previous)).subsec_millis(),
                                duration_text(*lap),
                                lap.subsec_millis()
                            ));
                        }
                    });
            }
            Tab::Timers => {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.timer_name)
                            .char_limit(80)
                            .desired_width(180.0),
                    );
                    ui.label("秒");
                    ui.add(egui::DragValue::new(&mut self.timer_seconds).range(1..=604800));
                    for (seconds, label) in
                        [(60, "1分"), (300, "5分"), (1500, "25分"), (3600, "1小时")]
                    {
                        if ui.button(label).clicked() {
                            self.timer_seconds = seconds;
                        }
                    }
                    let add =
                        ui.add_enabled(self.timers.len() < 32, egui::Button::new("添加计时器"));
                    #[cfg(feature = "ui-preview")]
                    {
                        self.rects[2] = add.rect;
                    }
                    if add.clicked() {
                        self.message = self.add_timer().err().unwrap_or_default();
                    }
                });
                let mut remove = None;
                for t in &mut self.timers {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.label(&t.name);
                        ui.label(
                            egui::RichText::new(countdown_text(t.remaining(now)))
                                .size(30.0)
                                .monospace(),
                        );
                        ui.add(
                            egui::ProgressBar::new(
                                (1.0 - t.remaining(now).as_secs_f32() / t.cycle.as_secs_f32())
                                    .clamp(0.0, 1.0),
                            )
                            .text(if t.finished {
                                "已结束"
                            } else if t.running() {
                                "计时中"
                            } else {
                                "已暂停 / 未开始"
                            }),
                        );
                        ui.horizontal_wrapped(|ui| {
                            let toggle = ui.add_enabled(
                                !t.finished,
                                egui::Button::new(if t.running() {
                                    "暂停"
                                } else {
                                    "开始 / 继续"
                                }),
                            );
                            #[cfg(feature = "ui-preview")]
                            if t.id == 1 {
                                self.rects[3] = toggle.rect;
                            }
                            if toggle.clicked() {
                                t.toggle(now);
                            }
                            if ui.button("重置").clicked() {
                                t.restart();
                                self.notices.retain(|n| n.source != Source::Timer(t.id));
                            }
                            if ui.button("移除此计时器").clicked() {
                                remove = Some(t.id);
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
                if let Some(id) = remove {
                    self.timers.retain(|t| t.id != id);
                    self.notices.retain(|n| n.source != Source::Timer(id));
                }
                if self.timers.is_empty() {
                    ui.label("添加多个命名计时器，每个独立开始、暂停和重置。");
                }
            }
            Tab::Alarms => {
                ui.add(
                    egui::TextEdit::singleline(&mut self.alarm_name)
                        .char_limit(80)
                        .hint_text("闹钟名称")
                        .desired_width(280.0),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.alarm_text)
                            .char_limit(32)
                            .desired_width(200.0),
                    );
                    egui::ComboBox::from_id_salt("alarm-zone")
                        .selected_text(self.alarm_zone.name())
                        .show_ui(ui, |ui| {
                            for zone in &self.zones {
                                ui.selectable_value(&mut self.alarm_zone, *zone, zone.name());
                            }
                            ui.selectable_value(&mut self.alarm_zone, chrono_tz::UTC, "UTC");
                        });
                });
                ui.small("YYYY-MM-DD HH:MM · 按所选时区创建；时区可在世界时钟页添加。");
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.daily, "每天同一当地时间");
                    ui.checkbox(&mut self.late, "重复时间选第二次（夏令时结束）");
                    if ui
                        .add_enabled(self.alarms.len() < 64, egui::Button::new("创建闹钟"))
                        .clicked()
                    {
                        self.message = self.add_alarm(utc).err().unwrap_or_default();
                    }
                });
                ui.small("每日闹钟跳过不存在的时间。恢复后补一次提醒，不重放所有错过的日期。");
                let mut remove = None;
                for a in &mut self.alarms {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.label(&a.name);
                        ui.label(format!(
                            "{} · {}{}",
                            a.at.with_timezone(&a.zone).format("%Y-%m-%d %H:%M %:z"),
                            a.zone,
                            if a.daily { " · 每日" } else { " · 一次" }
                        ));
                        if let Some(at) = a.snooze {
                            ui.label(format!(
                                "稍后提醒：{}",
                                at.with_timezone(&a.zone).format("%H:%M:%S")
                            ));
                        }
                        ui.horizontal_wrapped(|ui| {
                            let enabled = ui.add_enabled(
                                a.daily || a.at > utc,
                                egui::Checkbox::new(&mut a.enabled, "启用"),
                            );
                            if enabled.changed() && !a.enabled {
                                a.snooze = None;
                            }
                            if ui.button("移除此闹钟").clicked() {
                                remove = Some(a.id);
                            }
                        });
                    });
                }
                if let Some(id) = remove {
                    self.alarms.retain(|a| a.id != id);
                    self.notices.retain(|n| n.source != Source::Alarm(id));
                }
            }
            Tab::Focus => {
                ui.label(format!(
                    "{} · 已完成 {} 次专注",
                    self.focus.label(),
                    self.focus.completed
                ));
                ui.label(
                    egui::RichText::new(countdown_text(self.focus.timer.remaining(now)))
                        .size(40.0)
                        .monospace(),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.focus.timer.finished,
                            egui::Button::new(if self.focus.timer.running() {
                                "暂停"
                            } else {
                                "开始 / 继续"
                            }),
                        )
                        .clicked()
                    {
                        self.focus.timer.toggle(now);
                    }
                    if ui
                        .add_enabled(
                            self.focus.timer.finished,
                            egui::Button::new("下一阶段（待开始）"),
                        )
                        .clicked()
                    {
                        self.focus.next();
                        self.notices.retain(|n| n.source != Source::Focus);
                    }
                    if ui.button("重置专注轮次").clicked() {
                        self.focus.reset();
                        self.notices.retain(|n| n.source != Source::Focus);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("专注 / 短休息 / 长休息（分钟）");
                    ui.add(egui::DragValue::new(&mut self.focus.work).range(1..=180));
                    ui.add(egui::DragValue::new(&mut self.focus.short).range(1..=60));
                    ui.add(egui::DragValue::new(&mut self.focus.long).range(1..=120));
                });
                ui.small("每4次专注进入长休息；新设置在重置或下一阶段生效。下一阶段需主动开始。");
            }
        }
        if !self.message.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.message);
        }
        ui.ctx()
            .request_repaint_after(Duration::from_millis(if self.stopwatch.running() {
                50
            } else {
                250
            }));
    }
    fn world_ui(&mut self, ui: &mut egui::Ui, utc: DateTime<Utc>) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.analog, "模拟表盘");
            if ui.button("当前时间").clicked() {
                self.meeting = 0;
            }
        });
        ui.add(egui::Slider::new(&mut self.meeting, -720..=1440).text("会议对照 · 距现在分钟"));
        ui.add(
            egui::TextEdit::singleline(&mut self.zone_query)
                .char_limit(64)
                .hint_text("添加时区：搜索北京、纽约、Tokyo…"),
        );
        if !self.zone_query.is_empty() {
            let query = self.zone_query.to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("world-zone-search")
                .max_height(150.0)
                .show(ui, |ui| {
                    for zone in chrono_tz::TZ_VARIANTS
                        .iter()
                        .filter(|z| zone_label(**z).to_lowercase().contains(&query))
                        .take(30)
                    {
                        if ui
                            .add_enabled(
                                self.zones.len() < 12 && !self.zones.contains(zone),
                                egui::Button::new(format!("添加 {}", zone_label(*zone))),
                            )
                            .clicked()
                        {
                            self.zones.push(*zone);
                        }
                    }
                });
        }
        let projected = utc + chrono::Duration::minutes(self.meeting.into());
        let mut remove = None;
        for (index, zone) in self.zones.iter().enumerate() {
            let dt = projected.with_timezone(zone);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(zone_label(*zone));
                    ui.label(format!(
                        "{} · {} · UTC{}",
                        dt.format("%Y-%m-%d"),
                        ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                            [dt.weekday().num_days_from_monday() as usize],
                        dt.format("%:z")
                    ));
                    if ui.small_button("移除").clicked() {
                        remove = Some(index);
                    }
                });
                ui.label(
                    egui::RichText::new(dt.format("%H:%M:%S").to_string())
                        .size(34.0)
                        .monospace(),
                );
                if self.analog {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(140.0, 140.0), egui::Sense::hover());
                    let center = rect.center();
                    let color = ui.visuals().text_color();
                    let p = ui.painter();
                    p.circle_stroke(center, 64.0, egui::Stroke::new(1.0, color));
                    for tick in 0..12 {
                        let angle = tick as f32 * std::f32::consts::TAU / 12.0
                            - std::f32::consts::FRAC_PI_2;
                        let d = egui::vec2(angle.cos(), angle.sin());
                        p.line_segment(
                            [center + d * 58.0, center + d * 63.0],
                            egui::Stroke::new(1.0, color),
                        );
                    }
                    for (value, scale, length, width) in [
                        (
                            (dt.hour() % 12) as f32 + dt.minute() as f32 / 60.0,
                            12.0,
                            36.0,
                            3.0,
                        ),
                        (
                            dt.minute() as f32 + dt.second() as f32 / 60.0,
                            60.0,
                            52.0,
                            2.0,
                        ),
                        (dt.second() as f32, 60.0, 58.0, 1.0),
                    ] {
                        let angle =
                            value / scale * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        p.line_segment(
                            [
                                center,
                                center + egui::vec2(angle.cos(), angle.sin()) * length,
                            ],
                            egui::Stroke::new(width, color),
                        );
                    }
                }
            });
            ui.add_space(6.0);
        }
        if let Some(index) = remove {
            self.zones.remove(index);
        }
        ui.small(format!(
            "内置时区数据 {}；不联网校时，时间来自本机。会议偏移只改变对照，不设置闹钟。",
            chrono_tz::IANA_TZDB_VERSION
        ));
    }
    pub fn notice_ui(&mut self, ctx: &egui::Context) {
        if self.notices.is_empty() {
            return;
        }
        let mut dismiss = None;
        let mut snooze = None;
        egui::Window::new("时钟提醒")
            .id(egui::Id::new("clock-notices"))
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label("提醒保持至确认；本轮暂不播放声音。");
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for n in &self.notices {
                            ui.group(|ui| {
                                ui.label(&n.title);
                                ui.horizontal(|ui| {
                                    if ui.button("已知晓").clicked() {
                                        dismiss = Some(n.source);
                                    }
                                    if n.source != Source::Focus
                                        && ui.button("5分钟后再提醒").clicked()
                                    {
                                        snooze = Some(n.source);
                                    }
                                });
                            });
                        }
                    });
            });
        if let Some(source) = dismiss {
            self.notices.retain(|n| n.source != source);
        }
        if let Some(source) = snooze {
            self.snooze(source, Utc::now());
        }
    }
}

fn zone_label(zone: Tz) -> String {
    let city = match zone.name() {
        "Asia/Shanghai" => "北京 / 上海",
        "Asia/Tokyo" => "东京",
        "Europe/London" => "伦敦",
        "America/New_York" => "纽约",
        "America/Los_Angeles" => "洛杉矶",
        "Asia/Kolkata" => "印度",
        "Asia/Singapore" => "新加坡",
        "Australia/Sydney" => "悉尼",
        "Europe/Paris" => "巴黎",
        "Europe/Berlin" => "柏林",
        _ => "",
    };
    if city.is_empty() {
        zone.name().into()
    } else {
        format!("{city} · {}", zone.name())
    }
}
