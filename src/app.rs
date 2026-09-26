use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, Color32, RichText};
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use sysinfo::{ProcessesToUpdate, System};

use crate::{
    config::load_config,
    dev_tools::{
        DiffState, HTTP_METHODS, HttpPane, HttpRequestSpec, HttpWorkbenchState, compare_text,
        default_http_storage_path, execute_http,
    },
    network_tools::{NetworkPane, NetworkState, resolve_host, test_tcp},
    preferences::{self, Preferences},
    service::{ActionResult, ServiceManager, ServiceState, ServiceStatus},
    tools::{ToolKind, ToolState, generate_qr, generate_uuid, run_tool},
    tray::{Navigation, TrayAction, TrayController, TrayEntry, TrayTool},
    workbench::{DataState, FileState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Theme {
    Dark,
    Light,
}

impl Theme {
    fn label(self) -> &'static str {
        match self {
            Self::Dark => "暗主题",
            Self::Light => "亮主题",
        }
    }
}

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    panel: Color32,
    card: Color32,
    input_bg: Color32,
    surface: Color32,
    accent: Color32,
    accent_hover: Color32,
    green: Color32,
    amber: Color32,
    red: Color32,
    text: Color32,
    muted: Color32,
}

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            bg: Color32::from_rgb(15, 20, 29),
            panel: Color32::from_rgb(24, 31, 43),
            card: Color32::from_rgb(31, 40, 55),
            input_bg: Color32::from_rgb(11, 16, 24),
            surface: Color32::from_rgb(39, 50, 68),
            accent: Color32::from_rgb(61, 137, 218),
            accent_hover: Color32::from_rgb(78, 157, 234),
            green: Color32::from_rgb(59, 201, 134),
            amber: Color32::from_rgb(244, 180, 66),
            red: Color32::from_rgb(239, 99, 105),
            text: Color32::from_rgb(232, 237, 245),
            muted: Color32::from_rgb(166, 178, 196),
        },
        Theme::Light => Palette {
            bg: Color32::from_rgb(244, 247, 251),
            panel: Color32::from_rgb(232, 237, 244),
            card: Color32::from_rgb(255, 255, 255),
            input_bg: Color32::from_rgb(248, 250, 253),
            surface: Color32::from_rgb(220, 228, 238),
            accent: Color32::from_rgb(42, 104, 180),
            accent_hover: Color32::from_rgb(28, 83, 155),
            green: Color32::from_rgb(29, 139, 88),
            amber: Color32::from_rgb(166, 105, 0),
            red: Color32::from_rgb(190, 47, 55),
            text: Color32::from_rgb(31, 42, 56),
            muted: Color32::from_rgb(91, 105, 124),
        },
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Home,
    Services,
    SmallTools,
    EncodingTools,
    Http,
    Diff,
    Network,
    Data,
    Files,
    Settings,
    Plugins,
    Integrations,
    Frameworks,
}

struct ToolEntry {
    id: String,
    title: String,
    description: String,
    category: String,
    keywords: String,
    page: Page,
    kind: Option<ToolKind>,
}
impl ToolEntry {
    fn score(&self, query: &str) -> Option<u32> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Some(0);
        }
        let haystack = format!(
            "{} {} {} {} {}",
            self.id, self.title, self.description, self.category, self.keywords
        )
        .to_lowercase();
        if !query.split_whitespace().all(|word| haystack.contains(word)) {
            return None;
        }
        Some(if self.id == query || self.title.to_lowercase() == query {
            1000
        } else if self.title.to_lowercase().starts_with(&query) {
            500
        } else {
            100
        })
    }
}
fn page_bounds(total: usize, index: usize) -> (std::ops::Range<usize>, usize) {
    const SIZE: usize = 18;
    let pages = total.div_ceil(SIZE).max(1);
    let start = index.min(pages - 1) * SIZE;
    (start..(start + SIZE).min(total), pages)
}
fn tool_category(id: &str) -> &'static str {
    if let Some(tool) = crate::framework::Tool::from_id(id) {
        return tool.category();
    }
    match id {
        "data" | "json" | "json-path" | "json-diff" | "data-schema" | "yaml" => "数据与格式",
        "http" | "network" | "url" | "url-inspect" | "cidr" | "jwt" => "网络与接口",
        "files" | "services" => "文件与系统",
        "timestamp" | "uuid" | "random" | "cron" | "number" | "qr" | "color" => "时间与生成",
        "java-trace" => "Java 与 JVM",
        "django-trace" => "Python 与 Django",
        "plugins" | "integrations" => "扩展与集成",
        _ => "文本与编码",
    }
}
fn catalog() -> Vec<ToolEntry> {
    let mut entries = vec![
        ToolEntry {
            id: "data".into(),
            title: "数据工作台".into(),
            description: "CSV / JSON 筛选、排序与导出".into(),
            page: Page::Data,
            kind: None,
            category: String::new(),
            keywords: String::new(),
        },
        ToolEntry {
            id: "files".into(),
            title: "批量文件校验".into(),
            description: "SHA-256 / SHA-512 摘要对比".into(),
            page: Page::Files,
            kind: None,
            category: String::new(),
            keywords: String::new(),
        },
        ToolEntry {
            id: "http".into(),
            title: "HTTP 请求调试".into(),
            description: "请求标签、站点与历史".into(),
            page: Page::Http,
            kind: None,
            category: String::new(),
            keywords: String::new(),
        },
        ToolEntry {
            id: "diff".into(),
            title: "文本差异对比".into(),
            description: "逐行比较两段文本".into(),
            page: Page::Diff,
            kind: None,
            category: String::new(),
            keywords: String::new(),
        },
        ToolEntry {
            id: "network".into(),
            title: "网络诊断".into(),
            description: "DNS 解析与单端口 TCP 连通性".into(),
            page: Page::Network,
            kind: None,
            category: String::new(),
            keywords: String::new(),
        },
        ToolEntry {
            id: "plugins".into(),
            title: "插件中心".into(),
            description: "安装、启停与管理工具连接器".into(),
            page: Page::Plugins,
            kind: None,
            category: String::new(),
            keywords: "plugin extension LLM MCP 扩展".into(),
        },
        ToolEntry {
            id: "integrations".into(),
            title: "本机集成发现".into(),
            description: "发现模型与开发软件入口及本地服务端口".into(),
            page: Page::Integrations,
            kind: None,
            category: String::new(),
            keywords: "Ollama Codex Docker Python Node 软件".into(),
        },
        ToolEntry {
            id: "services".into(),
            title: "本地服务".into(),
            description: "服务启停、健康检查与日志".into(),
            page: Page::Services,
            kind: None,
            category: String::new(),
            keywords: "service server process".into(),
        },
    ];
    entries.extend(
        crate::framework::Tool::ALL
            .into_iter()
            .map(|tool| ToolEntry {
                id: tool.id().into(),
                title: tool.label().into(),
                description: tool.description().into(),
                category: tool.category().into(),
                keywords: format!(
                    "{} {} {} {}",
                    tool.category(),
                    tool.id(),
                    tool.description(),
                    tool.help()
                ),
                page: Page::Frameworks,
                kind: None,
            }),
    );
    entries.extend(ToolKind::ALL.into_iter().map(|kind| ToolEntry {
        id: kind.id().into(),
        title: kind.label().into(),
        description: kind.description().into(),
        page: if kind.is_encoding() {
            Page::EncodingTools
        } else {
            Page::SmallTools
        },
        kind: Some(kind),
        category: String::new(),
        keywords: String::new(),
    }));
    for e in &mut entries {
        e.category = tool_category(&e.id).into();
        e.keywords.push_str(match e.id.as_str() {
            "java-trace" => " java jvm spring exception stacktrace 异常 堆栈",
            "django-trace" => " django python traceback error 异常 堆栈",
            "json" => " format validate minify 格式 校验 压缩",
            "data" => " csv tsv table filter sort 表格 筛选 排序",
            "files" => " hash checksum file 文件 摘要",
            "diff" | "json-diff" => " compare difference 比较 差异",
            "regex" => " regex match 正则 匹配",
            "timestamp" => " time date 时间 日期",
            _ => "",
        });
    }
    entries
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ServiceFilter {
    #[default]
    All,
    Managed,
    External,
    Stopped,
}

impl ServiceFilter {
    const ALL: [Self; 4] = [Self::All, Self::Managed, Self::External, Self::Stopped];

    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Managed => "托管运行",
            Self::External => "外部占用",
            Self::Stopped => "已停止",
        }
    }

    fn matches(self, state: ServiceState, managed: bool) -> bool {
        match self {
            Self::All => true,
            Self::Managed => managed,
            Self::External => !managed && state.is_available(),
            Self::Stopped => state == ServiceState::Stopped,
        }
    }
}

enum BackgroundEvent {
    Statuses(Vec<ServiceStatus>),
    Action(Result<ActionResult, String>),
    Logs(String, Result<String, String>),
    ConfigPreview(String, Result<String, String>),
    RestoreFinished(String),
    NavigateTool(TrayTool),
    TrayNavigate(TrayAction),
    NetworkResult(u64, Result<String, String>),
    HttpResult {
        tab_id: u64,
        method: String,
        url: String,
        duration_ms: u128,
        result: Result<String, String>,
    },
}

pub struct DevToolsApp {
    manager: Arc<ServiceManager>,
    tray: Option<TrayController>,
    page: Page,
    statuses: Vec<ServiceStatus>,
    selected_service: Option<String>,
    search: String,
    service_filter: ServiceFilter,
    event_tx: Sender<BackgroundEvent>,
    event_rx: Receiver<BackgroundEvent>,
    refresh_inflight: bool,
    last_refresh: Instant,
    notification: String,
    notification_error: bool,
    startup_warning: Option<String>,
    log_view: Option<String>,
    log_text: String,
    config_view: Option<(String, String)>,
    config_text: String,
    tool_state: ToolState,
    http_state: HttpWorkbenchState,
    diff_state: DiffState,
    network_state: NetworkState,
    qr_texture: Option<egui::TextureHandle>,
    config_path_input: String,
    config_error: Option<String>,
    quit_requested: bool,
    tray_bridge_stop: Arc<AtomicBool>,
    tray_exit_requested: Arc<AtomicBool>,
    window_handle: Option<isize>,
    theme: Theme,
    colors: Palette,
    preferences: Preferences,
    preferences_path: PathBuf,
    tool_search: String,
    launcher_query: String,
    launcher_open: bool,
    launcher_focus: bool,
    launcher_index: usize,
    toast: Option<(String, Instant)>,
    data_state: DataState,
    file_state: FileState,
    clear_tool_confirm: bool,
    plugins: crate::plugin_ui::PluginState,
    integrations: crate::integrations::IntegrationState,
    frameworks: crate::framework::State,
    home_filter: String,
    home_category: String,
    home_page_index: usize,
    home_query_key: (String, String, String),
}

