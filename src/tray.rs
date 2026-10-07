use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::recorder_ui::TrayRecordingStatus;
use crate::{config::ServiceSpec, service::ServiceStatus, tools::ToolKind};

const OPEN_ID: &str = "app.open";
const START_ALL_ID: &str = "service.start-all";
const STOP_ALL_ID: &str = "service.stop-all";
const EXIT_ID: &str = "app.exit";
const START_PREFIX: &str = "service.start.";
const STOP_PREFIX: &str = "service.stop.";
const RESTART_PREFIX: &str = "service.restart.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayTool {
    Small(ToolKind),
    Http,
    Diff,
    Network,
    Data,
    Files,
    Plugins,
    Integrations,
    Framework(crate::framework::Tool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    QuickPanel,
    ContextPanel,
    Search,
    Settings,
    OpenEntry(String),
    Collection(String),
    OpenTool(TrayTool),
    RecorderTogglePause,
    RecorderStop,
    Start(String),
    Stop(String),
    Restart(String),
    StartAll,
    StopAll,
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayEntry {
    pub id: String,
    pub title: String,
    pub category: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigation {
    entries: Vec<TrayEntry>,
    favorites: Vec<TrayEntry>,
    recent: Vec<TrayEntry>,
    frequent: Vec<TrayEntry>,
}
impl Navigation {
    pub fn new(mut entries: Vec<TrayEntry>, preferences: &crate::preferences::Preferences) -> Self {
        entries.sort_by(|a, b| (&a.category, &a.title, &a.id).cmp(&(&b.category, &b.title, &b.id)));
        let selected = |ids: &[String]| {
            let mut seen = std::collections::HashSet::new();
            ids.iter()
                .filter(|id| seen.insert(id.as_str()))
                .filter_map(|id| entries.iter().find(|e| &e.id == id).cloned())
                .collect()
        };
        let favorites = selected(&preferences.favorites);
        let recent = selected(&preferences.recent);
        let mut frequent: Vec<_> = entries
            .iter()
            .filter(|e| preferences.usage.get(&e.id).copied().unwrap_or(0) > 0)
            .cloned()
            .collect();
        frequent.sort_by_key(|e| {
            (
                std::cmp::Reverse(preferences.usage[&e.id]),
                preferences
                    .recent
                    .iter()
                    .position(|id| id == &e.id)
                    .unwrap_or(usize::MAX),
            )
        });
        Self {
            favorites,
            recent,
            frequent,
            entries,
        }
    }
}

// Escape native menu mnemonic markers and keep untrusted plugin names on one line.
fn menu_label(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let label = if clean.chars().count() > 36 {
        format!("{}…", clean.chars().take(35).collect::<String>())
    } else {
        clean
    };
    label.replace('&', "&&")
}
fn entry_item(entry: &TrayEntry) -> MenuItem {
    MenuItem::with_id(
        format!("entry.{}", entry.id),
        menu_label(&entry.title),
        true,
        None,
    )
}
fn clear_menu(menu: &Submenu) -> Result<()> {
    while menu.remove_at(0).is_some() {}
    Ok(())
}
fn fill_shortcuts(
    menu: &Submenu,
    name: &str,
    entries: &[TrayEntry],
    limit: usize,
    empty: &str,
) -> Result<()> {
    clear_menu(menu)?;
    menu.set_text(format!(
        "{}{}  ·  {}",
        if name == "收藏" { "★  " } else { "" },
        name,
        entries.len()
    ));
    if entries.is_empty() {
        menu.append(&MenuItem::new(empty, false, None))?;
    }
    for entry in entries.iter().take(limit) {
        menu.append(&entry_item(entry))?;
    }
    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(
            format!("collection.{name}"),
            format!("查看全部{name}…"),
            true,
            None,
        ),
    ])?;
    Ok(())
}

pub struct TrayController {
    _icon: TrayIcon,
    _menu: Menu,
    navigation: Submenu,
    favorites: Submenu,
    recent: Submenu,
    frequent: Submenu,
    snapshot: Option<Navigation>,
    start_all: MenuItem,
    stop_all: MenuItem,
    summary: MenuItem,
    recorder_status: MenuItem,
    recorder_pause: MenuItem,
    recorder_stop: MenuItem,
    recorder_snapshot: TrayRecordingStatus,
    service_menus: HashMap<String, Submenu>,
    status_items: HashMap<String, MenuItem>,
    start_items: HashMap<String, MenuItem>,
    restart_items: HashMap<String, MenuItem>,
    stop_items: HashMap<String, MenuItem>,
}

