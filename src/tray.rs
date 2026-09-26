use std::collections::HashMap;

use anyhow::{Context, Result};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::{config::ServiceSpec, service::ServiceStatus, tools::ToolKind};

const OPEN_ID: &str = "app.open";
const START_ALL_ID: &str = "service.start-all";
const STOP_ALL_ID: &str = "service.stop-all";
const EXIT_ID: &str = "app.exit";
const START_PREFIX: &str = "service.start.";
const STOP_PREFIX: &str = "service.stop.";
const RESTART_PREFIX: &str = "service.restart.";
const TOOL_PREFIX: &str = "tool.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayTool {
    Small(ToolKind),
    Http,
    Diff,
    Network,
    Data,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    OpenTool(TrayTool),
    Start(String),
    Stop(String),
    Restart(String),
    StartAll,
    StopAll,
    Exit,
}

pub struct TrayController {
    _icon: TrayIcon,
    summary: MenuItem,
    service_menus: HashMap<String, Submenu>,
    status_items: HashMap<String, MenuItem>,
    start_items: HashMap<String, MenuItem>,
    restart_items: HashMap<String, MenuItem>,
    stop_items: HashMap<String, MenuItem>,
}

impl TrayController {
    pub fn new<'a>(services: impl Iterator<Item = &'a ServiceSpec>) -> Result<Self> {
        let menu = Menu::new();
        let open = MenuItem::with_id(
            OPEN_ID,
            format!("打开 Zi DevTools v{}", env!("CARGO_PKG_VERSION")),
            true,
            None,
        );
        let summary = MenuItem::new("正在读取服务状态…", false, None);
        menu.append_items(&[&open, &PredefinedMenuItem::separator()])?;

        let small_tools = Submenu::new("小工具", true);
        for kind in ToolKind::ALL.into_iter().filter(|kind| !kind.is_encoding()) {
            small_tools.append(&MenuItem::with_id(
                format!("{TOOL_PREFIX}small.{}", tool_slug(kind)),
                kind.label(),
                true,
                None,
            ))?;
        }
        let encoding_tools = Submenu::new("编码工具", true);
        for kind in ToolKind::ALL.into_iter().filter(|kind| kind.is_encoding()) {
            encoding_tools.append(&MenuItem::with_id(
                format!("{TOOL_PREFIX}small.{}", tool_slug(kind)),
                kind.label(),
                true,
                None,
            ))?;
        }
        let developer_tools = Submenu::new("开发工具", true);
        developer_tools.append_items(&[
            &MenuItem::with_id("tool.http", "HTTP 请求调试", true, None),
            &MenuItem::with_id("tool.diff", "文本差异对比", true, None),
            &MenuItem::with_id("tool.network", "网络诊断", true, None),
            &MenuItem::with_id("tool.data", "数据工作台", true, None),
            &MenuItem::with_id("tool.files", "文件校验", true, None),
        ])?;
        menu.append_items(&[&small_tools, &encoding_tools, &developer_tools])?;

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
        let start_all = MenuItem::with_id(START_ALL_ID, "全部启动", true, None);
        let stop_all = MenuItem::with_id(STOP_ALL_ID, "全部停止", true, None);
        let exit = MenuItem::with_id(EXIT_ID, "退出", true, None);
        services_menu.append_items(&[&separator, &start_all, &stop_all])?;
        menu.append_items(&[&services_menu, &PredefinedMenuItem::separator(), &exit])?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(format!("Zi DevTools v{}", env!("CARGO_PKG_VERSION")))
            .with_icon(make_icon()?)
            .build()
            .context("创建 Windows 托盘图标失败")?;
        Ok(Self {
            _icon: icon,
            summary,
            service_menus,
            status_items,
            start_items,
            restart_items,
            stop_items,
        })
    }

    pub fn poll_actions() -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => actions.push(TrayAction::ShowWindow),
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
    match id {
        OPEN_ID => Some(TrayAction::ShowWindow),
        "tool.http" => Some(TrayAction::OpenTool(TrayTool::Http)),
        "tool.diff" => Some(TrayAction::OpenTool(TrayTool::Diff)),
        "tool.network" => Some(TrayAction::OpenTool(TrayTool::Network)),
        "tool.data" => Some(TrayAction::OpenTool(TrayTool::Data)),
        "tool.files" => Some(TrayAction::OpenTool(TrayTool::Files)),
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

    #[test]
    fn maps_each_service_menu_item_to_its_own_action() {
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