impl DevToolsApp {
    /// Only compiled for the isolated screenshot fixture, never a production entry point.
    #[cfg(feature = "ui-preview")]
    pub fn preview_scene(&mut self, ctx: &egui::Context, scene: usize, fixture: PathBuf) {
        let light = scene % 2 == 1 || scene == 8;
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.launcher_open = false;
        self.tool_search.clear();
        self.preferences.recent.clear();
        self.preferences.usage.clear();
        self.home_page_index = 0;
        self.home_query_key = (String::new(), "全部".into(), "全部分类".into());
        self.home_filter = "全部".into();
        self.home_category = "全部分类".into();
        self.preferences.favorites = vec!["data".into(), "files".into(), "json".into()];
        match scene {
            0 | 1 => self.page = Page::Home,
            2 | 3 => {
                self.navigate(Page::EncodingTools, Some(ToolKind::Yaml));
                self.tool_state.input = ToolKind::Yaml.sample().into();
                self.tool_state.output =
                    run_tool(ToolKind::Yaml, 0, &self.tool_state.input, "", 10).unwrap();
            }
            4 | 5 => {
                self.page = Page::Data;
                self.data_state.sample();
            }
            6 | 7 => {
                self.page = Page::Files;
                self.file_state.preview(fixture);
            }
            9..=20 => {
                let kind = [
                    ToolKind::JsonPath,
                    ToolKind::JsonDiff,
                    ToolKind::DataQuality,
                    ToolKind::Cron,
                    ToolKind::Random,
                    ToolKind::Unicode,
                ][(scene - 9) / 2];
                self.navigate(
                    if kind.is_encoding() {
                        Page::EncodingTools
                    } else {
                        Page::SmallTools
                    },
                    Some(kind),
                );
                self.tool_state.input = kind.sample().into();
                self.tool_state.pattern = kind.secondary_sample().into();
                self.tool_state.output = run_tool(
                    kind,
                    0,
                    &self.tool_state.input,
                    &self.tool_state.pattern,
                    10,
                )
                .unwrap();
            }
            60..=63 => {
                self.visit("json");
                self.visit("files");
                self.visit("json");
                self.event_tx
                    .send(BackgroundEvent::TrayNavigate(if scene == 60 {
                        TrayAction::Settings
                    } else {
                        TrayAction::Collection(["收藏", "最近", "常用"][scene - 61].into())
                    }))
                    .unwrap();
                self.drain_events(ctx);
                assert!(!self.launcher_open);
                if scene == 60 {
                    assert!(self.page == Page::Settings);
                } else {
                    assert_eq!(self.home_filter, ["收藏", "最近", "常用"][scene - 61]);
                }
            }
            32..=59 => {
                self.page = Page::Frameworks;
                self.frameworks
                    .preview(crate::framework::Tool::ALL[(scene - 32) / 2]);
            }
            27..=30 => {
                let kind = if scene <= 28 {
                    ToolKind::JavaTrace
                } else {
                    ToolKind::DjangoTrace
                };
                self.navigate(Page::SmallTools, Some(kind));
                self.tool_state.input = kind.sample().into();
                self.tool_state.output = run_tool(kind, 0, &self.tool_state.input, "", 10).unwrap();
            }
            31 => {
                self.page = Page::Home;
                self.home_page_index = 1;
            }
            21..=26 => {
                let bytes = include_bytes!("../plugins-examples/local-text.json");
                if self.plugins.store.packages.is_empty() {
                    self.plugins.store.install(bytes).unwrap();
                }
                self.plugins.store.set_enabled("local-text", true).unwrap();
                match scene {
                    21 | 22 => {
                        self.page = Page::Plugins;
                        self.plugins.selected = None;
                    }
                    23 => {
                        self.page = Page::Home;
                        self.home_category = "文本与编码".into();
                    }
                    24 => {
                        self.page = Page::Home;
                        self.visit("plugin:local-text/deduplicate");
                        self.visit("json");
                        self.home_filter = "最近".into();
                    }
                    25 => {
                        self.page = Page::Plugins;
                        self.plugins.select("plugin:local-text/deduplicate");
                    }
                    _ => {
                        self.page = Page::Home;
                        self.open_launcher();
                        self.launcher_query = "去重".into();
                    }
                }
            }
            _ => {
                self.page = Page::Home;
                self.open_launcher();
                self.launcher_query = "json".into();
            }
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_plugin_navigation(&self) -> bool {
        self.page == Page::Plugins
            && self.plugins.selected.as_deref() == Some("plugin:local-text/deduplicate")
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_framework_navigation(&self) -> (bool, bool) {
        (
            self.page == Page::Frameworks
                && self.frameworks.selected == crate::framework::Tool::Sql,
            self.frameworks.preview_completed > 0 && !self.frameworks.is_running(),
        )
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_navigation(&self) -> (bool, bool) {
        (self.launcher_open, self.page == Page::Files)
    }
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config_path: PathBuf,
        restore_services: bool,
    ) -> Self {
        let preferences_path = config_path
            .parent()
            .map(|p| p.join("ui-preferences.json"))
            .unwrap_or_else(preferences::path);
        let preferences = Preferences::load(&preferences_path);
        let theme = if preferences.light {
            Theme::Light
        } else {
            Theme::Dark
        };
        configure_ui(&cc.egui_ctx, theme);
        let (manager, config_error) = match load_config(&config_path).and_then(ServiceManager::new)
        {
            Ok(manager) => (manager, None),
            Err(error) => {
                let fallback = fallback_config(config_path.clone());
                (
                    ServiceManager::new(fallback).expect("fallback config must initialize"),
                    Some(error.to_string()),
                )
            }
        };
        let (tray, tray_error) =
            match TrayController::new(manager.config_snapshot().services.values()) {
                Ok(tray) => (Some(tray), None),
                Err(error) => (None, Some(format!("托盘初始化失败：{error}"))),
            };
        let (event_tx, event_rx) = unbounded();
        let tray_bridge_stop = Arc::new(AtomicBool::new(false));
        let tray_exit_requested = Arc::new(AtomicBool::new(false));
        let window_handle = main_window_handle(cc);
        start_tray_bridge(
            Arc::clone(&manager),
            event_tx.clone(),
            cc.egui_ctx.clone(),
            window_handle,
            Arc::clone(&tray_bridge_stop),
            Arc::clone(&tray_exit_requested),
        );
        let duplicate_count = other_instance_count();
        let mut app = Self {
            manager,
            tray,
            page: Page::Home,
            statuses: Vec::new(),
            selected_service: None,
            search: String::new(),
            service_filter: ServiceFilter::All,
            event_tx,
            event_rx,
            refresh_inflight: false,
            last_refresh: Instant::now() - Duration::from_secs(30),
            notification: tray_error.clone().unwrap_or_default(),
            notification_error: tray_error.is_some(),
            startup_warning: (duplicate_count > 0).then(|| {
                format!(
                    "检测到 {duplicate_count} 个其他 Zi DevTools 实例；请确认当前托盘图标版本，旧实例不会自动退出"
                )
            }),
            log_view: None,
            log_text: String::new(),
            config_view: None,
            config_text: String::new(),
            tool_state: ToolState::default(),
            http_state: HttpWorkbenchState::load(default_http_storage_path()),
            diff_state: DiffState::default(),
            network_state: NetworkState::default(),
            qr_texture: None,
            config_path_input: config_path.to_string_lossy().into_owned(),
            config_error,
            quit_requested: false,
            tray_bridge_stop,
            tray_exit_requested,
            window_handle,
            theme,
            colors: palette(theme),
            preferences, preferences_path: preferences_path.clone(),
            tool_search: String::new(), launcher_query: String::new(), launcher_open: false,
            launcher_focus: false, launcher_index: 0, toast: None,
            data_state: DataState::default(), file_state: FileState::default(), clear_tool_confirm:false,
            plugins: crate::plugin_ui::PluginState::new(preferences_path.parent().unwrap_or(std::path::Path::new(".")).join("plugins")),
            integrations: Default::default(), frameworks: Default::default(), home_filter: "全部".into(), home_category: "全部分类".into(), home_page_index: 0, home_query_key: Default::default(),
        };
        if restore_services {
            app.spawn_restore();
        }
        app.request_refresh();
        app
    }

    fn request_refresh(&mut self) {
        if self.refresh_inflight {
            return;
        }
        self.refresh_inflight = true;
        self.last_refresh = Instant::now();
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(BackgroundEvent::Statuses(manager.list_services()));
        });
    }

    fn spawn_restore(&self) {
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let results = manager.restore_running_services();
            let restored = results.iter().filter(|item| item.is_ok()).count();
            let failed = results.len().saturating_sub(restored);
            let _ = tx.send(BackgroundEvent::RestoreFinished(format!(
                "启动恢复完成：成功 {restored}，失败 {failed}"
            )));
        });
    }

    fn run_action(&mut self, service_id: String, action: &'static str) {
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        self.notification = format!("正在{} {}…", action_label(action), service_id);
        self.notification_error = false;
        std::thread::spawn(move || {
            let result = match action {
                "start" => manager.start(&service_id),
                "stop" => manager.stop(&service_id),
                "restart" => manager.restart(&service_id),
                _ => unreachable!(),
            }
            .map_err(|error| format!("{service_id}: {error}"));
            let _ = tx.send(BackgroundEvent::Action(result));
        });
    }

    fn run_all(&mut self, start: bool) {
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        self.notification = if start {
            "正在启动全部服务…".to_owned()
        } else {
            "正在停止全部服务…".to_owned()
        };
        std::thread::spawn(move || {
            let _ = tx.send(BackgroundEvent::Action(execute_batch(&manager, start)));
        });
    }

    fn request_logs(&mut self, service_id: String) {
        self.log_view = Some(service_id.clone());
        self.log_text = "正在读取日志…".to_owned();
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = manager.logs(&service_id, 5_000).map_err(|e| e.to_string());
            let _ = tx.send(BackgroundEvent::Logs(service_id, result));
        });
    }

    fn request_config_preview(&mut self, service_id: String, path: String) {
        self.config_view = Some((service_id.clone(), path.clone()));
        self.config_text = "正在读取配置文件…".to_owned();
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = manager
                .read_config_file(&service_id, &path)
                .map_err(|e| e.to_string());
            let _ = tx.send(BackgroundEvent::ConfigPreview(path, result));
        });
    }

    fn clear_logs(&mut self, service_id: String) {
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = manager.clear_logs(&service_id).map(|()| ActionResult {
                service_id,
                message: "日志已清空".to_owned(),
            });
            let _ = tx.send(BackgroundEvent::Action(result.map_err(|e| e.to_string())));
        });
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                BackgroundEvent::Statuses(statuses) => {
                    self.refresh_inflight = false;
                    if let Some(tray) = &self.tray {
                        tray.update_status(&statuses);
                    }
                    self.statuses = statuses;
                }
                BackgroundEvent::Action(result) => {
                    match result {
                        Ok(value) => {
                            self.notification = if value.service_id == "all" {
                                value.message
                            } else {
                                format!("{}：{}", value.service_id, value.message)
                            };
                            self.notification_error = false;
                        }
                        Err(error) => {
                            self.notification = error;
                            self.notification_error = true;
                        }
                    }
                    self.refresh_inflight = false;
                    self.last_refresh = Instant::now() - Duration::from_secs(30);
                    self.request_refresh();
                }
                BackgroundEvent::Logs(service_id, result) => {
                    if self.log_view.as_deref() == Some(&service_id) {
                        self.log_text = result.unwrap_or_else(|error| format!("读取失败：{error}"));
                    }
                }
                BackgroundEvent::ConfigPreview(path, result) => {
                    if self
                        .config_view
                        .as_ref()
                        .is_some_and(|(_, current)| current == &path)
                    {
                        self.config_text =
                            result.unwrap_or_else(|error| format!("读取失败：{error}"));
                    }
                }
                BackgroundEvent::RestoreFinished(message) => {
                    self.notification = message;
                    self.notification_error = false;
                    self.last_refresh = Instant::now() - Duration::from_secs(30);
                }
                BackgroundEvent::TrayNavigate(action) => match action {
                    TrayAction::Search => self.open_launcher(),
                    TrayAction::Settings => {
                        self.launcher_open = false;
                        self.page = Page::Settings;
                    }
                    TrayAction::Collection(filter) => {
                        self.launcher_open = false;
                        self.page = Page::Home;
                        self.home_filter = filter;
                        self.home_category = "全部分类".into();
                        self.tool_search.clear();
                        self.home_page_index = 0;
                    }
                    TrayAction::OpenEntry(id) => {
                        if let Some(entry) = self.entries("").into_iter().find(|e| e.id == id) {
                            self.open_entry(&entry);
                        } else {
                            self.toast = Some((
                                "该工具已停用或移除，请在插件中心检查".into(),
                                Instant::now(),
                            ));
                        }
                    }
                    _ => {}
                },
                BackgroundEvent::NavigateTool(tool) => match tool {
                    TrayTool::Small(kind) => {
                        self.page = if kind.is_encoding() {
                            Page::EncodingTools
                        } else {
                            Page::SmallTools
                        };
                        self.tool_state.select(kind);
                        self.visit(kind.id());
                    }
                    TrayTool::Http => self.navigate(Page::Http, None),
                    TrayTool::Diff => self.navigate(Page::Diff, None),
                    TrayTool::Network => self.navigate(Page::Network, None),
                    TrayTool::Data => self.navigate(Page::Data, None),
                    TrayTool::Files => self.navigate(Page::Files, None),
                    TrayTool::Plugins => {
                        self.plugins.selected = None;
                        self.navigate(Page::Plugins, None);
                    }
                    TrayTool::Integrations => self.navigate(Page::Integrations, None),
                    TrayTool::Framework(tool) => {
                        self.frameworks.select(tool);
                        self.page = Page::Frameworks;
                        self.visit(tool.id());
                    }
                },
                BackgroundEvent::NetworkResult(request_id, result) => {
                    if self.network_state.request_id == request_id {
                        self.network_state.busy = false;
                        self.network_state.output = result.unwrap_or_else(|error| error);
                    }
                }
                BackgroundEvent::HttpResult {
                    tab_id,
                    method,
                    url,
                    duration_ms,
                    result,
                } => {
                    let status = match &result {
                        Ok(output) => output.lines().next().unwrap_or("HTTP 响应").to_owned(),
                        Err(_) => "请求失败".to_owned(),
                    };
                    if let Some(tab) = self.http_state.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    {
                        tab.busy = false;
                        match result {
                            Ok(output) => {
                                tab.output = output;
                                tab.message.clear();
                            }
                            Err(error) => tab.message = error,
                        }
                    }
                    self.http_state.record(&method, &url, &status, duration_ms);
                }
            }
            ctx.request_repaint();
        }
    }

    fn maybe_reload_config(&mut self) {
        let current = self.manager.config_snapshot();
        let modified = fs::metadata(&current.path).and_then(|m| m.modified()).ok();
        if modified.is_some() && modified != current.modified {
            match load_config(&current.path).and_then(|config| self.manager.replace_config(config))
            {
                Ok(()) => {
                    self.config_error = None;
                    match TrayController::new(self.manager.config_snapshot().services.values()) {
                        Ok(tray) => self.tray = Some(tray),
                        Err(error) => {
                            self.notification = format!("托盘初始化失败：{error}");
                            self.notification_error = true;
                        }
                    }
                    self.last_refresh = Instant::now() - Duration::from_secs(30);
                }
                Err(error) => self.config_error = Some(error.to_string()),
            }
        }
    }

    fn set_theme(&mut self, ctx: &egui::Context, theme: Theme) {
        self.theme = theme;
        self.colors = palette(theme);
        apply_theme(ctx, theme);
        self.preferences.light = theme == Theme::Light;
        if let Err(error) = self.preferences.save(&self.preferences_path) {
            self.toast = Some((error.to_string(), Instant::now()));
        }
    }

    fn hide_to_tray(&self, ctx: &egui::Context) {
        hide_main_window(self.window_handle);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        ctx.request_repaint();
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let previous_page = self.page;
        let p = self.colors;
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(224.0)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(16.0))
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Zi")
                            .size(27.0)
                            .strong()
                            .color(p.accent_hover),
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("DEVTOOLS").size(17.0).strong());
                        ui.label(
                            RichText::new("你的本地开发工作台")
                                .size(11.0)
                                .color(p.muted),
                        );
                    });
                });
                ui.add_space(18.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new("搜索工具     Ctrl K"),
                    )
                    .clicked()
                {
                    self.open_launcher();
                }
                ui.add_space(14.0);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-scroll")
                    .max_height((ui.available_height() - 102.0).max(160.0))
                    .show(ui, |ui| {
                        nav_button(ui, &mut self.page, Page::Home, "工具首页");
                        nav_button(ui, &mut self.page, Page::Services, "本地服务");
                        nav_button(ui, &mut self.page, Page::Plugins, "插件与连接器");
                        nav_button(ui, &mut self.page, Page::Integrations, "本机集成发现");
                        nav_button(ui, &mut self.page, Page::Frameworks, "Java / Django 诊断");
                        ui.add_space(10.0);
                        ui.label(RichText::new("轻量工具").size(11.0).strong().color(p.muted));
                        ui.add_space(4.0);
                        nav_button(ui, &mut self.page, Page::SmallTools, "文本与常用工具");
                        nav_button(ui, &mut self.page, Page::EncodingTools, "编码与格式转换");
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("开发工作台")
                                .size(11.0)
                                .strong()
                                .color(p.muted),
                        );
                        ui.add_space(4.0);
                        for (page, label) in [
                            (Page::Data, "CSV / JSON 数据"),
                            (Page::Files, "批量文件校验"),
                            (Page::Http, "HTTP 请求调试"),
                            (Page::Diff, "文本差异对比"),
                            (Page::Network, "网络诊断"),
                        ] {
                            nav_button(ui, &mut self.page, page, label);
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("Stage 19  ·  v{}", env!("CARGO_PKG_VERSION")))
                            .size(11.0)
                            .color(p.muted),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(self.theme.label())
                            .on_hover_text("切换亮 / 暗主题，自动保存")
                            .clicked()
                        {
                            self.set_theme(
                                ctx,
                                if self.theme == Theme::Dark {
                                    Theme::Light
                                } else {
                                    Theme::Dark
                                },
                            );
                        }
                        if ui
                            .add_enabled(self.tray.is_some(), egui::Button::new("托盘").small())
                            .on_hover_text("隐藏主窗口，任务继续运行")
                            .clicked()
                        {
                            self.hide_to_tray(ctx);
                        }
                        if ui.small_button("设置").clicked() {
                            self.page = Page::Settings;
                        }
                    });
                    ui.add_space(8.0);
                    ui.separator();
                });
            });
        if self.page != previous_page {
            self.tool_search.clear();
            if self.page == Page::Plugins {
                self.plugins.selected = None;
            }
            if let Some(e) = catalog().into_iter().find(|e| {
                e.page == self.page
                    && e.kind.is_none()
                    && (e.page != Page::Frameworks || e.id == self.frameworks.selected.id())
            }) {
                self.visit(&e.id);
            }
        }
    }

    fn open_launcher(&mut self) {
        self.launcher_open = true;
        self.launcher_focus = true;
        self.launcher_query.clear();
        self.launcher_index = 0;
    }
    fn navigate(&mut self, page: Page, kind: Option<ToolKind>) {
        self.page = page;
        self.tool_search.clear();
        if let Some(kind) = kind {
            self.tool_state.select(kind);
        }
        self.launcher_open = false;
        let id = kind.map(|k| k.id().to_owned()).or_else(|| {
            catalog()
                .into_iter()
                .find(|e| e.page == page && e.kind.is_none())
                .map(|e| e.id)
        });
        if let Some(id) = id {
            self.visit(&id);
        }
    }
    fn toggle_favorite(&mut self, id: &str) {
        self.preferences.toggle(id);
        self.toast = Some((
            if self.preferences.favorites.iter().any(|s| s == id) {
                "已收藏 · 可从托盘快速打开"
            } else {
                "已取消收藏"
            }
            .into(),
            Instant::now(),
        ));
        if let Err(e) = self.preferences.save(&self.preferences_path) {
            self.toast = Some((e.to_string(), Instant::now()));
        }
    }

    fn entries(&self, query: &str) -> Vec<ToolEntry> {
        let mut entries = catalog();
        entries.extend(self.plugins.store.tool_refs().map(|(id, t)| ToolEntry {
            id,
            title: t.name.clone(),
            description: t.description.clone(),
            category: t.category.clone(),
            keywords: t.keywords.join(" "),
            page: Page::Plugins,
            kind: None,
        }));
        let mut scored: Vec<_> = entries
            .into_iter()
            .filter_map(|e| {
                e.score(query).map(|score| {
                    let rank = score
                        + if self.preferences.favorites.contains(&e.id) {
                            30
                        } else {
                            0
                        }
                        + self
                            .preferences
                            .recent
                            .iter()
                            .position(|id| id == &e.id)
                            .map(|i| 20_u32.saturating_sub(i as u32))
                            .unwrap_or(0)
                        + self
                            .preferences
                            .usage
                            .get(&e.id)
                            .copied()
                            .unwrap_or(0)
                            .min(10);
                    (rank, e)
                })
            })
            .collect();
        scored.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
        scored.into_iter().map(|(_, e)| e).collect()
    }
    fn visit(&mut self, id: &str) {
        self.preferences.visit(id);
        if let Err(e) = self.preferences.save(&self.preferences_path) {
            self.toast = Some((e.to_string(), Instant::now()));
        }
    }
    fn open_entry(&mut self, e: &ToolEntry) {
        if let Some(tool) = crate::framework::Tool::from_id(&e.id) {
            self.frameworks.select(tool);
            self.page = Page::Frameworks;
            self.launcher_open = false;
            self.visit(&e.id);
        } else if e.id.starts_with("plugin:") {
            self.plugins.select(&e.id);
            self.page = Page::Plugins;
            self.launcher_open = false;
            self.visit(&e.id);
        } else {
            if e.page == Page::Plugins {
                self.plugins.selected = None;
            }
            self.navigate(e.page, e.kind);
        }
    }
    fn home_page(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        ui.heading(RichText::new("你的工具工作台").size(30.0));
        ui.label(
            RichText::new(format!(
                "{} 项内置能力 · {} 项已启用插件工具 · Ctrl K 随时打开",
                catalog().len(),
                self.plugins.store.tool_refs().count()
            ))
            .color(p.muted),
        );
        ui.add_space(16.0);
        ui.add_sized(
            [ui.available_width(), 38.0],
            egui::TextEdit::singleline(&mut self.tool_search)
                .hint_text("搜索名称、用途或关键词，例如 JSON 提取、LLM、文件…"),
        );
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            for filter in ["全部", "收藏", "最近", "常用"] {
                ui.selectable_value(&mut self.home_filter, filter.into(), filter);
            }
            ui.separator();
            egui::ComboBox::from_id_salt("tool-category")
                .selected_text(&self.home_category)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.home_category, "全部分类".into(), "全部分类");
                    for category in crate::plugins::CATEGORIES {
                        ui.selectable_value(&mut self.home_category, (*category).into(), *category);
                    }
                });
            if ui.small_button("重置筛选").clicked() {
                self.tool_search.clear();
                self.home_category = "全部分类".into();
                self.home_filter = "全部".into();
            }
        });
        let key = (
            self.tool_search.clone(),
            self.home_filter.clone(),
            self.home_category.clone(),
        );
        if self.home_query_key != key {
            self.home_page_index = 0;
            self.home_query_key = key;
        }
        let mut entries = self.entries(&self.tool_search);
        entries.retain(|e| {
            (self.home_category == "全部分类" || e.category == self.home_category)
                && match self.home_filter.as_str() {
                    "收藏" => self.preferences.favorites.contains(&e.id),
                    "最近" => self.preferences.recent.contains(&e.id),
                    "常用" => self.preferences.usage.contains_key(&e.id),
                    _ => true,
                }
        });
        if self.home_filter == "收藏" {
            entries.sort_by_key(|e| {
                self.preferences
                    .favorites
                    .iter()
                    .position(|id| id == &e.id)
                    .unwrap_or(usize::MAX)
            });
        }
        if self.home_filter == "最近" {
            entries.sort_by_key(|e| {
                self.preferences
                    .recent
                    .iter()
                    .position(|id| id == &e.id)
                    .unwrap_or(usize::MAX)
            });
        }
        if self.home_filter == "常用" {
            entries.sort_by_key(|e| {
                (
                    std::cmp::Reverse(self.preferences.usage.get(&e.id).copied().unwrap_or(0)),
                    self.preferences
                        .recent
                        .iter()
                        .position(|id| id == &e.id)
                        .unwrap_or(usize::MAX),
                )
            });
        }
        ui.add_space(12.0);
        let (_, pages) = page_bounds(entries.len(), self.home_page_index);
        self.home_page_index = self.home_page_index.min(pages - 1);
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{} 个匹配工具 · 第 {} / {} 页",
                entries.len(),
                self.home_page_index + 1,
                pages
            ));
            if ui
                .add_enabled(self.home_page_index > 0, egui::Button::new("上一页"))
                .clicked()
            {
                self.home_page_index -= 1;
            }
            if ui
                .add_enabled(
                    self.home_page_index + 1 < pages,
                    egui::Button::new("下一页"),
                )
                .clicked()
            {
                self.home_page_index += 1;
            }
            if self.home_filter == "常用" {
                ui.label("按打开次数排序");
            }
        });
        if entries.is_empty() {
            ui.add_space(30.0);
            ui.strong("这个视图还没有工具");
            ui.label("试试其他分类、清空搜索，或在插件中心安装并启用连接器。打开工具后会自动进入最近与常用。");
        }
        let columns = if ui.available_width() >= 750.0 { 3 } else { 2 };
        let (range, _) = page_bounds(entries.len(), self.home_page_index);
        for row in entries[range].chunks(columns) {
            ui.columns(columns, |cols| {
                for (col, e) in cols.iter_mut().zip(row) {
                    egui::Frame::new()
                        .fill(p.card)
                        .stroke(egui::Stroke::new(1.0, p.surface))
                        .corner_radius(12)
                        .inner_margin(14.0)
                        .show(col, |ui| {
                            ui.set_min_height(132.0);
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&e.category).small().color(p.muted));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let selected = self.preferences.favorites.contains(&e.id);
                                        if ui
                                            .small_button(if selected { "★" } else { "☆" })
                                            .on_hover_text("收藏 / 取消收藏")
                                            .clicked()
                                        {
                                            self.toggle_favorite(&e.id);
                                        }
                                    },
                                );
                            });
                            ui.label(RichText::new(&e.title).size(16.0).strong());
                            ui.label(RichText::new(&e.description).small().color(p.muted));
                            ui.label(
                                RichText::new(if e.id.starts_with("plugin:") {
                                    "插件"
                                } else {
                                    "内置"
                                })
                                .small()
                                .color(p.muted),
                            );
                            if ui
                                .add_sized(
                                    [ui.available_width(), 28.0],
                                    egui::Button::new("打开工具 →"),
                                )
                                .clicked()
                            {
                                self.open_entry(e);
                            }
                        });
                }
            });
            ui.add_space(10.0);
        }
    }

    fn launcher(&mut self, ctx: &egui::Context) {
        if !self.launcher_open {
            return;
        }
        let mut open = true;
        let mut chosen = None;
        egui::Window::new("快速打开工具")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 70.0])
            .show(ctx, |ui| {
                let response = ui.add_sized(
                    [ui.available_width(), 38.0],
                    egui::TextEdit::singleline(&mut self.launcher_query)
                        .hint_text("输入名称或用途…"),
                );
                let mut scroll_selection = self.launcher_focus;
                if self.launcher_focus {
                    response.request_focus();
                    self.launcher_focus = false;
                }
                if response.changed() {
                    self.launcher_index = 0;
                    scroll_selection = true;
                }
                let query = self.launcher_query.to_lowercase();
                let entries = self.entries(&query);
                if entries.is_empty() {
                    ui.label("没有匹配的工具");
                } else {
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
                    {
                        scroll_selection = true;
                        self.launcher_index = (self.launcher_index + 1).min(entries.len() - 1);
                    }
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                        scroll_selection = true;
                        self.launcher_index = self.launcher_index.saturating_sub(1);
                    }
                    self.launcher_index = self.launcher_index.min(entries.len() - 1);
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .show(ui, |ui| {
                            for (i, e) in entries.iter().enumerate() {
                                let response = ui.selectable_label(
                                    i == self.launcher_index,
                                    format!(
                                        "{}   ·   {}   ·   {}",
                                        e.title,
                                        e.category,
                                        if e.id.starts_with("plugin:") {
                                            "插件"
                                        } else {
                                            "内置"
                                        }
                                    ),
                                );
                                if scroll_selection && i == self.launcher_index {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    chosen = Some(e.id.clone());
                                }
                            }
                        });
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        let e = &entries[self.launcher_index];
                        chosen = Some(e.id.clone());
                    }
                }
                ui.separator();
                ui.label(
                    RichText::new("↑ ↓ 选择     Enter 打开     Esc 关闭")
                        .small()
                        .weak(),
                );
            });
        if !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.launcher_open = false;
        }
        if let Some(id) = chosen
            && let Some(entry) = self.entries("").into_iter().find(|e| e.id == id)
        {
            self.open_entry(&entry);
        }
    }

    fn services_page(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        let running = self
            .statuses
            .iter()
            .filter(|status| status.state == ServiceState::Running)
            .count();
        let external = self
            .statuses
            .iter()
            .filter(|status| {
                matches!(
                    status.state,
                    ServiceState::External | ServiceState::PortOpen
                )
            })
            .count();
        let stopped = self.statuses.len().saturating_sub(running + external);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("本地服务").size(28.0));
                ui.label(RichText::new("管理开发环境进程、健康状态与日志").color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("全部停止").clicked() {
                    self.run_all(false);
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("全部启动").color(Color32::WHITE))
                            .fill(p.accent),
                    )
                    .clicked()
                {
                    self.run_all(true);
                }
                if ui.button("↻ 刷新").clicked() {
                    self.last_refresh = Instant::now() - Duration::from_secs(30);
                }
            });
        });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            metric(ui, "托管运行", running, p.green);
            metric(ui, "外部/占用", external, p.amber);
            metric(ui, "已停止", stopped, p.muted);
            metric(ui, "服务总数", self.statuses.len(), p.accent);
        });
        ui.add_space(16.0);
        if let Some(warning) = self.startup_warning.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(69, 55, 34))
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(warning).color(Color32::WHITE));
                        if ui.small_button("关闭提示").clicked() {
                            self.startup_warning = None;
                        }
                    });
                });
            ui.add_space(12.0);
        }
        if let Some(error) = self.config_error.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(69, 37, 44))
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("服务配置无法加载：{error}"));
                        if ui.button("打开设置").clicked() {
                            self.page = Page::Settings;
                        }
                    });
                });
            ui.add_space(12.0);
        }
        if !self.notification.is_empty() {
            egui::Frame::new()
                .fill(if self.notification_error {
                    Color32::from_rgb(69, 37, 44)
                } else {
                    Color32::from_rgb(27, 65, 54)
                })
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&self.notification).color(Color32::WHITE));
                        if ui.small_button("关闭提示").clicked() {
                            self.notification.clear();
                        }
                    });
                });
            ui.add_space(12.0);
        }
        ui.horizontal(|ui| {
            ui.label("筛选服务");
            ui.add_sized(
                [420.0, 32.0],
                egui::TextEdit::singleline(&mut self.search).hint_text("名称、标签或端口"),
            );
        });
        ui.horizontal(|ui| {
            for filter in ServiceFilter::ALL {
                if ui
                    .selectable_label(self.service_filter == filter, filter.label())
                    .clicked()
                {
                    self.service_filter = filter;
                }
            }
        });

        let query = self.search.to_lowercase();
        let statuses: Vec<ServiceStatus> = self
            .statuses
            .iter()
            .filter(|status| {
                self.service_filter.matches(status.state, status.managed)
                    && (query.is_empty()
                        || status.name.to_lowercase().contains(&query)
                        || status.id.to_lowercase().contains(&query)
                        || status
                            .tags
                            .iter()
                            .any(|tag| tag.to_lowercase().contains(&query))
                        || status
                            .port
                            .is_some_and(|port| port.to_string().contains(&query)))
            })
            .cloned()
            .collect();
        ui.label(
            RichText::new(format!(
                "显示 {} / {} 项",
                statuses.len(),
                self.statuses.len()
            ))
            .small()
            .color(p.muted),
        );
        ui.add_space(8.0);
        if statuses.is_empty() {
            ui.add_space(24.0);
            ui.label(
                RichText::new(if self.statuses.is_empty() {
                    "没有加载到服务。请在“设置”中检查配置文件。"
                } else {
                    "没有匹配的服务，请调整筛选词。"
                })
                .color(p.muted),
            );
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for status in statuses {
                self.service_card(ui, status);
                ui.add_space(10.0);
            }
        });
    }

    fn service_card(&mut self, ui: &mut egui::Ui, status: ServiceStatus) {
        let p = self.colors;
        egui::Frame::new()
            .fill(p.card)
            .corner_radius(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let state_color = if status.managed && status.health.ok == Some(false) {
                        p.amber
                    } else {
                        match status.state {
                            ServiceState::Running => p.green,
                            ServiceState::External | ServiceState::PortOpen => p.amber,
                            ServiceState::Stopped => p.muted,
                        }
                    };
                    ui.label(RichText::new("●").color(state_color).size(17.0));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&status.name).size(17.0).strong());
                            ui.label(
                                RichText::new(status.display_state())
                                    .color(state_color)
                                    .small(),
                            );
                            for tag in status.tags.iter().take(3) {
                                ui.label(RichText::new(tag).color(p.muted).small());
                            }
                        });
                        if !status.description.is_empty() {
                            ui.label(RichText::new(&status.description).color(p.muted));
                        }
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if status.managed {
                        if ui.button("停止").clicked() {
                            self.run_action(status.id.clone(), "stop");
                        }
                        if ui.button("重启").clicked() {
                            self.run_action(status.id.clone(), "restart");
                        }
                    } else if status.state == ServiceState::Stopped
                        && ui
                            .add(
                                egui::Button::new(RichText::new("启动").color(Color32::WHITE))
                                    .fill(p.accent),
                            )
                            .clicked()
                    {
                        self.run_action(status.id.clone(), "start");
                    }
                    if matches!(
                        status.state,
                        ServiceState::External | ServiceState::PortOpen
                    ) {
                        ui.label(
                            RichText::new("外部进程占用：不执行启停")
                                .small()
                                .color(p.amber),
                        );
                    }
                    if ui.button("查看日志").clicked() {
                        self.request_logs(status.id.clone());
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(port) = status.port {
                        ui.label(RichText::new(format!("端口 {port}")).color(p.muted));
                    }
                    if let Some(pid) = status.pid {
                        ui.label(RichText::new(format!("PID {pid}")).color(p.muted));
                    }
                    if status.managed {
                        ui.label(RichText::new("本程序托管").color(p.green));
                    } else if status.port_open == Some(true) {
                        ui.label(RichText::new("检测到外部监听").color(p.amber));
                    }
                    if let Some(timeout) = status.graceful_stop_timeout_ms {
                        ui.label(RichText::new(format!("优雅停止 {timeout} ms")).color(p.muted));
                    }
                    if let Some(code) = status.health.status_code {
                        ui.label(RichText::new(format!("HTTP {code}")).color(p.muted));
                    }
                    if let Some(ms) = status.health.elapsed_ms {
                        ui.label(RichText::new(format!("{ms} ms")).color(p.muted));
                    }
                    ui.label(RichText::new(status.repo.display().to_string()).color(p.muted));
                });

                let expanded = self.selected_service.as_deref() == Some(&status.id);
                if ui
                    .small_button(if expanded {
                        "收起详情"
                    } else {
                        "查看详情"
                    })
                    .clicked()
                {
                    self.selected_service = if expanded {
                        None
                    } else {
                        Some(status.id.clone())
                    };
                }
                if expanded {
                    ui.separator();
                    ui.label(RichText::new("启动命令").small().color(p.muted));
                    ui.monospace(&status.command);
                    if let Some(url) = &status.health_url {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("健康检查").small().color(p.muted));
                            if ui.link(url).clicked() {
                                let _ = open::that(url);
                            }
                        });
                    }
                    ui.label(
                        RichText::new(format!(
                            "日志 {} · {} bytes · 环境变量 {} 项（值已隐藏）",
                            status.log_path.display(),
                            status.log_size,
                            status.env_count
                        ))
                        .small()
                        .color(p.muted),
                    );
                    if let Some(message) = &status.health.message {
                        ui.label(RichText::new(message).small().color(p.muted));
                    }
                    if !status.config_files.is_empty() {
                        ui.label(RichText::new("配置文件").small().color(p.muted));
                        for file in &status.config_files {
                            ui.horizontal(|ui| {
                                let marker = if file.exists { "●" } else { "○" };
                                ui.label(
                                    RichText::new(format!(
                                        "{marker} {} · {} bytes",
                                        file.configured_path, file.size
                                    ))
                                    .small()
                                    .color(if file.exists { p.muted } else { p.red }),
                                );
                                if file.exists && ui.small_button("查看").clicked() {
                                    self.request_config_preview(
                                        status.id.clone(),
                                        file.configured_path.clone(),
                                    );
                                }
                                if file.exists && ui.small_button("打开所在目录").clicked() {
                                    if let Some(parent) = file.absolute_path.parent() {
                                        let _ = open::that(parent);
                                    }
                                }
                            });
                        }
                    }
                }
            });
    }

    fn small_tools_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.tool_page(ui, ctx, false);
    }

    fn encoding_tools_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.tool_page(ui, ctx, true);
    }

    fn tool_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, encoding: bool) {
        let p = self.colors;
        if self.tool_state.selected.is_encoding() != encoding
            && let Some(first) = ToolKind::ALL
                .into_iter()
                .find(|kind| kind.is_encoding() == encoding)
        {
            self.tool_state.select(first);
        }
        let title = if encoding {
            "编码工具"
        } else {
            "小工具"
        };
        ui.heading(RichText::new(title).size(28.0));
        ui.label(RichText::new("所有转换都在本机内存中完成，不上传数据").color(p.muted));
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.tool_search)
                .hint_text("筛选工具…")
                .desired_width(260.0),
        );
        ui.add_space(10.0);
        // Keep the Stage-10 single-column flow: the tool picker wraps above
        // the editor instead of splitting the editor into a second pane.
        ui.horizontal_wrapped(|ui| {
            for kind in ToolKind::ALL {
                if kind.is_encoding() != encoding
                    || !format!("{} {} {}", kind.id(), kind.label(), kind.description())
                        .to_lowercase()
                        .contains(&self.tool_search.to_lowercase())
                {
                    continue;
                }
                let selected = self.tool_state.selected == kind;
                let button =
                    egui::Button::new(RichText::new(kind.label()).size(13.0).color(if selected {
                        Color32::WHITE
                    } else {
                        p.text
                    }))
                    .selected(selected)
                    .fill(if selected { p.accent } else { p.surface });
                if ui.add(button).on_hover_text(kind.description()).clicked() {
                    self.tool_state.select(kind);
                    self.visit(kind.id());
                }
            }
        });
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(p.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(self.tool_state.selected.label())
                            .size(20.0)
                            .strong(),
                    );
                    let id = self.tool_state.selected.id();
                    let favorite = self.preferences.favorites.iter().any(|s| s == id);
                    if ui
                        .small_button(if favorite {
                            "★ 已收藏"
                        } else {
                            "☆ 收藏"
                        })
                        .clicked()
                    {
                        self.toggle_favorite(id);
                    }
                    if ui
                        .add_enabled(
                            self.tool_state.input.is_empty(),
                            egui::Button::new("填入示例").small(),
                        )
                        .on_hover_text("仅在输入为空时可用")
                        .clicked()
                    {
                        self.tool_state.input = self.tool_state.selected.sample().into();
                        if self.tool_state.pattern.is_empty() {
                            self.tool_state.pattern =
                                self.tool_state.selected.secondary_sample().into();
                        }
                    }
                    if ui
                        .add_enabled(
                            !self.tool_state.input.is_empty() || !self.tool_state.output.is_empty(),
                            egui::Button::new("清空").small(),
                        )
                        .clicked()
                    {
                        self.clear_tool_confirm = true;
                    }
                });
                ui.label(
                    RichText::new(self.tool_state.selected.description())
                        .small()
                        .color(p.muted),
                );
                ui.add_space(12.0);
                match self.tool_state.selected {
                    ToolKind::Uuid => {
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("生成 UUID v4").color(Color32::WHITE),
                                    )
                                    .fill(p.accent),
                                )
                                .clicked()
                            {
                                self.tool_state.output = generate_uuid();
                            }
                            if ui.button("复制").clicked() {
                                ctx.copy_text(self.tool_state.output.clone());
                            }
                        });
                    }
                    ToolKind::Qr => {
                        ui.label("输入文本或 URL");
                        ui.add_sized(
                            [ui.available_width(), 100.0],
                            egui::TextEdit::multiline(&mut self.tool_state.input)
                                .font(egui::TextStyle::Monospace),
                        );
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("生成二维码").color(Color32::WHITE),
                                )
                                .fill(p.accent),
                            )
                            .clicked()
                        {
                            match generate_qr(&self.tool_state.input) {
                                Ok(image) => {
                                    let side = image.side as usize;
                                    let texture = ctx.load_texture(
                                        "zi-qr-preview",
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [side, side],
                                            &image.rgba,
                                        ),
                                        egui::TextureOptions::NEAREST,
                                    );
                                    self.tool_state.output =
                                        format!("二维码尺寸：{side} × {side} 像素");
                                    self.tool_state.qr_image = Some(image);
                                    self.qr_texture = Some(texture);
                                    self.tool_state.message.clear();
                                }
                                Err(error) => self.tool_state.message = error.to_string(),
                            }
                        }
                        if let Some(texture) = &self.qr_texture {
                            ui.add_space(10.0);
                            ui.image((texture.id(), egui::vec2(300.0, 300.0)));
                            if ui.button("保存 PNG 到图片文件夹").clicked()
                                && let Some(image) = &self.tool_state.qr_image
                            {
                                let folder = dirs::picture_dir()
                                    .unwrap_or_else(|| {
                                        dirs::home_dir().unwrap_or_default().join("Pictures")
                                    })
                                    .join("ZiDevTools");
                                let path = folder.join(format!("qr-{}.png", uuid::Uuid::new_v4()));
                                let result = fs::create_dir_all(&folder)
                                    .map_err(anyhow::Error::from)
                                    .and_then(|_| image.save_png(&path));
                                match result {
                                    Ok(()) => {
                                        self.tool_state.output =
                                            format!("已保存：{}", path.display())
                                    }
                                    Err(error) => {
                                        self.tool_state.message = format!("保存失败：{error}")
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(label) = self.tool_state.selected.option_label()
                            && self.tool_state.selected != ToolKind::JsonDiff
                        {
                            ui.label(label);
                            ui.add_sized(
                                [ui.available_width(), 32.0],
                                egui::TextEdit::singleline(&mut self.tool_state.pattern)
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.add_space(8.0);
                        }
                        if self.tool_state.selected == ToolKind::Jwt {
                            ui.label(
                                RichText::new("仅查看内容；不验证签名、有效期或可信度")
                                    .color(p.amber),
                            );
                        }
                        if self.tool_state.selected == ToolKind::Number {
                            egui::ComboBox::from_id_salt("number-base")
                                .selected_text(format!("输入进制：{}", self.tool_state.number_base))
                                .show_ui(ui, |ui| {
                                    for base in [2, 8, 10, 16] {
                                        ui.selectable_value(
                                            &mut self.tool_state.number_base,
                                            base,
                                            format!("{base} 进制"),
                                        );
                                    }
                                });
                        }
                        let help = self.tool_state.selected.help();
                        if !help.is_empty() {
                            ui.label(RichText::new(help).small().color(p.muted));
                            ui.add_space(8.0);
                        }
                        ui.horizontal(|ui| {
                            ui.strong(if self.tool_state.selected == ToolKind::JsonDiff {
                                "左侧 JSON"
                            } else {
                                "输入"
                            });
                            ui.label(
                                RichText::new(format!(
                                    "{} 字符 · {} 字节",
                                    self.tool_state.input.chars().count(),
                                    self.tool_state.input.len()
                                ))
                                .small()
                                .color(p.muted),
                            );
                        });
                        ui.add_sized(
                            [
                                ui.available_width(),
                                if self.tool_state.selected == ToolKind::JsonDiff {
                                    88.0
                                } else {
                                    160.0
                                },
                            ],
                            egui::TextEdit::multiline(&mut self.tool_state.input)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("在这里粘贴需要处理的内容…"),
                        );
                        ui.add_space(8.0);
                        if self.tool_state.selected == ToolKind::JsonDiff {
                            ui.strong("右侧 JSON");
                            ui.add_sized(
                                [ui.available_width(), 88.0],
                                egui::TextEdit::multiline(&mut self.tool_state.pattern)
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.add_space(8.0);
                        }
                        self.tool_actions(ui);
                    }
                }
                if !self.tool_state.message.is_empty() {
                    egui::Frame::new()
                        .fill(p.red.gamma_multiply(0.12))
                        .corner_radius(8)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&self.tool_state.message).color(p.red));
                        });
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label("上次处理结果");
                    if ui
                        .add_enabled(
                            !self.tool_state.output.is_empty(),
                            egui::Button::new("复制结果").small(),
                        )
                        .clicked()
                    {
                        ctx.copy_text(self.tool_state.output.clone());
                        self.toast = Some(("结果已复制".into(), Instant::now()));
                    }
                    if ui
                        .add_enabled(
                            !self.tool_state.output.is_empty(),
                            egui::Button::new("交换输入 / 输出").small(),
                        )
                        .clicked()
                    {
                        std::mem::swap(&mut self.tool_state.input, &mut self.tool_state.output);
                    }
                });
                let mut output = self.tool_state.output.as_str();
                ui.add_sized(
                    [ui.available_width(), 180.0],
                    egui::TextEdit::multiline(&mut output)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("处理结果将显示在这里；可以选择文本或点击复制结果"),
                );
            });
    }

    fn tool_actions(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        let mut selected = None;
        let shortcut = !self.launcher_open
            && ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));
        ui.horizontal_wrapped(|ui| {
            for (index, label) in self.tool_state.selected.actions().iter().enumerate() {
                let button = egui::Button::new(RichText::new(*label).color(if index == 0 {
                    Color32::WHITE
                } else {
                    p.text
                }))
                .fill(if index == 0 { p.accent } else { p.surface });
                if ui.add(button).clicked() || (shortcut && index == 0) {
                    selected = Some(index);
                }
            }
        });
        if let Some(index) = selected {
            match run_tool(
                self.tool_state.selected,
                index,
                &self.tool_state.input,
                &self.tool_state.pattern,
                self.tool_state.number_base,
            ) {
                Ok(output) => {
                    self.tool_state.output = output;
                    self.tool_state.message.clear();
                    self.toast = Some(("处理完成".into(), Instant::now()));
                }
                Err(error) => {
                    self.tool_state.message = error.to_string();
                    self.tool_state.output.clear();
                }
            }
        }
    }

    fn http_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.colors;
        ui.heading(RichText::new("HTTP 请求调试").size(28.0));
        ui.label(RichText::new("多请求标签 · 站点配置 · 最近 100 条历史").color(p.muted));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            workbench_tab(ui, &mut self.http_state.pane, HttpPane::Request, "请求");
            workbench_tab(ui, &mut self.http_state.pane, HttpPane::Sites, "站点配置");
            workbench_tab(ui, &mut self.http_state.pane, HttpPane::History, "历史记录");
        });
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(p.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| match self.http_state.pane {
                HttpPane::Request => self.http_request_pane(ui, ctx),
                HttpPane::Sites => self.http_sites_pane(ui),
                HttpPane::History => self.http_history_pane(ui),
            });
        if !self.http_state.message.is_empty() {
            ui.label(RichText::new(&self.http_state.message).color(p.amber));
        }
    }

    fn http_request_pane(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.colors;
        ui.horizontal_wrapped(|ui| {
            for (index, tab) in self.http_state.tabs.iter().enumerate() {
                let selected = index == self.http_state.selected;
                let button = egui::Button::new(RichText::new(&tab.name).size(13.0))
                    .selected(selected)
                    .fill(if selected { p.accent } else { p.panel });
                if ui.add(button).clicked() {
                    self.http_state.selected = index;
                }
            }
            if ui.small_button("＋ 新建").clicked() {
                let _ = self.http_state.add_tab();
            }
            if ui.small_button("× 关闭当前").clicked() {
                self.http_state.close_selected();
            }
        });
        ui.separator();
        let tab = &mut self.http_state.tabs[self.http_state.selected];
        ui.horizontal(|ui| {
            ui.label("名称");
            ui.add(egui::TextEdit::singleline(&mut tab.name).desired_width(160.0));
            egui::ComboBox::from_id_salt(("site-select", tab.id))
                .selected_text(
                    self.http_state
                        .selected_site
                        .and_then(|i| self.http_state.saved.sites.get(i))
                        .map_or("选择站点", |site| site.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.http_state.selected_site.is_none(), "不使用站点")
                        .clicked()
                    {
                        self.http_state.selected_site = None;
                    }
                    for (index, site) in self.http_state.saved.sites.iter().enumerate() {
                        if ui
                            .selectable_label(
                                self.http_state.selected_site == Some(index),
                                &site.name,
                            )
                            .clicked()
                        {
                            self.http_state.selected_site = Some(index);
                        }
                    }
                });
            if ui.button("使用站点地址").clicked()
                && let Some(site) = self
                    .http_state
                    .selected_site
                    .and_then(|i| self.http_state.saved.sites.get(i))
            {
                tab.url = site.base_url.clone();
            }
        });
        ui.label(
            RichText::new("请求由本机发出；写入类方法可能修改目标数据；不自动跟随重定向。")
                .color(p.amber),
        );
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("http-method", tab.id))
                .selected_text(&tab.method)
                .show_ui(ui, |ui| {
                    for method in HTTP_METHODS {
                        ui.selectable_value(&mut tab.method, method.to_owned(), method);
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut tab.url)
                    .desired_width(ui.available_width())
                    .hint_text("https://example.com/api"),
            );
        });
        ui.label("请求头（每行 Header: value；仅保留在当前进程内存）");
        ui.add_sized(
            [ui.available_width(), 90.0],
            egui::TextEdit::multiline(&mut tab.headers).font(egui::TextStyle::Monospace),
        );
        ui.label("请求体（纯文本 / JSON；仅保留在当前进程内存）");
        ui.add_sized(
            [ui.available_width(), 100.0],
            egui::TextEdit::multiline(&mut tab.body).font(egui::TextStyle::Monospace),
        );
        if ui
            .add_enabled(
                !tab.busy,
                egui::Button::new(if tab.busy {
                    "请求中…"
                } else {
                    "发送请求"
                })
                .fill(p.accent),
            )
            .clicked()
        {
            tab.busy = true;
            tab.message.clear();
            let tab_id = tab.id;
            let spec = HttpRequestSpec {
                method: tab.method.clone(),
                url: tab.url.clone(),
                headers: tab.headers.clone(),
                body: tab.body.clone(),
            };
            let tx = self.event_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let result = execute_http(&spec).map_err(|error| error.to_string());
                let _ = tx.send(BackgroundEvent::HttpResult {
                    tab_id,
                    method: spec.method,
                    url: spec.url,
                    duration_ms: started.elapsed().as_millis(),
                    result,
                });
                ctx.request_repaint();
            });
        }
        if !tab.message.is_empty() {
            ui.label(RichText::new(&tab.message).color(p.red));
        }
        ui.horizontal(|ui| {
            ui.label("响应预览（最多 256 KiB）");
            if ui.small_button("复制响应").clicked() {
                ctx.copy_text(tab.output.clone());
            }
        });
        ui.add_sized(
            [ui.available_width(), 220.0],
            egui::TextEdit::multiline(&mut tab.output)
                .font(egui::TextStyle::Monospace)
                .interactive(false),
        );
    }

    fn http_sites_pane(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        ui.label(
            RichText::new("站点只保存名称与基础地址，不保存认证头、请求体或查询参数。")
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            ui.label("名称");
            ui.add(egui::TextEdit::singleline(&mut self.http_state.site_name).desired_width(160.0));
            ui.label("基础地址");
            ui.add(
                egui::TextEdit::singleline(&mut self.http_state.site_url)
                    .desired_width(320.0)
                    .hint_text("http://127.0.0.1:8080"),
            );
            if ui.button("添加站点").clicked() {
                self.http_state.message = match self.http_state.add_site() {
                    Ok(()) => "站点已保存".into(),
                    Err(error) => error.to_string(),
                };
            }
        });
        ui.separator();
        let mut remove = None;
        for (index, site) in self.http_state.saved.sites.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&site.name).strong());
                ui.label(&site.base_url);
                if ui.small_button("使用").clicked() {
                    self.http_state.tabs[self.http_state.selected].url = site.base_url.clone();
                    self.http_state.selected_site = Some(index);
                    self.http_state.pane = HttpPane::Request;
                }
                if ui.small_button("删除").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.http_state.saved.sites.remove(index);
            self.http_state.selected_site = None;
            if let Err(error) = self.http_state.save() {
                self.http_state.message = error.to_string();
            }
        }
    }

    fn http_history_pane(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        ui.label(
            RichText::new(
                "历史仅保存方法、去掉查询参数的 URL、状态和耗时；重新打开不会恢复认证信息。",
            )
            .color(p.muted),
        );
        ui.horizontal(|ui| {
            ui.label("查找");
            ui.add(
                egui::TextEdit::singleline(&mut self.http_state.history_filter)
                    .desired_width(240.0)
                    .hint_text("方法、URL 或状态"),
            );
            if ui.button("清空历史").clicked() {
                self.http_state.confirm_clear_history = true;
            }
        });
        if self.http_state.confirm_clear_history {
            ui.horizontal(|ui| {
                ui.label(RichText::new("确认删除全部 HTTP 历史？").color(p.amber));
                if ui.button("确认删除").clicked() {
                    self.http_state.saved.history.clear();
                    self.http_state.confirm_clear_history = false;
                    if let Err(error) = self.http_state.save() {
                        self.http_state.message = error.to_string();
                    }
                }
                if ui.button("取消").clicked() {
                    self.http_state.confirm_clear_history = false;
                }
            });
        }
        ui.separator();
        let mut reopen = None;
        let filter = self.http_state.history_filter.trim().to_lowercase();
        for entry in &self.http_state.saved.history {
            if !filter.is_empty()
                && !entry.method.to_lowercase().contains(&filter)
                && !entry.safe_url.to_lowercase().contains(&filter)
                && !entry.status.to_lowercase().contains(&filter)
            {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label(&entry.time);
                ui.label(RichText::new(&entry.method).strong());
                ui.label(&entry.safe_url);
                ui.label(&entry.status);
                ui.label(format!("{} ms", entry.duration_ms));
                if ui.small_button("重新打开").clicked() {
                    reopen = Some((entry.method.clone(), entry.safe_url.clone()));
                }
            });
        }
        if let Some((method, url)) = reopen {
            if self.http_state.add_tab() {
                let tab = &mut self.http_state.tabs[self.http_state.selected];
                tab.method = method;
                tab.url = url;
                tab.name = "历史请求".into();
            }
        }
    }

    fn diff_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.colors;
        ui.heading(RichText::new("文本差异对比").size(28.0));
        ui.label(RichText::new("本地逐行比较；不读取文件、不上传文本").color(p.muted));
        ui.add_space(18.0);
        egui::Frame::new()
            .fill(p.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| {
                ui.columns(2, |columns| {
                    columns[0].label("原文");
                    columns[0].add_sized(
                        [columns[0].available_width(), 210.0],
                        egui::TextEdit::multiline(&mut self.diff_state.diff_before)
                            .font(egui::TextStyle::Monospace),
                    );
                    columns[1].label("修改后");
                    columns[1].add_sized(
                        [columns[1].available_width(), 210.0],
                        egui::TextEdit::multiline(&mut self.diff_state.diff_after)
                            .font(egui::TextStyle::Monospace),
                    );
                });
                if ui
                    .add(
                        egui::Button::new(RichText::new("比较差异").color(Color32::WHITE))
                            .fill(p.accent),
                    )
                    .clicked()
                {
                    match compare_text(&self.diff_state.diff_before, &self.diff_state.diff_after) {
                        Ok(output) => {
                            self.diff_state.diff_output = output;
                            self.diff_state.message.clear();
                        }
                        Err(error) => self.diff_state.message = error.to_string(),
                    }
                }
                if !self.diff_state.message.is_empty() {
                    ui.label(RichText::new(&self.diff_state.message).color(p.red));
                }
                ui.horizontal(|ui| {
                    ui.label("Unified Diff");
                    if ui.small_button("复制差异").clicked() {
                        ctx.copy_text(self.diff_state.diff_output.clone());
                    }
                });
                ui.add_sized(
                    [ui.available_width(), 260.0],
                    egui::TextEdit::multiline(&mut self.diff_state.diff_output)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false),
                );
            });
    }

    fn network_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.colors;
        ui.heading(RichText::new("网络诊断").size(28.0));
        ui.label(
            RichText::new("从本机解析 DNS 或测试单个 TCP 端口；不会扫描端口范围").color(p.muted),
        );
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.network_state.pane, NetworkPane::Dns, "DNS 解析");
            ui.selectable_value(&mut self.network_state.pane, NetworkPane::Tcp, "TCP 连通性");
        });
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(p.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| {
                ui.label("主机名或 IP");
                ui.add(
                    egui::TextEdit::singleline(&mut self.network_state.host)
                        .desired_width(360.0)
                        .hint_text("localhost"),
                );
                if self.network_state.pane == NetworkPane::Tcp {
                    ui.label("端口 (1–65535)");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.network_state.port)
                            .desired_width(120.0),
                    );
                }
                let label = if self.network_state.busy {
                    "执行中…"
                } else if self.network_state.pane == NetworkPane::Dns {
                    "解析 DNS"
                } else {
                    "测试连接"
                };
                if ui
                    .add_enabled(
                        !self.network_state.busy,
                        egui::Button::new(RichText::new(label).color(Color32::WHITE))
                            .fill(p.accent),
                    )
                    .clicked()
                {
                    self.network_state.busy = true;
                    self.network_state.output = "正在执行，请稍候…".into();
                    self.network_state.request_id += 1;
                    let request_id = self.network_state.request_id;
                    let pane = self.network_state.pane;
                    let host = self.network_state.host.clone();
                    let port = self.network_state.port.clone();
                    let tx = self.event_tx.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let result = match pane {
                            NetworkPane::Dns => resolve_host(&host),
                            NetworkPane::Tcp => test_tcp(&host, &port),
                        }
                        .map_err(|error| error.to_string());
                        let _ = tx.send(BackgroundEvent::NetworkResult(request_id, result));
                        ctx.request_repaint();
                    });
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label("结果");
                    if ui.small_button("复制").clicked() {
                        ctx.copy_text(self.network_state.output.clone());
                    }
                });
                ui.add_sized(
                    [ui.available_width(), 280.0],
                    egui::TextEdit::multiline(&mut self.network_state.output)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false),
                );
            });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading(RichText::new("设置").size(28.0));
        ui.label(RichText::new("使用 Zi DevTools 独立的配置与状态目录").color(self.colors.muted));
        ui.add_space(20.0);
        egui::Frame::new()
            .fill(self.colors.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Local Services 配置").strong());
                ui.add_space(8.0);
                ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut self.config_path_input),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("加载配置").fill(self.colors.accent))
                        .clicked()
                    {
                        let path = PathBuf::from(self.config_path_input.trim());
                        match load_config(&path)
                            .and_then(|config| self.manager.replace_config(config))
                        {
                            Ok(()) => {
                                self.config_error = None;
                                match TrayController::new(
                                    self.manager.config_snapshot().services.values(),
                                ) {
                                    Ok(tray) => self.tray = Some(tray),
                                    Err(error) => {
                                        self.notification = format!("托盘初始化失败：{error}");
                                        self.notification_error = true;
                                    }
                                }
                                self.last_refresh = Instant::now() - Duration::from_secs(30);
                            }
                            Err(error) => self.config_error = Some(error.to_string()),
                        }
                    }
                    if ui.button("打开所在目录").clicked()
                        && let Some(parent) = PathBuf::from(self.config_path_input.trim()).parent()
                    {
                        let _ = open::that(parent);
                    }
                    if ui.button("打开状态目录").clicked() {
                        let _ = open::that(&self.manager.config_snapshot().state_dir);
                    }
                });
                if let Some(error) = &self.config_error {
                    ui.label(RichText::new(error).color(self.colors.red));
                }
                let config = self.manager.config_snapshot();
                ui.separator();
                ui.label(format!("当前服务：{}", config.services.len()));
                ui.label(format!("状态目录：{}", config.state_dir.display()));
                ui.label(
                    RichText::new(
                        "配置热重载已启用；服务 YAML 的 env 字段不展开，仅可手工预览 config_files。",
                    )
                        .color(self.colors.muted),
                );
            });
        ui.add_space(14.0);
        egui::Frame::new()
            .fill(self.colors.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| {
                ui.label(RichText::new("外观").strong());
                ui.label(
                    RichText::new("切换后立即生效，仅影响本程序界面").color(self.colors.muted),
                );
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.theme == Theme::Dark, Theme::Dark.label())
                        .clicked()
                    {
                        self.set_theme(ctx, Theme::Dark);
                    }
                    if ui
                        .selectable_label(self.theme == Theme::Light, Theme::Light.label())
                        .clicked()
                    {
                        self.set_theme(ctx, Theme::Light);
                    }
                });
            });
        ui.add_space(14.0);
        egui::Frame::new()
            .fill(self.colors.card)
            .corner_radius(12.0)
            .inner_margin(18.0)
            .show(ui, |ui| {
                ui.label(RichText::new("托盘行为").strong());
                ui.label("关闭窗口时程序继续驻留托盘，托管服务保持运行。左键恢复窗口；右键可搜索工具、打开收藏/最近/常用、按分类访问插件和内置工具，并控制服务。");
                ui.label(
                    RichText::new("收藏按添加顺序、最近按打开时间、常用按次数排列；快捷菜单最多显示 8 / 8 / 6 项，可进入完整列表。原生菜单外观跟随 Windows。选择“退出”才会结束本程序。")
                        .color(self.colors.muted),
                );
                if ui.button("立即隐藏到托盘").clicked() {
                    self.hide_to_tray(ctx);
                }
            });
    }

    fn overlays(&mut self, ctx: &egui::Context) {
        let p = self.colors;
        if let Some(service_id) = self.log_view.clone() {
            let log_path = self
                .statuses
                .iter()
                .find(|status| status.id == service_id)
                .map(|status| status.log_path.clone());
            let mut open = true;
            egui::Window::new(format!("服务日志 · {service_id}"))
                .open(&mut open)
                .default_size([900.0, 580.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("刷新").clicked() {
                            self.request_logs(service_id.clone());
                        }
                        if ui.button("清空日志").clicked() {
                            self.clear_logs(service_id.clone());
                            self.log_text.clear();
                        }
                        if ui.button("复制").clicked() {
                            ctx.copy_text(self.log_text.clone());
                        }
                        if ui
                            .add_enabled(
                                log_path.as_ref().is_some_and(|path| path.is_file()),
                                egui::Button::new("打开完整日志文件"),
                            )
                            .clicked()
                            && let Some(path) = &log_path
                            && let Err(error) = open::that(path)
                        {
                            self.notification = format!("打开日志失败：{error}");
                            self.notification_error = true;
                        }
                    });
                    ui.label(
                        RichText::new("预览末尾最多 5000 行 / 4 MiB；原始文件不截断")
                            .small()
                            .color(p.muted),
                    );
                    ui.separator();
                    egui::ScrollArea::both()
                        .max_height(480.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&self.log_text).monospace())
                                    .selectable(true),
                            );
                        });
                });
            if !open {
                self.log_view = None;
            }
        }
        if let Some((service_id, path)) = self.config_view.clone() {
            let mut open = true;
            egui::Window::new(format!("配置预览 · {service_id} · {path}"))
                .open(&mut open)
                .default_size([900.0, 580.0])
                .show(ctx, |ui| {
                    ui.label(RichText::new("只读预览；最多显示 80 KB").color(p.muted));
                    ui.add(
                        egui::TextEdit::multiline(&mut self.config_text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(28),
                    );
                });
            if !open {
                self.config_view = None;
            }
        }
    }
}