impl TrayController {
    pub fn new<'a>(services: impl Iterator<Item = &'a ServiceSpec>) -> Result<Self> {
        let menu = Menu::new();
        let title = MenuItem::new(
            format!("Zi DevTools  ·  v{}", env!("CARGO_PKG_VERSION")),
            false,
            None,
        );
        let open = MenuItem::with_id(OPEN_ID, "打开主窗口", true, None);
        let quick = MenuItem::with_id("app.quick", "快捷面板…", true, None);
        let search = MenuItem::with_id("app.search", "搜索工具…", true, None);
        let recorder_menu = Submenu::new("屏幕录制", true);
        let recorder_status = MenuItem::new("尚未录制", false, None);
        let recorder_pause = MenuItem::with_id("recorder.pause", "暂停录制", false, None);
        let recorder_stop = MenuItem::with_id("recorder.stop", "停止并保存", false, None);
        recorder_menu.append_items(&[
            &MenuItem::with_id("entry.screen-recorder", "打开录屏工具…", true, None),
            &PredefinedMenuItem::separator(),
            &recorder_status,
            &recorder_pause,
            &recorder_stop,
        ])?;
        let summary = MenuItem::new("正在读取服务状态…", false, None);
        let favorites = Submenu::new("★  收藏", true);
        let recent = Submenu::new("最近使用", true);
        let frequent = Submenu::new("常用工具", true);
        let navigation = Submenu::new("全部工具", true);
        menu.append_items(&[
            &title,
            &PredefinedMenuItem::separator(),
            &open,
            &quick,
            &search,
            &recorder_menu,
            &PredefinedMenuItem::separator(),
            &favorites,
            &recent,
            &frequent,
            &navigation,
            &PredefinedMenuItem::separator(),
        ])?;

        let services_menu = Submenu::new("本地服务", true);
        services_menu.append_items(&[&summary, &PredefinedMenuItem::separator()])?;

        let mut service_menus = HashMap::new();
        let mut status_items = HashMap::new();
        let mut start_items = HashMap::new();
        let mut restart_items = HashMap::new();
        let mut stop_items = HashMap::new();
        for service in services {
            let submenu = Submenu::new(format!("○ {} · 检查中", service.name), true);
            let status = MenuItem::new("状态：检查中", false, None);
            let separator = PredefinedMenuItem::separator();
            let start =
                MenuItem::with_id(format!("{START_PREFIX}{}", service.id), "启动", true, None);
            let restart = MenuItem::with_id(
                format!("{RESTART_PREFIX}{}", service.id),
                "重启",
                false,
                None,
            );
            let stop =
                MenuItem::with_id(format!("{STOP_PREFIX}{}", service.id), "停止", false, None);
            submenu.append_items(&[&status, &separator, &start, &restart, &stop])?;
            services_menu.append(&submenu)?;
            service_menus.insert(service.id.clone(), submenu);
            status_items.insert(service.id.clone(), status);
            start_items.insert(service.id.clone(), start);
            restart_items.insert(service.id.clone(), restart);
            stop_items.insert(service.id.clone(), stop);
        }

        let separator = PredefinedMenuItem::separator();
        let start_all = MenuItem::with_id(START_ALL_ID, "启动可用服务", false, None);
        let stop_all = MenuItem::with_id(STOP_ALL_ID, "停止托管服务", false, None);
        let exit = MenuItem::with_id(EXIT_ID, "退出", true, None);
        services_menu.append_items(&[&separator, &start_all, &stop_all])?;
        menu.append_items(&[
            &services_menu,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("app.settings", "设置…", true, None),
            &exit,
        ])?;

        let icon = TrayIconBuilder::new()
            .with_menu_on_left_click(false)
            .with_tooltip(format!("Zi DevTools v{}", env!("CARGO_PKG_VERSION")))
            .with_icon(make_icon()?)
            .build()
            .context("创建 Windows 托盘图标失败")?;
        Ok(Self {
            _icon: icon,
            _menu: menu,
            navigation,
            favorites,
            recent,
            frequent,
            snapshot: None,
            start_all,
            stop_all,
            summary,
            recorder_status,
            recorder_pause,
            recorder_stop,
            recorder_snapshot: TrayRecordingStatus::Idle,
            service_menus,
            status_items,
            start_items,
            restart_items,
            stop_items,
        })
    }

    pub fn sync_navigation(&mut self, model: Navigation) -> Result<()> {
        if self.snapshot.as_ref() == Some(&model) {
            return Ok(());
        }
        fill_shortcuts(
            &self.favorites,
            "收藏",
            &model.favorites,
            8,
            "在工具首页点击 ☆ 添加收藏",
        )?;
        fill_shortcuts(
            &self.recent,
            "最近",
            &model.recent,
            8,
            "打开工具后显示在这里",
        )?;
        fill_shortcuts(
            &self.frequent,
            "常用",
            &model.frequent,
            6,
            "按工具打开次数排序",
        )?;
        clear_menu(&self.navigation)?;
        self.navigation
            .set_text(format!("全部工具  ·  {}", model.entries.len()));
        let mut categories: BTreeMap<&str, Vec<&TrayEntry>> = BTreeMap::new();
        for entry in &model.entries {
            categories.entry(&entry.category).or_default().push(entry);
        }
        for (category, entries) in categories {
            let submenu = Submenu::new(menu_label(category), true);
            for entry in entries {
                submenu.append(&entry_item(entry))?;
            }
            self.navigation.append(&submenu)?;
        }
        self.snapshot = Some(model);
        Ok(())
    }

    pub fn update_recorder(&mut self, state: TrayRecordingStatus) {
        if self.recorder_snapshot == state {
            return;
        }
        self.recorder_snapshot = state;
        let (label, pause_label, stop_label, can_pause, can_stop) = match state {
            TrayRecordingStatus::Idle => ("尚未录制", "暂停录制", "停止并保存", false, false),
            TrayRecordingStatus::Countdown => ("倒计时中", "暂停录制", "取消倒计时", false, true),
            TrayRecordingStatus::Starting => ("正在启动…", "暂停录制", "停止并保存", false, true),
            TrayRecordingStatus::Recording => ("● 正在录制", "暂停录制", "停止并保存", true, true),
            TrayRecordingStatus::Paused => ("Ⅱ 已暂停", "继续录制", "停止并保存", true, true),
            TrayRecordingStatus::Saving => {
                ("正在保存 MP4…", "暂停录制", "停止并保存", false, false)
            }
        };
        self.recorder_status.set_text(label);
        self.recorder_pause.set_text(pause_label);
        self.recorder_stop.set_text(stop_label);
        self.recorder_pause.set_enabled(can_pause);
        self.recorder_stop.set_enabled(can_stop);
    }

    pub fn poll_actions() -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    ..
                } => actions.push(TrayAction::ContextPanel),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => actions.push(TrayAction::QuickPanel),
                _ => {}
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id.0;
            let action = action_from_menu_id(&id);
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }

    pub fn update_status(&self, statuses: &[ServiceStatus]) {
        let running = statuses.iter().filter(|status| status.managed).count();
        let external = statuses
            .iter()
            .filter(|status| !status.managed && status.state.is_available())
            .count();
        self.summary.set_text(format!(
            "托管 {running} · 外部/占用 {external} · 共 {} 项",
            statuses.len()
        ));
        self.start_all
            .set_enabled(statuses.iter().any(|s| !s.state.is_available()));
        self.stop_all
            .set_enabled(statuses.iter().any(|s| s.managed));
        for status in statuses {
            if let Some(submenu) = self.service_menus.get(&status.id) {
                let dot = if status.state.is_available() {
                    "●"
                } else {
                    "○"
                };
                submenu.set_text(format!(
                    "{dot} {} · {}",
                    status.name,
                    status.display_state()
                ));
            }
            if let Some(item) = self.status_items.get(&status.id) {
                let detail = status
                    .pid
                    .map(|pid| format!(" · PID {pid}"))
                    .unwrap_or_default();
                item.set_text(format!("状态：{}{detail}", status.display_state()));
            }
            if let Some(item) = self.start_items.get(&status.id) {
                item.set_enabled(!status.state.is_available());
            }
            if let Some(item) = self.stop_items.get(&status.id) {
                item.set_enabled(status.managed);
            }
            if let Some(item) = self.restart_items.get(&status.id) {
                item.set_enabled(status.managed);
            }
        }
    }
}