fn execute_batch(manager: &ServiceManager, start: bool) -> Result<ActionResult, String> {
    let mut ok = 0;
    let mut skipped = 0;
    let mut failures = Vec::new();
    for id in manager.service_ids() {
        match manager.service_status(&id) {
            Ok(status)
                if (start && status.state == ServiceState::Stopped)
                    || (!start && status.managed) =>
            {
                let result = if start {
                    manager.start(&id)
                } else {
                    manager.stop(&id)
                };
                match result {
                    Ok(_) => ok += 1,
                    Err(error) => failures.push(format!("{id}: {error}")),
                }
            }
            Ok(_) => skipped += 1,
            Err(error) => failures.push(format!("{id}: {error}")),
        }
    }
    let operation = if start { "启动" } else { "停止" };
    if failures.is_empty() {
        Ok(ActionResult {
            service_id: "all".to_owned(),
            message: format!("批量{operation}完成：成功 {ok}，跳过 {skipped}"),
        })
    } else {
        let detail = failures
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("；");
        Err(format!(
            "批量{operation}完成：成功 {ok}，跳过 {skipped}，失败 {}。{detail}",
            failures.len()
        ))
    }
}

fn start_tray_bridge(
    manager: Arc<ServiceManager>,
    tx: Sender<BackgroundEvent>,
    ctx: egui::Context,
    window_handle: Option<isize>,
    stop: Arc<AtomicBool>,
    exit_requested: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            for action in TrayController::poll_actions() {
                match action {
                    TrayAction::ShowWindow => {
                        restore_main_window(window_handle, &ctx);
                    }
                    TrayAction::OpenTool(tool) => {
                        let _ = tx.send(BackgroundEvent::NavigateTool(tool));
                        restore_main_window(window_handle, &ctx);
                    }
                    action @ (TrayAction::Search
                    | TrayAction::Settings
                    | TrayAction::OpenEntry(_)
                    | TrayAction::Collection(_)) => {
                        let _ = tx.send(BackgroundEvent::TrayNavigate(action));
                        restore_main_window(window_handle, &ctx);
                    }
                    TrayAction::Exit => {
                        exit_requested.store(true, Ordering::Release);
                        restore_main_window(window_handle, &ctx);
                    }
                    action => {
                        let manager = Arc::clone(&manager);
                        let tx = tx.clone();
                        let ctx = ctx.clone();
                        std::thread::spawn(move || {
                            let result = match action {
                                TrayAction::Start(id) => {
                                    manager.start(&id).map_err(|error| format!("{id}: {error}"))
                                }
                                TrayAction::Stop(id) => {
                                    manager.stop(&id).map_err(|error| format!("{id}: {error}"))
                                }
                                TrayAction::Restart(id) => manager
                                    .restart(&id)
                                    .map_err(|error| format!("{id}: {error}")),
                                TrayAction::StartAll | TrayAction::StopAll => {
                                    execute_batch(&manager, matches!(action, TrayAction::StartAll))
                                }
                                TrayAction::ShowWindow
                                | TrayAction::OpenTool(_)
                                | TrayAction::Search
                                | TrayAction::Settings
                                | TrayAction::OpenEntry(_)
                                | TrayAction::Collection(_)
                                | TrayAction::Exit => unreachable!(),
                            };
                            let _ = tx.send(BackgroundEvent::Action(result));
                            ctx.request_repaint();
                        });
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(60));
        }
    });
}

#[cfg(windows)]
fn main_window_handle(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    cc.window_handle()
        .ok()
        .and_then(|window| match window.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
}

#[cfg(not(windows))]
fn main_window_handle(_cc: &eframe::CreationContext<'_>) -> Option<isize> {
    None
}

fn restore_main_window(window_handle: Option<isize>, ctx: &egui::Context) {
    #[cfg(windows)]
    if let Some(handle) = window_handle {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsWindow, SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        let window = handle as windows_sys::Win32::Foundation::HWND;
        // The handle comes from this app's eframe CreationContext and is checked before use.
        unsafe {
            if IsWindow(window) != 0 {
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window);
                ctx.request_repaint();
                return;
            }
        }
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
}

fn hide_main_window(window_handle: Option<isize>) {
    #[cfg(windows)]
    if let Some(handle) = window_handle {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SW_HIDE, ShowWindow};
        let window = handle as windows_sys::Win32::Foundation::HWND;
        unsafe {
            if IsWindow(window) != 0 {
                ShowWindow(window, SW_HIDE);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window_handle;
}

impl Drop for DevToolsApp {
    fn drop(&mut self) {
        self.tray_bridge_stop.store(true, Ordering::Release);
    }
}

impl eframe::App for DevToolsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(message) = self.frameworks.poll() {
            self.toast = Some((message, Instant::now()));
        }
        if self.frameworks.is_running() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.page != Page::Frameworks {
            self.frameworks.clear_token();
        }
        if let Some(message) = self.plugins.poll() {
            self.toast = Some((message, Instant::now()));
        }
        if self.plugins.is_running() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.page != Page::Plugins {
            self.plugins.clear_token();
        }
        self.drain_events(ctx);
        let model = Navigation::new(
            self.entries("")
                .into_iter()
                .map(|e| TrayEntry {
                    id: e.id,
                    title: e.title,
                    category: e.category,
                })
                .collect(),
            &self.preferences,
        );
        if let Some(tray) = &mut self.tray
            && let Err(error) = tray.sync_navigation(model)
        {
            self.notification = format!("托盘快捷菜单更新失败：{error}");
            self.notification_error = true;
        }

        if self.tray_exit_requested.swap(false, Ordering::AcqRel) {
            self.quit_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.last_refresh.elapsed() >= Duration::from_secs(2) {
            self.maybe_reload_config();
            self.request_refresh();
        }
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quit_requested
            && self.tray.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_to_tray(ctx);
        }

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)) {
            self.open_launcher();
        }
        egui::TopBottomPanel::bottom("status-bar")
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(20, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some((message, at)) = &self.toast {
                        if at.elapsed() < Duration::from_secs(4) {
                            ui.label(RichText::new(message).color(self.colors.green));
                        } else {
                            ui.label(RichText::new("就绪").small().color(self.colors.muted));
                        }
                    } else {
                        ui.label(RichText::new("就绪").small().color(self.colors.muted));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Ctrl K  搜索工具    ·    Ctrl Enter  执行转换")
                                .size(11.0)
                                .color(self.colors.muted),
                        );
                    });
                });
            });
        self.sidebar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.colors.bg).inner_margin(24.0))
            .show(ctx, |ui| match self.page {
                Page::Home => {
                    egui::ScrollArea::vertical()
                        .id_salt("home-scroll")
                        .show(ui, |ui| self.home_page(ui));
                }
                Page::Data => {
                    egui::ScrollArea::vertical()
                        .id_salt("data-page")
                        .show(ui, |ui| self.data_state.ui(ui, ctx));
                }
                Page::Files => {
                    egui::ScrollArea::vertical()
                        .id_salt("files-page")
                        .show(ui, |ui| self.file_state.ui(ui, ctx));
                }
                Page::Services => self.services_page(ui),
                Page::SmallTools => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.small_tools_page(ui, ctx));
                }
                Page::EncodingTools => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.encoding_tools_page(ui, ctx));
                }
                Page::Http => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.http_page(ui, ctx));
                }
                Page::Diff => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.diff_page(ui, ctx));
                }
                Page::Network => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.network_page(ui, ctx));
                }
                Page::Settings => self.settings_page(ui, ctx),
                Page::Plugins => {
                    let previous = self.plugins.selected.clone();
                    egui::ScrollArea::vertical()
                        .id_salt("plugins-page")
                        .show(ui, |ui| self.plugins.ui(ui, !self.launcher_open));
                    if self.plugins.selected != previous
                        && let Some(id) = self.plugins.selected.clone()
                    {
                        self.visit(&id);
                    }
                }
                Page::Frameworks => {
                    let before = self.frameworks.selected;
                    ui.horizontal(|ui| {
                        let id = self.frameworks.selected.id();
                        if ui
                            .button(if self.preferences.favorites.iter().any(|s| s == id) {
                                "★ 已收藏"
                            } else {
                                "☆ 收藏当前诊断"
                            })
                            .clicked()
                        {
                            self.toggle_favorite(id);
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt(("frameworks", self.frameworks.selected.id()))
                        .show(ui, |ui| self.frameworks.ui(ui, !self.launcher_open));
                    if self.frameworks.selected != before {
                        self.visit(self.frameworks.selected.id());
                    }
                }
                Page::Integrations => {
                    egui::ScrollArea::vertical()
                        .id_salt("integrations-page")
                        .show(ui, |ui| self.integrations.ui(ui));
                }
            });
        self.overlays(ctx);
        self.launcher(ctx);
        if self.clear_tool_confirm {
            egui::Window::new("清空当前工具？")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("将清空当前工具的输入、输出和错误信息。其他工具的草稿会保留。");
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            self.clear_tool_confirm = false;
                        }
                        if ui.button("确认清空").clicked() {
                            self.tool_state.input.clear();
                            self.tool_state.pattern.clear();
                            self.tool_state.output.clear();
                            self.tool_state.message.clear();
                            self.tool_state.qr_image = None;
                            self.qr_texture = None;
                            self.clear_tool_confirm = false;
                        }
                    });
                });
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn configure_ui(ctx: &egui::Context, theme: Theme) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
    ];
    if let Some((_, bytes)) = candidates
        .iter()
        .find_map(|path| fs::read(path).ok().map(|bytes| (*path, bytes)))
    {
        fonts.font_data.insert(
            "windows-cjk".to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("windows-cjk".to_owned());
        }
    }
    ctx.set_fonts(fonts);
    apply_theme(ctx, theme);
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let p = palette(theme);
    let mut visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.card;
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(p.muted);
    visuals.extreme_bg_color = p.input_bg;
    visuals.faint_bg_color = p.surface;
    visuals.code_bg_color = p.input_bg;
    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.fg_stroke.color = p.text;
    visuals.widgets.active.bg_stroke.color = p.accent_hover;
    visuals.widgets.hovered.bg_fill = p.accent_hover;
    visuals.widgets.hovered.fg_stroke.color = p.text;
    visuals.widgets.inactive.bg_fill = p.surface;
    visuals.widgets.inactive.fg_stroke.color = p.text;
    visuals.widgets.noninteractive.bg_fill = p.panel;
    visuals.widgets.noninteractive.fg_stroke.color = p.text;
    visuals.widgets.inactive.weak_bg_fill = p.surface;
    visuals.widgets.hovered.weak_bg_fill = if theme == Theme::Dark {
        Color32::from_rgb(52, 67, 88)
    } else {
        Color32::from_rgb(207, 219, 235)
    };
    visuals.widgets.active.weak_bg_fill = p.surface;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.surface);
    visuals.selection.bg_fill = p.accent;
    visuals.selection.stroke.color = Color32::WHITE;
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(9.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(7);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(7);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(7);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
    style.spacing.window_margin = egui::Margin::same(18);
    style.visuals.window_corner_radius = egui::CornerRadius::same(12);
    ctx.set_style(style);
}