fn action_from_menu_id(id: &str) -> Option<TrayAction> {
    if let Some(entry) = id.strip_prefix("entry.").filter(|s| !s.is_empty()) {
        return Some(TrayAction::OpenEntry(entry.to_owned()));
    }
    if let Some(collection) = id
        .strip_prefix("collection.")
        .filter(|s| ["收藏", "最近", "常用"].contains(s))
    {
        return Some(TrayAction::Collection(collection.to_owned()));
    }
    if let Some(tool) = id
        .strip_prefix("tool.framework.")
        .and_then(crate::framework::Tool::from_id)
    {
        return Some(TrayAction::OpenTool(TrayTool::Framework(tool)));
    }
    match id {
        "app.quick" => Some(TrayAction::QuickPanel),
        "app.search" => Some(TrayAction::Search),
        "app.settings" => Some(TrayAction::Settings),
        "recorder.pause" => Some(TrayAction::RecorderTogglePause),
        "recorder.stop" => Some(TrayAction::RecorderStop),
        OPEN_ID => Some(TrayAction::ShowWindow),
        "tool.http" => Some(TrayAction::OpenTool(TrayTool::Http)),
        "tool.diff" => Some(TrayAction::OpenTool(TrayTool::Diff)),
        "tool.network" => Some(TrayAction::OpenTool(TrayTool::Network)),
        "tool.data" => Some(TrayAction::OpenTool(TrayTool::Data)),
        "tool.files" => Some(TrayAction::OpenTool(TrayTool::Files)),
        "tool.plugins" => Some(TrayAction::OpenTool(TrayTool::Plugins)),
        "tool.integrations" => Some(TrayAction::OpenTool(TrayTool::Integrations)),
        START_ALL_ID => Some(TrayAction::StartAll),
        STOP_ALL_ID => Some(TrayAction::StopAll),
        EXIT_ID => Some(TrayAction::Exit),
        _ => id
            .strip_prefix("tool.small.")
            .and_then(tool_from_slug)
            .map(|kind| TrayAction::OpenTool(TrayTool::Small(kind)))
            .or_else(|| {
                id.strip_prefix(START_PREFIX)
                    .map(|service| TrayAction::Start(service.to_owned()))
                    .or_else(|| {
                        id.strip_prefix(RESTART_PREFIX)
                            .map(|service| TrayAction::Restart(service.to_owned()))
                    })
                    .or_else(|| {
                        id.strip_prefix(STOP_PREFIX)
                            .map(|service| TrayAction::Stop(service.to_owned()))
                    })
            }),
    }
}