fn fallback_config(path: PathBuf) -> crate::config::DashboardConfig {
    crate::config::DashboardConfig {
        path,
        state_dir: crate::config::default_state_dir(),
        services: Default::default(),
        modified: None,
    }
}

fn other_instance_count() -> usize {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .processes()
        .values()
        .filter(|process| {
            process.pid().as_u32() != std::process::id()
                && process
                    .name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("ZiDevTools.exe")
        })
        .count()
}

fn nav_button(ui: &mut egui::Ui, page: &mut Page, target: Page, text: &str) {
    let selected = *page == target;
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            let text = RichText::new(text).size(15.0).color(if selected {
                Color32::WHITE
            } else {
                ui.visuals().text_color()
            });
            ui.add_sized(
                [ui.available_width(), 36.0],
                egui::Button::new(text)
                    .right_text("")
                    .selected(selected)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(8),
            )
        })
        .inner;
    if response.clicked() {
        *page = target;
    }
    ui.add_space(3.0);
}

fn workbench_tab(ui: &mut egui::Ui, selected: &mut HttpPane, target: HttpPane, text: &str) {
    let accent = ui.visuals().selection.bg_fill;
    let panel = ui.visuals().panel_fill;
    let active = *selected == target;
    let button = egui::Button::new(RichText::new(text).size(14.0))
        .selected(active)
        .fill(if active { accent } else { panel });
    if ui.add(button).clicked() {
        *selected = target;
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: usize, color: Color32) {
    let card = ui.visuals().window_fill;
    egui::Frame::new()
        .fill(card)
        .corner_radius(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(120.0);
            ui.label(
                RichText::new(value.to_string())
                    .size(24.0)
                    .strong()
                    .color(color),
            );
            ui.label(RichText::new(label).color(ui.visuals().weak_text_color()));
        });
}