fn tool_slug(kind: ToolKind) -> &'static str {
    kind.id()
}

fn tool_from_slug(slug: &str) -> Option<ToolKind> {
    ToolKind::ALL
        .into_iter()
        .find(|kind| tool_slug(*kind) == slug)
}

pub fn rgba_icon() -> (Vec<u8>, u32, u32) {
    const SIZE: usize = 32;
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - (SIZE as f32 - 1.0) / 2.0;
            let dy = y as f32 - (SIZE as f32 - 1.0) / 2.0;
            let radius = (dx * dx + dy * dy).sqrt();
            let pixel = if radius > 15.0 {
                [0, 0, 0, 0]
            } else if radius > 12.6 {
                [92, 111, 255, 255]
            } else if (9..=22).contains(&x)
                && ((9..=11).contains(&y)
                    || (20..=22).contains(&y)
                    || ((11..=20).contains(&y) && (30..=34).contains(&(x + y))))
            {
                [237, 242, 255, 255]
            } else {
                [26, 35, 63, 255]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    (rgba, SIZE as u32, SIZE as u32)
}

fn make_icon() -> Result<Icon> {
    let (rgba, width, height) = rgba_icon();
    Icon::from_rgba(rgba, width, height).context("生成托盘图标失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> TrayEntry {
        TrayEntry {
            id: id.into(),
            title: id.into(),
            category: "测试".into(),
        }
    }

    #[test]
    fn shortcuts_follow_history_and_filter_disabled_plugins() {
        let mut prefs = crate::preferences::Preferences::default();
        prefs
            .favorites
            .extend(["plugin:off/x", "b", "a", "b"].map(str::to_owned));
        prefs.visit("a");
        prefs.visit("a");
        prefs.visit("plugin:on/x");
        prefs.visit("b");
        let model = Navigation::new(vec![entry("a"), entry("b"), entry("plugin:on/x")], &prefs);
        assert_eq!(
            model
                .favorites
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(
            model
                .recent
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "plugin:on/x", "a"]
        );
        assert_eq!(
            model
                .frequent
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "plugin:on/x"]
        );
        assert_eq!(
            action_from_menu_id("entry.plugin:on/x"),
            Some(TrayAction::OpenEntry("plugin:on/x".into()))
        );
        assert_eq!(
            action_from_menu_id("collection.收藏"),
            Some(TrayAction::Collection("收藏".into()))
        );
        assert_eq!(action_from_menu_id("entry."), None);
        assert_eq!(action_from_menu_id("collection.unknown"), None);
    }

    #[test]
    fn native_labels_escape_mnemonics_and_control_characters() {
        assert_eq!(menu_label("A&B\tC\nD"), "A&&B C D");
        assert_eq!(menu_label(&"长".repeat(50)).chars().count(), 36);
    }

    #[cfg(windows)]
    #[test]
    fn native_shortcuts_are_bounded_and_refresh_without_stale_items() {
        let menu = Submenu::new("收藏", true);
        let entries: Vec<_> = (0..12).map(|n| entry(&format!("tool-{n}"))).collect();
        fill_shortcuts(&menu, "收藏", &entries, 8, "empty").unwrap();
        assert_eq!(menu.items().len(), 10); // eight tools, separator, full collection
        assert_eq!(menu.items()[0].id().0, "entry.tool-0");
        fill_shortcuts(&menu, "收藏", &[], 8, "empty").unwrap();
        assert_eq!(menu.items().len(), 3);
        assert_eq!(menu.items()[2].id().0, "collection.收藏");
    }

    #[test]
    fn maps_each_service_menu_item_to_its_own_action() {
        assert_eq!(
            action_from_menu_id("recorder.pause"),
            Some(TrayAction::RecorderTogglePause)
        );
        assert_eq!(
            action_from_menu_id("recorder.stop"),
            Some(TrayAction::RecorderStop)
        );
        assert_eq!(
            action_from_menu_id("service.start.api.dev"),
            Some(TrayAction::Start("api.dev".to_owned()))
        );
        assert_eq!(
            action_from_menu_id("service.restart.api.dev"),
            Some(TrayAction::Restart("api.dev".to_owned()))
        );
        assert_eq!(
            action_from_menu_id("service.stop.api.dev"),
            Some(TrayAction::Stop("api.dev".to_owned()))
        );
        assert_eq!(
            action_from_menu_id("service.start-all"),
            Some(TrayAction::StartAll)
        );
        assert_eq!(action_from_menu_id("unknown"), None);
    }

    #[test]
    fn maps_every_tool_menu_item_to_its_page() {
        for kind in ToolKind::ALL {
            assert_eq!(
                action_from_menu_id(&format!("tool.small.{}", tool_slug(kind))),
                Some(TrayAction::OpenTool(TrayTool::Small(kind)))
            );
        }
        assert_eq!(
            action_from_menu_id("tool.http"),
            Some(TrayAction::OpenTool(TrayTool::Http))
        );
        assert_eq!(
            action_from_menu_id("tool.diff"),
            Some(TrayAction::OpenTool(TrayTool::Diff))
        );
        assert_eq!(
            action_from_menu_id("tool.network"),
            Some(TrayAction::OpenTool(TrayTool::Network))
        );
        for (id, tool) in [
            ("tool.data", TrayTool::Data),
            ("tool.files", TrayTool::Files),
            ("tool.plugins", TrayTool::Plugins),
            ("tool.integrations", TrayTool::Integrations),
        ] {
            assert_eq!(action_from_menu_id(id), Some(TrayAction::OpenTool(tool)));
        }
        assert_eq!(action_from_menu_id("tool.small.unknown"), None);
    }

    #[test]
    fn separates_encoding_tools_from_general_tools() {
        assert!(ToolKind::ALL.iter().any(|kind| kind.is_encoding()));
        assert!(ToolKind::ALL.iter().any(|kind| !kind.is_encoding()));
        assert!(!ToolKind::CaseConvert.is_encoding());
    }
}