fn action_label(action: &str) -> &'static str {
    match action {
        "start" => "启动",
        "stop" => "停止",
        "restart" => "重启",
        _ => "操作",
    }
}

#[cfg(test)]
mod service_filter_tests {
    use super::*;

    #[test]
    fn pagination_reaches_last_tool_and_clamps_after_filtering() {
        assert_eq!(page_bounds(100, 5), (90..100, 6));
        assert_eq!(page_bounds(0, 99), (0..0, 1));
        assert_eq!(page_bounds(18, 1), (0..18, 1));
        assert_eq!(page_bounds(19, 1), (18..19, 2));
    }
    #[test]
    fn registry_has_unique_categories_and_multiterm_search() {
        let entries = catalog();
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), entries.len());
        assert!(
            entries
                .iter()
                .all(|e| crate::plugins::CATEGORIES.contains(&e.category.as_str()))
        );
        let json = entries.iter().find(|e| e.id == "json").unwrap();
        assert!(json.score("JSON format").is_some());
        assert!(json.score("json unmatched").is_none());
        assert!(json.score("json").unwrap() > json.score("format").unwrap());
    }
    #[test]
    fn separates_managed_external_and_stopped_services() {
        assert!(ServiceFilter::Managed.matches(ServiceState::Running, true));
        assert!(!ServiceFilter::Managed.matches(ServiceState::External, false));
        assert!(ServiceFilter::External.matches(ServiceState::PortOpen, false));
        assert!(!ServiceFilter::External.matches(ServiceState::Stopped, false));
        assert!(ServiceFilter::Stopped.matches(ServiceState::Stopped, false));
    }
}

#[cfg(all(test, windows))]
mod native_window_tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, IsWindowVisible,
    };

    #[test]
    fn restores_a_hidden_windows_window() {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let title: Vec<u16> = "Zi DevTools restore test\0".encode_utf16().collect();
        // The predefined STATIC window class is sufficient for testing visibility changes.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!window.is_null());
        assert_eq!(unsafe { IsWindowVisible(window) }, 0);
        restore_main_window(Some(window as isize), &egui::Context::default());
        assert_ne!(unsafe { IsWindowVisible(window) }, 0);
        hide_main_window(Some(window as isize));
        assert_eq!(unsafe { IsWindowVisible(window) }, 0);
        unsafe { DestroyWindow(window) };
    }
}
