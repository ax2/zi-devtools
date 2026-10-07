mod commands;
mod handoff;
mod launcher;
mod navigation;
mod registry;
mod service_batch;
mod service_editor;
mod service_logs;
#[cfg(feature = "ui-preview")]
mod service_preview;
mod services;
mod task_center;

use registry::{Page, ToolEntry, catalog};

#[cfg(feature = "ui-preview")]
use crate::tools::run_tool;
#[cfg(feature = "ui-preview")]
use std::path::Path;
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
    tools::{ToolKind, ToolState, generate_qr, generate_uuid},
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
enum ServiceFilter {
    #[default]
    All,
    Managed,
    External,
    Stopped,
    Attention,
}

impl ServiceFilter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Managed,
        Self::External,
        Self::Stopped,
        Self::Attention,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Managed => "托管运行",
            Self::External => "外部占用",
            Self::Stopped => "已停止",
            Self::Attention => "需要关注",
        }
    }

    fn matches(self, state: ServiceState, managed: bool, attention: bool) -> bool {
        match self {
            Self::All => true,
            Self::Managed => managed,
            Self::External => !managed && state.is_available(),
            Self::Stopped => state == ServiceState::Stopped,
            Self::Attention => attention,
        }
    }
}

enum BackgroundEvent {
    Statuses(u64, Vec<ServiceStatus>),
    Action(Result<ActionResult, String>),
    ServiceAction(String, Result<ActionResult, String>),
    ServiceBatch(u64, service_batch::Event),
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
    #[cfg(feature = "ui-preview")]
    preview_panel_frames: usize,
    #[cfg(feature = "ui-preview")]
    preview_sidebar: std::collections::HashMap<&'static str, (egui::Rect, egui::Rect)>,
    #[cfg(feature = "ui-preview")]
    preview_services: std::collections::HashMap<&'static str, (egui::Rect, egui::Rect)>,
    #[cfg(feature = "ui-preview")]
    preview_service_rows: usize,
    #[cfg(feature = "ui-preview")]
    preview_tray_capture: Option<PathBuf>,
    #[cfg(feature = "ui-preview")]
    preview_tray_workflow: Option<(PathBuf, usize)>,
    #[cfg(feature = "ui-preview")]
    preview_text_copy: Option<(egui::Rect, bool)>,
    hotkey: crate::hotkey::Service,
    hotkey_edit: crate::hotkey::Setting,
    hotkey_status: String,
    recorder_hotkeys: crate::hotkey::RecorderHotkeys,
    recorder_hotkey_status: String,
    quick_open: bool,
    quick_active: Arc<AtomicBool>,
    quick_focus: bool,
    quick_had_focus: bool,
    quick_opened: Instant,
    quick_tab: String,
    quick_context: bool,
    quick_position: Option<egui::Pos2>,
    quick_size: egui::Vec2,
    intake: crate::intake::State,
    tasks: crate::tasks::Center,
    handoff: Option<handoff::Transfer>,
    tray: Option<TrayController>,
    page: Page,
    statuses: Vec<ServiceStatus>,
    selected_service: Option<String>,
    search: String,
    service_filter: ServiceFilter,
    service_compact: Option<bool>,
    service_pending: std::collections::HashMap<String, &'static str>,
    service_batch: service_batch::State,
    event_tx: Sender<BackgroundEvent>,
    event_rx: Receiver<BackgroundEvent>,
    refresh_inflight: bool,
    refresh_generation: u64,
    refresh_cancel: Arc<AtomicBool>,
    last_refresh: Instant,
    last_preferences_refresh: Instant,
    last_workflow_retry: Instant,
    #[cfg(feature = "ui-preview")]
    preview_shared_theme: Option<Theme>,
    notification: String,
    notification_error: bool,
    startup_warning: Option<String>,
    log_view: Option<String>,
    log_text: String,
    log_error: bool,
    log_lines: service_logs::LogLines,
    service_editor: service_editor::State,
    log_filter: String,
    log_follow: bool,
    log_auto: bool,
    log_inflight: bool,
    log_clear_confirm: bool,
    last_logs: Instant,
    config_view: Option<(String, String)>,
    config_text: String,
    tool_state: ToolState,
    calculator: crate::calculator::State,
    clipboard: crate::clipboard::State,
    clock: crate::clock::State,
    updates: crate::updates::State,
    portable_update: crate::updates::portable_ui::State,
    msi_update: crate::updates::msi_ui::State,
    delta_update: crate::updates::delta_ui::State,
    prefix: commands::Prefix,
    http_state: HttpWorkbenchState,
    diff_state: DiffState,
    network_state: NetworkState,
    qr_texture: Option<egui::TextureHandle>,
    config_path_input: String,
    config_error: Option<String>,
    quit_requested: bool,
    workspace_exit_confirm: bool,
    tray_bridge_stop: Arc<AtomicBool>,
    tray_exit_requested: Arc<AtomicBool>,
    window_handle: Option<isize>,
    theme: Theme,
    colors: Palette,
    preferences: Preferences,
    preferences_path: PathBuf,
    tool_search: String,
    library_query: String,
    launcher_query: String,
    launcher_open: bool,
    launcher_focus: bool,
    #[cfg(feature = "ui-preview")]
    workflow_bookmark_button: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    workflow_recent_button: Option<egui::Rect>,
    launcher_index: usize,
    launcher_scope: launcher::Scope,
    #[cfg(feature = "ui-preview")]
    launcher_scope_buttons: [Option<egui::Rect>; 4],
    toast: Option<(String, Instant)>,
    data_state: crate::workbench::sessions::Workspace,
    planner: crate::planner::State,
    planner_active: Arc<AtomicBool>,
    file_state: FileState,
    clear_tool_confirm: bool,
    plugins: crate::plugin_ui::PluginState,
    mcp: crate::mcp_ui::McpState,
    agent: crate::agent_ui::State,
    agent_records: crate::agent_record_ui::State,
    recorder: crate::recorder_ui::RecorderState,
    images: crate::image_tools::State,
    markdown: crate::markdown_preview::State,
    file_encoding: crate::file_encoding::State,
    checksum_manifest: crate::checksum_manifest::State,
    disk_inspector: crate::disk_inspector::State,
    duplicate_finder: crate::duplicate_finder::State,
    directory_compare: crate::directory_compare::State,
    sqlite_browser: crate::sqlite_browser::State,
    ascii_codes: crate::character_tools::AsciiState,
    symbols: crate::character_tools::SymbolState,
    ascii_art: crate::character_tools::ArtState,
    knowledge_sources: crate::knowledge_sources::State,
    document_ingestion: crate::document_ingestion::State,
    knowledge_index: crate::knowledge_index::State,
    vector_index: crate::vector_index::State,
    knowledge_search: crate::knowledge_search::State,
    hybrid_search: crate::hybrid_search::State,
    knowledge_answer: crate::knowledge_answer::State,
    embedding: crate::embedding::State,
    knowledge_capture: crate::knowledge_capture::State,
    knowledge_eval: crate::knowledge_eval::State,
    integrations: crate::integrations::IntegrationState,
    frameworks: crate::framework::State,
    home_filter: String,
    home_category: String,
    home_query_key: (String, String, String),
}

impl DevToolsApp {
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_overlay_prepare(&mut self, ctx: &egui::Context, index: usize) {
        self.preview_scene(
            ctx,
            348 + index % 2,
            std::path::PathBuf::from("synthetic.txt"),
        );
        assert!(hide_main_window(self.window_handle));
        self.images.preview_overlay_fixture(ctx, index);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_capture_start(&mut self) {
        self.images.request_screenshot_capture();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_busy(&self) -> bool {
        self.images.screenshot_busy()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_overlay_active(&self) -> bool {
        self.images.preview_overlay_active()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_overlay_dimensions(&self) -> [i32; 2] {
        self.images.preview_overlay_dimensions()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_overlay_key(&self, key: u32, scan: u32) {
        self.images.preview_overlay_key(key, scan);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_overlay_pointer(&self, x: i32, y: i32, kind: u8) {
        self.images.preview_overlay_pointer(x, y, kind);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_screenshot_overlay_check(&self, phase: u8) {
        self.images.preview_overlay_check(phase);
        if phase == 0 {
            assert!(main_window_cloaked(self.window_handle));
        }
        if matches!(phase, 1 | 2 | 4) {
            assert!(!main_window_cloaked(self.window_handle));
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_backup_click_position(&self, index: usize) -> Option<egui::Pos2> {
        self.planner.preview_backup_rects[index].map(|r| r.center())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_backup_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Notes;
        self.planner.preview_backup_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_files_cancel_position(&self) -> Option<egui::Pos2> {
        self.planner.preview_export_cancel_rect.map(|r| r.center())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_agenda_click_position(&self) -> Option<egui::Pos2> {
        self.planner.preview_agenda_rect.map(|r| r.center())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_agenda_assert(&self) {
        self.planner.preview_agenda_assert();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_recurrence_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) =
            self.planner.preview_recurrence_rects[index].expect("recurrence control rendered");
        assert!(clip.contains_rect(rect), "recurrence control clipped");
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_recurrence_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Calendar;
        self.planner.preview_recurrence_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_interval_position(&self, save: bool) -> egui::Pos2 {
        let (rect, clip) = if save {
            self.planner.preview_recurrence_rects[1]
        } else {
            self.planner.preview_interval_rect
        }
        .expect("interval control rendered");
        assert!(clip.contains_rect(rect), "interval control clipped");
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_interval_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Calendar;
        self.planner.preview_interval_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_actions_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = match index {
            0 => self.planner.preview_recurrence_rects[1],
            1 => self.planner.preview_discard_rect,
            2 => self.planner.preview_title_rect,
            _ => unreachable!(),
        }
        .expect("editor action control rendered");
        assert!(clip.contains_rect(rect), "editor action control clipped");
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_actions_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Notes;
        self.planner.preview_actions_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_cutoff_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = if index == 2 {
            self.planner.preview_recurrence_rects[1]
        } else {
            self.planner.preview_cutoff_rects[index]
        }
        .expect("cutoff control rendered");
        assert!(clip.contains_rect(rect), "cutoff control clipped");
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_cutoff_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Calendar;
        self.planner.preview_cutoff_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_cutoff_interop(&self, folder: &Path) {
        self.planner.preview_cutoff_interop(folder);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ics_position(&self, index: usize) -> egui::Pos2 {
        self.planner.preview_ics_rects[index]
            .expect("ICS control rendered")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ics_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Calendar;
        self.planner.preview_ics_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ics_interop(&self, folder: &Path) {
        self.planner.preview_ics_interop(folder);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_files_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Notes;
        self.planner.preview_files_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_purge_click_position(&self, confirm: bool) -> Option<egui::Pos2> {
        self.planner
            .preview_purge_rects
            .map(|r| r[usize::from(confirm)].center())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_purge_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Notes;
        self.planner.preview_purge_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_week_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = self.planner.preview_week_rects[index].expect("week control rendered");
        assert!(
            clip.contains_rect(rect),
            "week control clipped: {rect:?}, {clip:?}"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_week_smoke(&mut self, phase: u8) {
        self.page = Page::Calendar;
        self.planner.preview_week_smoke(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_planner_navigation_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = self.planner.preview_navigation_rects[index].unwrap();
        assert!(clip.contains_rect(rect), "navigation button clipped");
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_planner_navigation_smoke(&mut self, phase: u8, week: bool) {
        self.page = Page::Calendar;
        self.planner.preview_navigation_smoke(phase, week);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_week_scroll_position(&self, index: usize) -> egui::Pos2 {
        self.planner.preview_week_scroll_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_week_scroll_smoke(&mut self, phase: u8) {
        self.page = Page::Calendar;
        self.planner.preview_week_scroll_smoke(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_convert_position(&self) -> egui::Pos2 {
        let (rect, clip) = self
            .planner
            .preview_convert_rect
            .expect("convert control rendered");
        assert!(clip.contains_rect(rect));
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_convert_smoke(&mut self, phase: u8, calendar: bool) -> bool {
        if phase == 0 {
            self.page = if calendar {
                Page::Calendar
            } else {
                Page::Notes
            };
        } else {
            assert_eq!(
                self.page,
                if calendar {
                    Page::Notes
                } else {
                    Page::Calendar
                }
            );
        }
        self.planner.preview_convert_smoke(phase, calendar)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_duplicate_position(&self) -> egui::Pos2 {
        let (rect, clip) = self
            .planner
            .preview_duplicate_rect
            .expect("copy control rendered");
        assert!(
            clip.contains_rect(rect),
            "copy button clipped: rect={rect:?}, clip={clip:?}"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_duplicate_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Calendar;
        self.planner.preview_duplicate_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_listing_position(&self, index: usize) -> egui::Pos2 {
        self.planner.preview_list_rects[index]
            .expect("list control rendered")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_listing_smoke(&mut self, phase: u8) {
        self.page = Page::Notes;
        self.planner.preview_listing_smoke(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_snooze_position(&self, index: usize) -> egui::Pos2 {
        self.planner.preview_snooze_rects[index]
            .expect("snooze control rendered")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_conflict_copy(&mut self, phase: u8) -> bool {
        self.page = Page::Notes;
        self.planner.preview_conflict_copy(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_conflict_copy_position(&self) -> egui::Pos2 {
        let (rect, clip) = self
            .planner
            .preview_conflict_copy_rect
            .expect("conflict button rendered");
        assert!(
            clip.contains_rect(rect),
            "conflict action must fit fixed dock"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_conflict_review(&mut self, phase: u8, calendar: bool, deleted: bool) -> bool {
        self.page = if calendar {
            Page::Calendar
        } else {
            Page::Notes
        };
        self.planner
            .preview_conflict_review(phase, calendar, deleted)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_conflict_review_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) =
            self.planner.preview_conflict_review_rects[index].expect("comparison control rendered");
        assert!(
            clip.contains_rect(rect),
            "comparison control must remain visible: {rect:?} {clip:?}"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_snooze_smoke(&mut self, phase: u8) -> bool {
        self.page = Page::Home;
        self.planner.preview_snooze_smoke(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_reminder_click_position(&self) -> Option<egui::Pos2> {
        self.planner
            .preview_open_reminder_rect
            .map(|rect| rect.center())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_reminder_click_start(&mut self, ctx: &egui::Context) {
        self.preview_scene(ctx, 202, PathBuf::new());
        self.page = Page::Home;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_reminder_click_assert(&self) {
        assert_eq!(self.page, Page::Calendar);
        assert!(self.planner.preview_reminder_opened());
        println!(
            "PASS reminder UI click: opens exact calendar event from Home without acknowledging reminder"
        );
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_planner_editor(&mut self) {
        self.planner.preview_focus_editor();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_planner_timer(&mut self, ctx: &egui::Context, minimize: bool) {
        self.planner.preview_timer();
        self.page = Page::Home;
        self.quick_open = false;
        if minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        } else {
            self.hide_to_tray(ctx);
        }
        let handle = self.window_handle;
        let delivered = self.planner.preview_delivered.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            #[cfg(windows)]
            {
                let hidden = if minimize {
                    handle.is_some_and(|h| unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(h as _) != 0
                    })
                } else {
                    main_window_cloaked(handle)
                };
                if !hidden {
                    eprintln!("FAIL planner initial hidden/minimized state");
                    std::process::exit(1);
                }
            }
            std::thread::sleep(Duration::from_secs(6));
            #[cfg(windows)]
            let visible = !main_window_cloaked(handle)
                && handle.is_some_and(|h| unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(h as _) == 0
                });
            #[cfg(not(windows))]
            let visible = true;
            if visible && delivered.load(Ordering::Acquire) {
                println!(
                    "PASS planner real timed alarm: in-memory fixture, minimized={minimize}, delivered and restored without harness repaint"
                );
                std::process::exit(0);
            }
            eprintln!("FAIL planner alarm did not restore hidden/minimized window");
            std::process::exit(1);
        });
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_begin_recorder_selection(&mut self, ctx: &egui::Context) {
        self.recorder.preview_begin_selection(ctx);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_recorder_region(&self) -> Option<crate::recorder::Region> {
        self.recorder.preview_region()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_start_recorder_auto_minimize(
        &mut self,
        folder: &Path,
        countdown_seconds: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.tray.is_some(), "托盘不可用，无法验证自动最小化");
        self.recorder
            .preview_start_auto_minimize(folder, countdown_seconds)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_stop_recorder(&mut self) {
        self.recorder.request_stop();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_recorder_last_file(&self) -> Option<PathBuf> {
        self.recorder.preview_last_file()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_recorder_status(&self) -> crate::recorder_ui::TrayRecordingStatus {
        self.recorder.tray_status()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_open_quick(&mut self, ctx: &egui::Context) {
        self.preview_panel_frames = 0;
        self.open_quick(ctx);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sqlite_position(&self, index: usize) -> egui::Pos2 {
        self.data_state.preview_sqlite_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sqlite_smoke(&mut self, phase: u8, fixture: &std::path::Path) -> bool {
        match phase {
            0 => {
                self.data_state.input = "unrelated existing work".into();
                self.data_state.create("SQLite 导出验证").unwrap();
                self.data_state
                    .preview_sqlite_export(fixture.parent().unwrap().join("本地资料.sqlite"));
                self.page = Page::Data;
                true
            }
            1..=3 => {
                assert_eq!(self.page, Page::Data);
                self.data_state.preview_sqlite_check(phase)
            }
            4 => {
                assert_eq!(self.page, Page::SqliteBrowser);
                assert_eq!(self.data_state.instances.len(), 2);
                assert_eq!(
                    self.data_state.instances[0].state.input,
                    "unrelated existing work"
                );
                assert_eq!(self.data_state.instances[1].name, "SQLite 导出验证");
                assert!(self.data_state.input.contains("001"));
                self.sqlite_browser.preview_saved_export_ready()
            }
            _ => panic!("invalid SQLite smoke phase"),
        }
    }
    /// Only compiled for the isolated screenshot fixture, never a production entry point.
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_position(&self, index: usize) -> egui::Pos2 {
        self.data_state.preview_workflow_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_prepare(&mut self, ctx: &egui::Context, light: bool) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.data_state.input = "original table draft".into();
        self.open_startup_tool("text-flow");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_position(&self, index: usize) -> egui::Pos2 {
        self.data_state.preview_text_flow_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_check(&self, sent: bool) {
        assert_eq!(self.data_state.active_tool_id(), "text-flow");
        assert_eq!(
            self.preferences.recent.first().map(String::as_str),
            Some("text-flow")
        );
        self.data_state.preview_text_flow_check();
        assert_eq!(self.data_state.input, "original table draft");
        assert_eq!(self.handoff.is_some(), sent);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_file_path(&mut self, path: PathBuf) {
        self.data_state.preview_text_flow_file_path(path);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_file_check(&self, path: &std::path::Path, phase: u8) {
        assert_eq!(self.data_state.instances.len(), 1);
        assert_eq!(self.data_state.input, "original table draft");
        self.data_state.preview_text_flow_file_check(path, phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_flow_table_check(&self, phase: u8) {
        self.data_state.instances[0]
            .state
            .preview_text_flow_table_check(phase == 1);
        assert_eq!(
            self.data_state.instances[0].state.input,
            "original table draft"
        );
        assert!(self.handoff.is_none());
        assert_eq!(
            self.data_state.instances.len(),
            if phase == 3 { 2 } else { 1 }
        );
        if phase == 3 {
            self.data_state.preview_text_flow_table_received();
            assert_eq!(
                self.preferences.recent.first().map(String::as_str),
                Some("data")
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_output_prepare(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        sqlite: bool,
        light: bool,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.page = Page::Data;
        self.data_state
            .preview_workflow_output_prepare(path, sqlite);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_output_position(&self, index: usize) -> egui::Pos2 {
        self.data_state.preview_workflow_output_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_output_check(&self, phase: u8) -> bool {
        self.data_state.preview_workflow_output_check(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_output_reload(&mut self, path: &Path, phase: u8) {
        self.data_state.preview_workflow_output_reload(path, phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_browse_prepare(&mut self) {
        self.data_state.show_workflow_output();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sqlite_transfer_position(&self, index: usize) -> egui::Pos2 {
        self.sqlite_browser.preview_transfer_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_browse_check(&self, phase: u8) {
        self.data_state.instances[0]
            .state
            .preview_page_handoff_source();
        if phase < 3 {
            assert_eq!(self.page, Page::SqliteBrowser);
            assert_eq!(self.data_state.instances.len(), 1);
            self.sqlite_browser.preview_workflow_result_check(phase);
            assert_eq!(
                self.preferences.recent.first().map(String::as_str),
                Some("sqlite")
            );
        } else {
            assert_eq!(self.page, Page::Data);
            assert_eq!(self.data_state.instances.len(), 2);
            self.data_state.preview_page_handoff_result();
            assert_eq!(
                self.preferences.recent.first().map(String::as_str),
                Some("data")
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_check(&mut self, phase: u8) -> bool {
        self.data_state.preview_workflow_check(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_rows(&mut self, phase: u8) -> bool {
        self.data_state.preview_workflow_rows(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_inspection(&mut self, phase: u8) {
        self.data_state.preview_workflow_inspection(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_memory_position(&self, index: usize) -> egui::Pos2 {
        self.data_state.preview_workflow_memory_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_memory_check(&self, remembered: bool) {
        let folder = self.data_state.preview_workflow_memory_check(remembered);
        let expected = remembered.then_some(folder.clone());
        assert_eq!(self.preferences.workflow_library_folder, expected);
        assert_eq!(
            Preferences::load(&self.preferences_path).workflow_library_folder,
            expected
        );
        assert!(folder.join("daily.json").is_file());
        assert!(folder.join("orders.json").is_file());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_search_check(&mut self, phase: u8) {
        if phase == 0 {
            assert!(self.launcher_open);
            assert_eq!(self.launcher_query, "每日资料清洗");
            assert!(self.entries(&self.launcher_query).is_empty());
            let results = self.data_state.workflow_matches(&self.launcher_query);
            assert_eq!(results.total, 1);
            assert_eq!(results.entries[0].instance_id, self.data_state.active_id());
        } else {
            assert!(!self.launcher_open);
            self.data_state.preview_workflow_search_check(phase);
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_bookmark_position(&self) -> egui::Pos2 {
        self.workflow_bookmark_button
            .expect("visible workflow bookmark button")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_recent_position(&self) -> egui::Pos2 {
        self.workflow_recent_button
            .expect("visible recent removal button")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_recent_check(&mut self, phase: u8) {
        match phase {
            0 => {
                assert_eq!(self.preferences.workflow_recent.len(), 1);
                assert!(self.preferences.workflow_favorites.is_empty());
                self.data_state.preview_workflow_bookmark_state(0);
            }
            1 => {
                assert_eq!(self.preferences.workflow_recent.len(), 1);
                assert_eq!(self.preferences.workflow_recent[0].name, "每日资料清洗");
                assert_eq!(
                    Preferences::load(&self.preferences_path).workflow_recent,
                    self.preferences.workflow_recent
                );
                assert!(self.preferences.workflow_recent[0].path.is_file());
                self.data_state.preview_workflow_search_check(1);
            }
            2 => self.data_state.preview_workflow_search_check(2),
            3 => {
                assert!(self.launcher_open);
                assert_eq!(self.launcher_query, "每日资料清洗");
                let query = self.launcher_query.clone();
                let results = self.launcher_results(&query);
                assert_eq!(results.recent.len(), 1);
                assert!(results.saved.is_empty());
                assert!(results.tools.is_empty());
                assert_eq!(results.workflows.total, 0);
                self.data_state.preview_workflow_search_check(2);
            }
            4 => {
                assert!(self.preferences.workflow_recent.is_empty());
                assert!(
                    Preferences::load(&self.preferences_path)
                        .workflow_recent
                        .is_empty()
                );
                assert!(
                    self.preferences_path
                        .parent()
                        .unwrap()
                        .join("workflow-library-fixture/daily.json")
                        .is_file()
                );
                self.data_state.preview_workflow_search_check(2);
            }
            _ => panic!("unknown recent workflow fixture phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_search_scope_position(&self, index: usize) -> egui::Pos2 {
        self.launcher_scope_buttons[index]
            .expect("visible search scope")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_search_scope_check(&mut self, phase: u8) {
        assert!(self.launcher_open);
        assert_eq!(self.launcher_index, 0);
        let query = self.launcher_query.to_lowercase();
        let results = self.launcher_results(&query);
        match phase {
            1 => {
                assert!(self.launcher_scope == launcher::Scope::Tools);
                assert_eq!(query, "每日资料清洗");
                assert!(results.tools.is_empty());
                assert!(results.saved.is_empty());
                assert_eq!(results.workflows.total, 0);
            }
            2 => {
                assert!(self.launcher_scope == launcher::Scope::Workflows);
                assert_eq!(query, "每日资料清洗");
                assert!(results.tools.is_empty());
                assert_eq!(results.saved.len(), 1);
                assert_eq!(results.workflows.total, 1);
            }
            3 => {
                assert!(self.launcher_scope == launcher::Scope::Favorites);
                assert_eq!(query, "每日资料清洗");
                assert!(results.tools.is_empty());
                assert_eq!(results.saved.len(), 1);
                assert_eq!(results.workflows.total, 0);
            }
            4 => {
                assert!(self.launcher_scope == launcher::Scope::Favorites);
                assert_eq!(query, "json");
                assert!(results.tools.iter().any(|tool| tool.id == "json"));
                assert!(
                    results
                        .tools
                        .iter()
                        .all(|tool| self.preferences.favorites.contains(&tool.id))
                );
                assert_eq!(results.saved.len(), 1); // Filename daily.json also matches.
                assert_eq!(results.workflows.total, 0);
            }
            5 => {
                assert!(self.launcher_scope == launcher::Scope::All);
                assert_eq!(query, "json");
                assert!(results.tools.iter().any(|tool| tool.id == "json"));
                assert!(results.tools.len() > self.preferences.favorites.len());
            }
            6 => {
                assert!(self.launcher_scope == launcher::Scope::Workflows);
                assert!(query.is_empty());
                assert!(results.tools.is_empty());
                assert_eq!(results.workflows.total, 2);
                assert_eq!(results.saved.len(), 1);
            }
            _ => panic!("unknown scope fixture phase"),
        }
        self.data_state.preview_workflow_search_check(2);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_workflow_bookmark_check(&mut self, phase: u8) {
        match phase {
            0 => {
                assert_eq!(self.preferences.workflow_favorites.len(), 1);
                assert_eq!(
                    Preferences::load(&self.preferences_path).workflow_favorites,
                    self.preferences.workflow_favorites
                );
                self.data_state.preview_workflow_bookmark_state(0);
            }
            1 => {
                assert_eq!(self.preferences.workflow_favorites.len(), 1);
                assert_eq!(
                    Preferences::load(&self.preferences_path).workflow_favorites,
                    self.preferences.workflow_favorites
                );
                assert!(self.launcher_open);
                self.data_state.preview_workflow_bookmark_state(1);
            }
            2 => {
                assert!(self.launcher_open);
                assert_eq!(self.launcher_query, "每日资料清洗");
                assert!(self.entries(&self.launcher_query).is_empty());
                assert_eq!(
                    self.data_state.workflow_matches(&self.launcher_query).total,
                    0
                );
                assert_eq!(
                    self.preferences
                        .workflow_favorites
                        .iter()
                        .filter(|entry| entry.matches(&self.launcher_query))
                        .count(),
                    1
                );
            }
            3 => {
                assert!(self.preferences.workflow_favorites.is_empty());
                assert!(
                    Preferences::load(&self.preferences_path)
                        .workflow_favorites
                        .is_empty()
                );
                self.data_state.preview_workflow_bookmark_state(1);
                assert_eq!(
                    self.data_state.workflow_matches(&self.launcher_query).total,
                    0
                );
            }
            _ => panic!("unknown bookmark fixture phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_text_plugin(&mut self, ctx: &egui::Context, light: bool, phase: u8) {
        match phase {
            0 => {
                self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
                self.startup_warning = None;
                self.launcher_open = false;
                self.quick_open = false;
                self.open_startup_tool("json");
                self.tool_state.plugin_compatible = true;
                self.tool_state.input = r#"{"name":"Zi","name":"duplicate"}"#.into();
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            1 => {
                assert!(self.tool_state.message.contains("INVALID_INPUT"));
                assert!(self.tool_state.output.is_empty());
            }
            2 => {
                self.tool_state.input = r#"{"b":2,"a":1}"#.into();
                self.tool_state.message.clear();
            }
            3 => {
                assert!(self.tool_state.message.is_empty());
                assert_eq!(self.tool_state.output, "{\n  \"a\": 1,\n  \"b\": 2\n}");
            }
            4 => {
                self.tool_state.input = format!("[{}]", vec!["0"; 2000].join(","));
                self.tool_state.message.clear();
            }
            5 => {
                assert!(self.tool_state.message.is_empty());
                assert_eq!(self.tool_state.output.len(), 10002);
            }
            6 => {
                self.tool_state.select(ToolKind::Base64);
                self.tool_state.input.clear();
                self.tool_state.clear_result();
                self.tool_state.plugin_compatible = true;
            }
            7 => {
                assert!(self.tool_state.has_result());
                assert!(self.tool_state.output.is_empty());
                assert!(self.tool_state.message.is_empty());
            }
            _ => unreachable!(),
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_scene(&mut self, ctx: &egui::Context, scene: usize, fixture: PathBuf) {
        let light = scene % 2 == 1 || scene == 8;
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.quick_open = false;
        self.handoff = None;
        self.launcher_open = false;
        self.tool_search.clear();
        self.preferences.recent.clear();
        self.preferences.usage.clear();
        self.library_query.clear();
        self.home_query_key = (String::new(), "全部".into(), "全部分类".into());
        self.home_filter = "全部".into();
        self.home_category = "全部分类".into();
        self.preferences.favorites = vec!["data".into(), "files".into(), "json".into()];
        match scene {
            460..=461 => {
                self.page = Page::Data;
                self.data_state.preview_workflow_library(&fixture);
            }
            444..=453 => self.preview_service_scene(ctx, scene, &fixture),
            454..=455 => self.preview_log_transfer_scene(),
            456..=457 => self.preview_service_scene(ctx, scene, &fixture),
            458..=459 => self.preview_service_batch_report_scene(),
            432..=435 => self.preview_plot_fixture(true),
            436..=439 => {
                self.page = Page::Calculator;
                self.calculator.preview_statistics_fixture();
            }
            440..=443 => self.preview_table_statistic_fixture(true),
            430..=431 => self.preview_boundary_scene(),
            426..=429 => {
                self.preview_mapping_scene(false);
                if scene >= 428 {
                    self.data_state.preview_numeric_entry_dialog(true);
                }
            }
            418..=421 => self.preview_mapping_scene(false),
            422..=425 => self.preview_mapping_scene(true),
            410..=417 => self.preview_numeric_scene(scene >= 414),
            406..=409 => {
                self.page = Page::Calculator;
                self.calculator.preview_sheet_fixture();
            }
            402..=405 => {
                self.page = Page::Calculator;
                self.calculator.preview_matrix_fixture();
            }
            400..=401 => {
                self.page = Page::Clipboard;
                self.clipboard.preview_image_fixture(ctx);
            }
            398..=399 => {
                self.page = Page::Images;
                self.images.preview_relay_fixture(ctx);
            }
            396..=397 => {
                self.page = Page::Clipboard;
                self.clipboard.preview_policy_fixture();
            }
            394..=395 => {
                self.page = Page::Clipboard;
                self.clipboard.preview_retention_fixture();
            }
            392..=393 => {
                self.page = Page::Clipboard;
                self.clipboard.preview_storage_fixture();
            }
            390..=391 => {
                self.page = Page::Clipboard;
                self.clipboard.preview_fixture();
            }
            388..=389 => {
                self.page = Page::Recorder;
                self.recorder.preview_spotlight_fixture();
            }
            384..=387 => {
                self.page = Page::Updates;
                self.updates.preview_signed_report();
                self.portable_update.preview_delta(scene >= 386);
            }
            380..=383 => {
                self.page = Page::Updates;
                self.updates.preview_signed_report();
                self.msi_update.preview(scene >= 382);
            }
            376..=379 => {
                self.page = Page::Updates;
                self.preferences.updates = Default::default();
                self.portable_update.preview(scene >= 378);
            }
            370..=373 => {
                self.page = Page::Updates;
                self.preferences.updates = Default::default();
                self.updates.preview_signed_download(scene >= 372);
            }
            374..=375 => {
                self.page = Page::Updates;
                self.preferences.updates = Default::default();
                self.updates.preview_signed_report();
            }
            366..=369 => {
                self.page = Page::Updates;
                self.preferences.updates = Default::default();
                self.updates.preview_download(scene >= 368);
            }
            362..=365 => {
                self.page = Page::Updates;
                self.preferences.updates = Default::default();
                self.updates.preview_verification(scene >= 364);
            }
            358..=361 => {
                self.page = Page::DeltaUpdate;
                self.delta_update.preview_fixture(scene >= 360);
            }
            354..=357 => {
                self.page = Page::Updates;
                self.preferences.updates = Default::default();
                self.updates.preview_fixture(scene >= 356);
            }
            350..=353 => {
                self.page = Page::Recorder;
                self.recorder.preview_tutorial_fixture(scene >= 352);
            }
            348..=349 => {
                self.page = Page::Images;
                self.images.preview_screenshot_fixture(ctx);
            }
            310..=313 => {
                self.page = Page::Calculator;
                self.calculator.preview_fixture();
            }
            326..=347 => {
                self.page = Page::Clock;
                self.clock.preview_fixture((scene - 326) / 2);
            }
            320..=325 => {
                self.page = Page::Commands;
                self.preview_profile_fixture(scene >= 324);
            }
            316..=319 => {
                self.page = Page::Commands;
                self.preview_binding_fixture();
            }
            314..=315 => {
                self.page = Page::Commands;
            }
            306..=307 => {
                self.page = Page::Data;
                self.data_state.preview_workflow_empty(true);
            }
            308..=309 => {
                self.page = Page::Data;
                self.data_state.preview_workflow_empty(false);
            }
            304..=305 => {
                self.page = Page::Data;
                self.data_state.preview_workflow_empty(false);
            }
            300..=303 => {
                self.page = Page::Data;
                self.data_state.preview_workflow_import();
            }
            298..=299 => {
                self.page = Page::Data;
                self.data_state.preview_workflow();
            }
            296..=297 => {
                self.page = Page::Library;
                self.library_query = "备忘".into();
            }
            294..=295 => {
                self.page = Page::Library;
                self.library_query = "表格另存".into();
            }
            290..=293 => {
                self.page = Page::Data;
                self.data_state
                    .preview_sqlite_export(fixture.parent().unwrap().join("本地资料.sqlite"));
            }
            288..=289 => self.preview_handoff_discovery_smoke(0),
            286..=287 => self.preview_event_handoff_scene(),
            282..=285 => {
                self.page = Page::Calendar;
                self.planner.preview_week();
            }
            278..=281 => {
                self.page = if scene < 280 {
                    Page::Notes
                } else {
                    Page::Calendar
                };
                self.planner.preview_convert(scene < 280);
            }
            274..=277 => {
                self.page = if scene >= 276 {
                    Page::Calendar
                } else {
                    Page::Notes
                };
                self.planner.preview_duplicate(scene >= 276);
            }
            270..=273 => {
                self.page = Page::Notes;
                self.planner.preview_listing(scene >= 272);
            }
            266..=269 => {
                self.page = Page::Home;
                self.planner.preview_snooze(scene >= 268);
            }
            256..=265 => {
                let calendar = (258..=259).contains(&scene) || (262..=265).contains(&scene);
                self.page = if calendar {
                    Page::Calendar
                } else {
                    Page::Notes
                };
                self.planner.preview_actions(calendar, scene >= 264);
            }
            252..=255 => {
                self.page = Page::Calendar;
                self.planner.preview_cutoff();
            }
            246..=251 => {
                self.page = Page::Calendar;
                self.planner.preview_ics(scene >= 248);
            }
            238..=245 => {
                self.page = Page::Calendar;
                self.planner.preview_interval(scene >= 240);
                if (242..=243).contains(&scene) {
                    self.planner.preview_interval_agenda();
                }
                if scene >= 244 {
                    self.planner.preview_interval_focus();
                }
            }
            234..=237 => {
                self.page = Page::Calendar;
                self.planner.preview_recurrence(scene >= 236);
            }
            230..=233 => {
                self.page = Page::Calendar;
                self.planner.preview_agenda(false);
                self.preferences.favorites =
                    ["data", "files", "json", "base64", "timestamp", "memos"]
                        .map(str::to_owned)
                        .to_vec();
                self.home_filter = "收藏".into();
                self.library_query = "日历".into();
                if scene >= 232 {
                    let entry = self
                        .entries("")
                        .into_iter()
                        .filter(|e| e.kind.is_some())
                        .max_by_key(|e| e.title.chars().count())
                        .unwrap();
                    self.open_entry(&entry);
                }
            }
            226..=229 => {
                self.page = Page::Calendar;
                self.planner.preview_agenda(scene >= 228);
            }
            220..=225 => {
                self.page = Page::Notes;
                self.planner.preview_backup(scene);
            }
            216..=219 => {
                self.page = Page::Notes;
                self.planner.preview_file_exchange(scene >= 218);
            }
            212..=215 => {
                self.page = Page::Notes;
                self.planner.preview_trash(scene >= 214);
            }
            208..=211 => self.preview_memo_handoff(scene >= 210),
            198..=207 => {
                self.page = if scene >= 200 {
                    Page::Calendar
                } else {
                    Page::Notes
                };
                self.planner
                    .preview(scene >= 200, (202..=203).contains(&scene));
                if scene >= 206 {
                    self.planner.preview_focus_editor();
                }
            }
            0 | 1 => self.page = Page::Home,
            190..=197 => {
                self.page = if scene >= 196 {
                    Page::Tasks
                } else {
                    Page::Data
                };
                self.data_state
                    .preview_fixture(matches!(scene, 192 | 193), matches!(scene, 194 | 195));
                self.observe_tasks();
            }
            184 | 185 | 188 => self.page = Page::Library,
            189 => {
                self.page = Page::Library;
                self.library_query = "mcp".into();
            }
            186 | 187 => {
                self.page = Page::Library;
                self.library_query = "压缩图片".into();
            }
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
            98 | 99 => {
                self.page = Page::Mcp;
                self.mcp.preview_fixture(scene == 99);
            }
            169 => {
                self.page = Page::Mcp;
                self.mcp.preview_connected();
            }
            170 | 171 => {
                self.page = Page::Mcp;
                self.mcp.preview_http();
            }
            172 | 173 => {
                self.page = Page::Mcp;
                self.mcp.preview_http_authentication();
            }
            174 | 175 => {
                self.page = Page::Mcp;
                self.mcp.preview_oauth_metadata();
            }
            182 | 183 => {
                self.page = Page::Mcp;
                self.mcp.preview_oauth_refresh();
            }
            180 | 181 => {
                self.page = Page::Mcp;
                self.mcp.preview_oauth_registration();
            }
            178 | 179 => {
                self.page = Page::Mcp;
                self.mcp.preview_oauth_auto();
            }
            176 | 177 => {
                self.page = Page::Mcp;
                self.mcp.preview_oauth_refresh();
            }
            100 | 101 => {
                self.page = Page::Recorder;
                self.recorder.preview_fixture();
            }
            102 | 103 => {
                self.page = Page::Recorder;
                self.recorder.preview_interrupted();
            }
            104 | 105 => {
                self.page = Page::Images;
                self.images.preview_fixture(ctx);
            }
            106 | 107 => {
                self.page = Page::Images;
                self.images.preview_batch_fixture();
            }
            108 | 109 => {
                self.page = Page::Images;
                self.images.preview_metadata_fixture(ctx);
            }
            110 | 111 => {
                self.page = Page::Images;
                self.images.preview_editor_fixture(ctx);
            }
            112 | 113 => {
                self.page = Page::Markdown;
                self.markdown.preview_fixture();
            }
            114 | 115 => {
                self.page = Page::FileEncoding;
                self.file_encoding.preview_fixture();
            }
            116 | 117 => {
                self.page = Page::ChecksumManifest;
                self.checksum_manifest.preview_fixture();
            }
            155 | 156 => {
                self.page = Page::DiskInspector;
                self.disk_inspector.preview_fixture();
            }
            157 | 158 => {
                self.page = Page::DuplicateFinder;
                self.duplicate_finder.preview_fixture();
            }
            165 | 166 => {
                self.page = Page::DirectoryCompare;
                self.directory_compare.preview_fixture();
            }
            167 | 168 => {
                self.page = Page::SqliteBrowser;
                self.sqlite_browser.preview_fixture();
            }
            159 | 160 => self.page = Page::AsciiCodes,
            161 | 162 => {
                self.page = Page::Symbols;
                self.symbols
                    .preview_fixture(if scene == 161 { 4 } else { 5 });
            }
            163 | 164 => {
                self.page = Page::AsciiArt;
                self.ascii_art.preview_fixture();
            }
            118 | 119 => {
                self.page = Page::KnowledgeSources;
                self.knowledge_sources.preview_fixture();
            }
            120 | 121 => {
                self.page = Page::DocumentIngestion;
                self.knowledge_sources.preview_fixture();
                self.document_ingestion
                    .preview_fixture(self.knowledge_sources.sources());
            }
            122 | 123 => {
                self.page = Page::KnowledgeIndex;
                self.knowledge_sources.preview_fixture();
                self.knowledge_index
                    .preview_fixture(self.knowledge_sources.sources());
            }
            124 | 125 => {
                self.page = Page::KnowledgeSearch;
                self.knowledge_sources.preview_fixture();
                self.knowledge_search
                    .preview_fixture(self.knowledge_sources.sources());
            }
            126 | 127 => {
                self.page = Page::KnowledgeAnswer;
                self.knowledge_sources.preview_fixture();
                self.knowledge_answer
                    .preview_fixture(self.knowledge_sources.sources(), false);
            }
            144 | 145 => {
                self.page = Page::KnowledgeAnswer;
                self.knowledge_sources.preview_fixture();
                self.knowledge_answer
                    .preview_fixture(self.knowledge_sources.sources(), true);
            }
            128 | 129 => {
                self.page = Page::KnowledgeEval;
                self.knowledge_sources.preview_fixture();
                self.knowledge_eval
                    .preview_fixture(self.knowledge_sources.sources(), false);
            }
            146 | 147 => {
                self.page = Page::KnowledgeEval;
                self.knowledge_sources.preview_fixture();
                self.knowledge_eval
                    .preview_fixture(self.knowledge_sources.sources(), true);
            }
            148 | 149 => {
                self.page = Page::Mcp;
                self.mcp.preview_permissions();
            }
            150 | 151 => {
                self.page = Page::Agent;
                self.agent.preview_fixture(scene == 151);
            }
            152 => {
                self.page = Page::Agent;
                self.agent.preview_record_export();
            }
            153 | 154 => {
                self.page = Page::AgentRecords;
                self.agent_records.preview_fixture(scene == 154);
            }
            130 | 131 => {
                self.page = Page::KnowledgeCapture;
                self.knowledge_sources.preview_fixture();
                self.knowledge_capture
                    .preview_fixture(self.knowledge_sources.sources());
            }
            132 | 133 => {
                self.page = Page::KnowledgeMcp;
                self.knowledge_sources.preview_fixture();
            }
            134 | 135 => {
                self.page = Page::KnowledgeMcp;
                self.knowledge_sources.preview_fixture();
            }
            136 | 137 => {
                self.page = Page::Mcp;
                self.mcp.preview_tool_review(scene == 137);
            }
            138 | 139 => {
                self.page = Page::Embedding;
                self.embedding.preview_fixture();
            }
            140 | 141 => {
                self.page = Page::VectorIndex;
                self.knowledge_sources.preview_fixture();
                self.vector_index
                    .preview_fixture(self.knowledge_sources.sources());
            }
            142 | 143 => {
                self.page = Page::HybridSearch;
                self.knowledge_sources.preview_fixture();
                self.hybrid_search
                    .preview_fixture(self.knowledge_sources.sources());
            }
            78..=97 => {
                self.page = Page::Plugins;
                if !self
                    .plugins
                    .store
                    .packages
                    .iter()
                    .any(|p| p.manifest.id == "openai-local")
                {
                    self.plugins
                        .store
                        .install(include_bytes!("../plugins-examples/openai-compatible.json"))
                        .unwrap();
                }
                self.plugins
                    .store
                    .set_enabled("openai-local", true)
                    .unwrap();
                self.plugins.select("plugin:openai-local/chat");
                if scene >= 80 {
                    self.plugins.preview_profiles();
                }
                if scene >= 82 {
                    self.plugins.preview_model_discovery();
                }
                if scene >= 84 {
                    self.plugins.preview_conversation();
                }
                if (86..=87).contains(&scene) {
                    self.plugins.preview_stream();
                }
                if (88..=89).contains(&scene) {
                    self.plugins.preview_import();
                }
                if (90..=91).contains(&scene) {
                    self.plugins.preview_attachments();
                }
                if (92..=93).contains(&scene) {
                    self.plugins.preview_library();
                }
                if (94..=95).contains(&scene) {
                    self.plugins.preview_prompts();
                }
                if scene >= 96 {
                    self.plugins.preview_prompt_editor();
                }
            }
            76 | 77 => {
                self.page = Page::Tasks;
                self.preview_tasks();
            }
            74 | 75 => {
                self.page = Page::Data;
                self.data_state.preview_join();
            }
            70..=73 => {
                self.page = Page::Data;
                self.data_state.preview_schema(scene >= 72);
            }
            68 | 69 => {
                self.page = Page::Data;
                self.data_state.preview_transform();
            }
            66 | 67 => {
                self.navigate(Page::EncodingTools, Some(ToolKind::Json));
                self.tool_state.input = "[{\"name\":\"示例\",\"count\":3}]".into();
                self.tool_state.output =
                    run_tool(ToolKind::Json, 0, &self.tool_state.input, "", 10).unwrap();
                self.handoff =
                    Some(handoff::Transfer::new("JSON".into(), &self.tool_state.output).unwrap());
            }
            64 | 65 => {
                self.page = Page::Intake;
                self.intake
                    .accept(vec![fixture.with_file_name("订单数据.csv")]);
                self.intake.target = crate::intake::Target::Csv;
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
                let tool = crate::framework::Tool::ALL[(scene - 32) / 2];
                self.page = if tool.category() == "Java 与 JVM" {
                    Page::Java
                } else {
                    Page::Django
                };
                self.frameworks.preview(tool);
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
                self.page = Page::Library;
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
                        self.page = Page::Library;
                        self.home_category = "文本与编码".into();
                    }
                    24 => {
                        self.page = Page::Library;
                        self.visit("plugin:local-text/deduplicate");
                        self.visit("json");
                        self.home_filter = "最近".into();
                    }
                    25 => {
                        self.page = Page::Plugins;
                        self.plugins.select("plugin:local-text/deduplicate");
                    }
                    _ => {
                        self.page = Page::Library;
                        self.open_launcher();
                        self.launcher_query = "去重".into();
                    }
                }
            }
            _ => {
                self.page = Page::Library;
                self.open_launcher();
                self.launcher_query = "json".into();
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_position(&self, index: usize) -> egui::Pos2 {
        self.clock.rects[index].center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_audio_prepare(&mut self) {
        self.page = Page::Clock;
        self.clock.preview_audio_prepare();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_window_prepare(&mut self, ctx: &egui::Context, index: usize) {
        self.set_theme(
            ctx,
            if index % 2 == 1 {
                Theme::Light
            } else {
                Theme::Dark
            },
        );
        self.page = Page::Clock;
        self.clock.preview_window_prepare(ctx, index);
    }
    #[cfg(all(windows, feature = "ui-preview"))]
    pub fn preview_clock_window_dimensions(&self) -> [i32; 2] {
        self.clock.preview_window_dimensions()
    }
    #[cfg(all(windows, feature = "ui-preview"))]
    pub fn preview_clock_window_click(&self, index: usize) {
        self.clock.preview_window_click(index);
    }
    #[cfg(all(windows, feature = "ui-preview"))]
    pub fn preview_clock_window_key(&self, key: u32, scan: u32) {
        self.clock.preview_window_key(key, scan);
    }
    #[cfg(all(windows, feature = "ui-preview"))]
    pub fn preview_clock_window_check(&mut self, ctx: &egui::Context, phase: u8) -> isize {
        let handle = self.clock.preview_window_check(ctx, phase);
        if phase == 0 {
            self.hide_to_tray(ctx);
            assert!(self.preview_hidden());
        }
        if phase == 6 {
            assert!(!self.preview_hidden());
        }
        handle
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_audio_position(&self, index: usize) -> egui::Pos2 {
        self.clock.preview_audio_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_audio_check(&mut self, phase: u8) -> bool {
        self.clock.preview_audio_check(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_storage_prepare(&mut self, saving: bool) {
        self.clock.preview_storage_prepare(saving);
        self.page = Page::Clock;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_storage_ready(&self, phase: u8) -> bool {
        self.clock.preview_storage_ready(phase)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_storage_position(&self, index: usize) -> egui::Pos2 {
        self.clock.preview_storage_position(index)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_done(&mut self) -> bool {
        self.clock.preview_done()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clock_check(&mut self, phase: u8) {
        self.clock.preview_check(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_io(&mut self, ctx: &egui::Context, folder: &std::path::Path, phase: u8) {
        match phase {
            0 => self.calculator.preview_sheet_io_start(ctx, folder),
            1 => self.calculator.preview_sheet_io_read(ctx),
            2 => self.calculator.preview_sheet_io_check(false),
            3 => self.calculator.preview_sheet_io_check(true),
            4 => self.calculator.preview_sheet_overwrite(ctx),
            5 => self.calculator.preview_sheet_cleanup(),
            6 => {
                assert!(self.calculator.has_work());
                self.tray_exit_requested.store(true, Ordering::Release);
            }
            7 => {
                assert!(self.workspace_exit_confirm);
                assert!(!self.quit_requested);
                self.workspace_exit_confirm = false;
            }
            _ => unreachable!(),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_sheet_position(&self, apply: bool) -> egui::Pos2 {
        self.calculator.preview_sheet_position(apply)
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_matrix_position(&self) -> egui::Pos2 {
        self.calculator.preview_matrix_position()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_matrix_check(&self) {
        self.calculator.preview_matrix_check();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_calculator_position(&self) -> egui::Pos2 {
        self.calculator.preview_input_position()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_calculator_check(&self, phase: u8) {
        self.calculator.preview_check(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistics_check(&self, phase: u8) {
        self.calculator.preview_statistics_check(phase);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_statistics_position(&self) -> egui::Pos2 {
        self.calculator.preview_statistics_position()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_keyboard_fixture(&mut self) {
        if self.plugins.store.packages.is_empty() {
            self.plugins
                .store
                .install(include_bytes!("../plugins-examples/local-text.json"))
                .unwrap();
        }
        self.plugins.store.set_enabled("local-text", true).unwrap();
        self.frameworks.preview(crate::framework::Tool::Sql);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_instance_start(&mut self) {
        self.data_state
            .import_new(
                "id,name\n1,Alpha".into(),
                crate::workbench::DataFormat::Csv,
                false,
                "并行实例 A",
            )
            .unwrap();
        self.data_state
            .import_new(
                "id,name\n2,Beta".into(),
                crate::workbench::DataFormat::Csv,
                false,
                "并行实例 B",
            )
            .unwrap();
        self.page = Page::Home;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_instance_results(&mut self) {
        let a = self
            .data_state
            .instances
            .iter()
            .find(|i| i.name == "并行实例 A")
            .unwrap()
            .id
            .clone();
        let b = self
            .data_state
            .instances
            .iter()
            .find(|i| i.name == "并行实例 B")
            .unwrap()
            .id
            .clone();
        for id in [&a, &b] {
            assert!(
                self.tasks.rows.iter().any(
                    |r| r.instance.as_ref() == Some(id) && r.phase == crate::tasks::Phase::Done
                )
            );
        }
        self.open_task_result("data", Some(&a)).unwrap();
        assert!(self.data_state.input.contains("Alpha"));
        self.open_task_result("data", Some(&b)).unwrap();
        assert!(self.data_state.input.contains("Beta"));
        self.data_state.close(&a, true).unwrap();
        assert!(self.open_task_result("data", Some(&a)).is_err());
        assert_eq!(self.data_state.active_id(), b);
        println!(
            "PASS instances: two background parses while hidden, distinct task rows, exact result routing, closed instance refusal"
        );
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_catalog_routes(&mut self) {
        let entries = self.entries("");
        let favorites = self.preferences.favorites.clone();
        self.library_query = "压缩图片".into();
        for entry in &entries {
            let before = self.preferences.usage.get(&entry.id).copied().unwrap_or(0);
            self.open_startup_tool(&entry.id);
            assert_eq!(self.page, entry.page, "{}", entry.id);
            assert_eq!(self.preferences.recent.first(), Some(&entry.id));
            assert_eq!(self.preferences.usage[&entry.id], before + 1);
            assert_eq!(self.library_query, "压缩图片");
            if let Some(kind) = entry.kind {
                assert_eq!(self.tool_state.selected, kind);
            }
            if let Some(tool) = crate::framework::Tool::from_id(&entry.id) {
                assert_eq!(self.frameworks.selected, tool);
            }
            if entry.id.starts_with("plugin:") {
                assert_eq!(self.plugins.selected.as_ref(), Some(&entry.id));
            }
        }
        assert_eq!(self.preferences.favorites, favorites);
        self.plugins.store.set_enabled("local-text", false).unwrap();
        self.open_startup_tool("plugin:local-text/uppercase");
        assert_eq!(self.page, Page::Library);
        assert!(self.toast.as_ref().unwrap().0.contains("尚未启用"));
        self.open_startup_tool("unknown-no-such-tool");
        assert_eq!(self.page, Page::Library);
        assert!(
            !self
                .entries("")
                .iter()
                .any(|e| e.id.starts_with("plugin:local-text/"))
        );
        self.plugins.store.set_enabled("local-text", true).unwrap();
        self.page = Page::Library;
        let found = self.library_entries();
        assert!(found.iter().any(|e| e.id == "image-tools"));
        let original_preferences = (
            self.preferences.favorites.clone(),
            self.preferences.recent.clone(),
            self.preferences.usage.clone(),
        );
        let original_filter = self.home_filter.clone();
        let original_category = self.home_category.clone();
        self.home_category = "全部分类".into();
        self.preferences.favorites = vec!["json-diff".into(), "json".into()];
        self.preferences.recent = self.preferences.favorites.clone();
        self.preferences.usage.clear();
        self.preferences.usage.insert("json-diff".into(), 100);
        self.preferences.usage.insert("json".into(), 1);
        for filter in ["收藏", "最近", "常用"] {
            self.home_filter = filter.into();
            self.library_query = "json".into();
            assert_eq!(self.library_entries()[0].id, "json", "{filter}");
            self.library_query.clear();
            assert_eq!(self.library_entries()[0].id, "json-diff", "{filter}");
        }
        self.preferences.favorites = original_preferences.0;
        self.preferences.recent = original_preferences.1;
        self.preferences.usage = original_preferences.2;
        self.home_filter = original_filter;
        self.home_category = original_category;
        self.library_query = "压缩图片".into();
        println!("PASS search: relevance in favorites/recent/frequent; browse order retained");
        println!(
            "PASS catalog: {} routes, exact recent/usage IDs, retained query, plugin disable",
            entries.len()
        );
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_shared_preferences(&mut self, ready: bool) {
        if !ready {
            self.preview_shared_theme = Some(self.theme);
            self.open_startup_tool("json");
            let mut other = Preferences::load(&self.preferences_path);
            if !other.favorites.contains(&"base64".to_owned()) {
                other.toggle("base64");
            }
            other.visit("base64");
            other.light = !self.preferences.light;
            other.save(&self.preferences_path).unwrap();
            self.last_preferences_refresh = Instant::now();
            self.page = Page::Library;
            self.home_filter = "收藏".into();
            self.home_category = "全部分类".into();
            self.library_query.clear();
            assert!(!self.preferences.favorites.contains(&"base64".to_owned()));
        } else {
            assert!(self.preferences.favorites.contains(&"base64".to_owned()));
            assert_eq!(self.preferences.recent[0], "base64");
            assert_eq!(self.preferences.usage["base64"], 1);
            assert!(
                self.preview_shared_theme == Some(self.theme),
                "external discovery refresh must not switch the running window theme"
            );
            assert!(
                self.library_entries()
                    .iter()
                    .any(|entry| entry.id == "base64")
            );
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_hidden_panel(&mut self, ctx: &egui::Context, light: bool) {
        self.quick_open = false;
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.preview_panel_frames = 0;
        self.hide_to_tray(ctx);
        let tx = self.event_tx.clone();
        let wake_ctx = ctx.clone();
        let handle = self.window_handle;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            let _ = tx.send(BackgroundEvent::TrayNavigate(TrayAction::QuickPanel));
            wake_main_window(handle, &wake_ctx);
        });
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_panel_rendered(&self) -> bool {
        self.preview_panel_frames >= 8
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_hidden(&self) -> bool {
        #[cfg(windows)]
        {
            main_window_cloaked(self.window_handle)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_taskbar_detached(&self) -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GWL_EXSTYLE, GetWindowLongPtrW, WS_EX_TOOLWINDOW,
            };
            self.window_handle.is_some_and(|handle| unsafe {
                GetWindowLongPtrW(handle as _, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize != 0
            })
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_hide_and_restore_taskbar(&mut self, ctx: &egui::Context, restore: bool) {
        if restore {
            restore_main_window(self.window_handle, ctx);
        } else {
            self.hide_to_tray(ctx);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_minimized(&self) -> bool {
        #[cfg(windows)]
        {
            self.window_handle.is_some_and(|handle| unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(handle as _) != 0
            })
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_import_routes(&mut self) {
        use crate::intake::{Imported, Target};
        if !self.intake.paths.is_empty() {
            let imported = crate::intake::read(self.intake.paths.clone(), Target::Csv).unwrap();
            self.apply_import(imported).unwrap();
            assert!(self.data_state.input.contains("example,3"));
        }
        self.apply_import(Imported {
            target: Target::Json,
            paths: vec![],
            text: "{\"ok\":true}".into(),
        })
        .unwrap();
        assert_eq!(self.tool_state.selected, ToolKind::Json);
        assert_eq!(self.tool_state.input, "{\"ok\":true}");
        self.apply_import(Imported {
            target: Target::Threads,
            paths: vec![],
            text: "thread fixture".into(),
        })
        .unwrap();
        assert_eq!(self.frameworks.selected, crate::framework::Tool::Threads);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clipboard_media_smoke(&mut self, ctx: &egui::Context, phase: u8) -> bool {
        if phase == 0 {
            self.page = Page::Clipboard;
            self.clipboard.preview_image_fixture(ctx);
            return false;
        }
        let Some(picture) = self.clipboard.preview_image_ready(ctx) else {
            return false;
        };
        if phase == 1 {
            self.images
                .start_clipboard_relay(ctx, picture.png.clone())
                .unwrap();
            self.page = Page::Images;
            return true;
        }
        if !self.images.preview_clipboard_relay_apply(ctx, &picture.png) {
            return false;
        }
        let id = self.images.take_relay_route().unwrap();
        self.visit(id);
        assert_eq!(
            self.preferences.recent.first().map(String::as_str),
            Some("image-tools")
        );
        assert!(std::sync::Arc::ptr_eq(
            &self.clipboard.preview_image_ready(ctx).unwrap(),
            &picture
        ));
        true
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_relay_smoke(&mut self, ctx: &egui::Context, phase: u8) -> bool {
        self.page = Page::Images;
        let ready = self.images.preview_relay_smoke(ctx, phase);
        if ready {
            if let Some(id) = self.images.take_relay_route() {
                self.visit(id);
                assert_eq!(
                    self.preferences.recent.first().map(String::as_str),
                    Some(id)
                );
            }
        }
        ready
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_plugin_navigation(&self) -> bool {
        self.page == Page::Plugins
            && self.plugins.selected.as_deref() == Some("plugin:local-text/deduplicate")
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_framework_navigation(&self) -> (bool, bool) {
        (
            self.page == Page::Django && self.frameworks.selected == crate::framework::Tool::Sql,
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
        let clipboard =
            crate::clipboard::State::new(preferences_path.with_file_name("clipboard-policy.json"));
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
        let planner_active = Arc::new(AtomicBool::new(true));
        {
            let stop = Arc::clone(&tray_bridge_stop);
            let active = Arc::clone(&planner_active);
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    if active.load(Ordering::Acquire) {
                        wake_main_window(window_handle, &ctx);
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
        }
        let quick_active = Arc::new(AtomicBool::new(false));
        start_tray_bridge(
            Arc::clone(&manager),
            event_tx.clone(),
            cc.egui_ctx.clone(),
            window_handle,
            Arc::clone(&tray_bridge_stop),
            Arc::clone(&tray_exit_requested),
            Arc::clone(&quick_active),
        );
        let wake_ctx = cc.egui_ctx.clone();
        let mut hotkey_edit = preferences.hotkey.clone();
        if cfg!(feature = "ui-preview") {
            hotkey_edit.enabled = false;
        }
        let hotkey = crate::hotkey::Service::new(hotkey_edit.clone(), move || {
            wake_main_window(window_handle, &wake_ctx)
        });
        let recorder_wake = cc.egui_ctx.clone();
        let recorder_hotkeys =
            crate::hotkey::RecorderHotkeys::new(!cfg!(feature = "ui-preview"), move || {
                wake_main_window(window_handle, &recorder_wake)
            });
        let duplicate_count = other_instance_count();
        let mut recorder = crate::recorder_ui::RecorderState::default();
        recorder.set_auto_minimize(preferences.recorder_auto_minimize);
        recorder.set_auto_stop_minutes(preferences.recorder_auto_stop_minutes);
        recorder.set_quality(preferences.recorder_quality);
        let mut app = Self {
            #[cfg(feature = "ui-preview")]
            preview_panel_frames: 0,
            #[cfg(feature = "ui-preview")]
            preview_sidebar: Default::default(),
            #[cfg(feature = "ui-preview")]
            preview_services: Default::default(),
            #[cfg(feature = "ui-preview")]
            preview_service_rows: 0,
            #[cfg(feature = "ui-preview")]
            preview_tray_capture: None,
            #[cfg(feature = "ui-preview")]
            preview_tray_workflow: None,
            #[cfg(feature = "ui-preview")]
            preview_text_copy: None,
            hotkey, hotkey_edit, hotkey_status: "正在注册快捷键…".into(),
            recorder_hotkeys, recorder_hotkey_status: "正在注册录屏快捷键…".into(),
            quick_active,
            quick_open: false, quick_focus: false, quick_had_focus: false, quick_opened: Instant::now(), quick_tab: "收藏".into(), quick_context: false, quick_position: None, quick_size: egui::vec2(460.0,620.0), intake: Default::default(), tasks: Default::default(), handoff: None,
            manager,
            tray,
            page: Page::Home,
            statuses: Vec::new(),
            selected_service: None,
            search: String::new(),
            service_filter: ServiceFilter::All,
            service_compact: None,
            service_pending: Default::default(),
            service_batch: service_batch::State::default(),
            event_tx,
            event_rx,
            refresh_inflight: false,
            refresh_generation: 0,
            refresh_cancel: Arc::new(AtomicBool::new(false)),
            last_refresh: Instant::now() - Duration::from_secs(30),
            last_preferences_refresh: Instant::now(),
            last_workflow_retry: Instant::now(),
            #[cfg(feature = "ui-preview")]
            preview_shared_theme: None,
            notification: tray_error.clone().unwrap_or_default(),
            notification_error: tray_error.is_some(),
            startup_warning: (duplicate_count > 0).then(|| {
                format!(
                    "检测到 {duplicate_count} 个其他 Zi DevTools 实例；请确认当前托盘图标版本，旧实例不会自动退出"
                )
            }),
            log_view: None,
            log_text: String::new(),
            log_error: false,
            log_lines: service_logs::LogLines::default(),
            service_editor: service_editor::State::default(),
            log_filter: String::new(),
            log_follow: true,
            log_auto: true,
            log_inflight: false,
            log_clear_confirm: false,
            last_logs: Instant::now(),
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
            workspace_exit_confirm: false,
            planner: crate::planner::State::new(preferences_path.with_file_name("planner.sqlite3")),
            planner_active,
            tray_bridge_stop,
            tray_exit_requested,
            window_handle,
            theme,
            colors: palette(theme),
            preferences, preferences_path: preferences_path.clone(),
            tool_search: String::new(), library_query: String::new(), launcher_query: String::new(), launcher_open: false,
            launcher_focus: false, launcher_index: 0, toast: None,
            launcher_scope: Default::default(),
            #[cfg(feature = "ui-preview")]
            launcher_scope_buttons: [None; 4],
            #[cfg(feature = "ui-preview")]
            workflow_bookmark_button: None,
            #[cfg(feature = "ui-preview")]
            workflow_recent_button: None,
            data_state: crate::workbench::sessions::Workspace::new(preferences_path.with_file_name("workspace.sqlite3")), file_state: FileState::default(), clear_tool_confirm:false,
            plugins: crate::plugin_ui::PluginState::new(preferences_path.parent().unwrap_or(std::path::Path::new(".")).join("plugins")),
            mcp: crate::mcp_ui::McpState::new(preferences_path.parent().unwrap_or(std::path::Path::new(".")).join("mcp-permissions.json")),
            agent: crate::agent_ui::State::new(preferences_path.parent().unwrap_or(std::path::Path::new(".")).join("mcp-permissions.json")),
            agent_records: crate::agent_record_ui::State::default(),
            recorder,
            images: Default::default(),
            markdown: Default::default(),
            file_encoding: Default::default(),
            checksum_manifest: Default::default(),
            disk_inspector: Default::default(),
            duplicate_finder: Default::default(),
            directory_compare: Default::default(),
            sqlite_browser: Default::default(),
            ascii_codes: Default::default(),
            calculator: Default::default(),
            clipboard,
            updates: Default::default(),
            portable_update: Default::default(),
            msi_update: Default::default(),
            delta_update: Default::default(),
            clock: crate::clock::State::new(preferences_path.with_file_name("clock.json"),cc.egui_ctx.clone()),
            prefix: Default::default(),
            symbols: Default::default(),
            ascii_art: Default::default(),
            knowledge_sources: crate::knowledge_sources::State::new(crate::knowledge_sources::default_path()),
            document_ingestion: Default::default(),
            knowledge_index: crate::knowledge_index::State::new(crate::knowledge_index::default_path()),
            vector_index: crate::vector_index::State::new(crate::vector_index::default_path(), crate::knowledge_index::default_path()),
            knowledge_search: crate::knowledge_search::State::new(crate::knowledge_index::default_path()),
            hybrid_search: crate::hybrid_search::State::new(crate::knowledge_index::default_path(), crate::vector_index::default_path()),
            knowledge_answer: crate::knowledge_answer::State::new(crate::knowledge_index::default_path(), crate::vector_index::default_path()),
            embedding: Default::default(),
            knowledge_capture: Default::default(),
            knowledge_eval: crate::knowledge_eval::State::new(crate::knowledge_index::default_path(), crate::vector_index::default_path()),
            integrations: Default::default(), frameworks: Default::default(), home_filter: "全部".into(), home_category: "全部分类".into(), home_query_key: Default::default(),
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
        self.refresh_cancel.store(true, Ordering::Release);
        self.refresh_cancel = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.refresh_cancel);
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        let generation = self.refresh_generation;
        self.last_refresh = Instant::now();
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(BackgroundEvent::Statuses(
                generation,
                manager.list_services_cancellable(&cancelled),
            ));
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
        if self.service_batch.busy() || self.service_pending.contains_key(&service_id) {
            return;
        }
        self.service_pending.insert(service_id.clone(), action);
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
            let _ = tx.send(BackgroundEvent::ServiceAction(service_id, result));
        });
    }

    fn run_all(&mut self, start: bool, ctx: &egui::Context) {
        if self.service_batch.busy() || !self.service_pending.is_empty() {
            self.toast = Some(("已有服务操作进行中，请等待完成".into(), Instant::now()));
            return;
        }
        let config = self.manager.config_snapshot();
        if config.services.len() > 10_000 {
            self.toast = Some(("单次批量最多10000项，请缩小配置范围".into(), Instant::now()));
            return;
        }
        let generation = self.service_batch.begin(config.services.len());
        let cancel = Arc::clone(&self.service_batch.cancel);
        let manager = Arc::clone(&self.manager);
        let tx = self.event_tx.clone();
        let ctx = ctx.clone();
        let window_handle = self.window_handle;
        self.notification = if start {
            "正在启动全部服务…".to_owned()
        } else {
            "正在停止全部服务…".to_owned()
        };
        std::thread::spawn(move || {
            service_batch::run(
                config.services.values().cloned().collect(),
                &cancel,
                |event| {
                    let _ = tx.send(BackgroundEvent::ServiceBatch(generation, event));
                    wake_main_window(window_handle, &ctx);
                },
                |spec| {
                    manager
                        .execute_bound_batch_item(spec, &config.path, &config.state_dir, start)
                        .map(|result| result.map(|r| r.message))
                        .map_err(|e| e.to_string())
                },
            );
        });
    }

    fn request_logs(&mut self, service_id: String) {
        if self.log_view.as_deref() == Some(&service_id) && self.log_inflight {
            return;
        }
        if self.log_view.as_deref() != Some(&service_id) {
            self.log_error = false;
            self.log_text = "正在读取日志…".to_owned();
            self.log_lines.invalidate();
            self.log_filter.clear();
            self.log_clear_confirm = false;
        }
        self.log_view = Some(service_id.clone());
        self.log_inflight = true;
        self.last_logs = Instant::now();
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
                BackgroundEvent::ServiceBatch(generation, event) => {
                    if self.service_batch.apply(generation, event) {
                        let row = self
                            .service_batch
                            .job
                            .snapshot("services", "服务批量操作", true)
                            .unwrap();
                        let result = if row.phase == crate::tasks::Phase::Failed {
                            Err(row.summary)
                        } else {
                            Ok(ActionResult {
                                service_id: "all".into(),
                                message: row.summary,
                            })
                        };
                        let _ = self.event_tx.send(BackgroundEvent::Action(result));
                    }
                }
                BackgroundEvent::ServiceAction(id, result) => {
                    self.service_pending.remove(&id);
                    let _ = self.event_tx.send(BackgroundEvent::Action(result));
                }
                BackgroundEvent::Statuses(generation, statuses) => {
                    if generation != self.refresh_generation {
                        continue;
                    }
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
                        self.log_inflight = false;
                        self.log_error = result.is_err();
                        self.log_text = result.unwrap_or_else(|error| format!("读取失败：{error}"));
                        self.log_lines.invalidate();
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
                    self.refresh_inflight = false;
                    self.request_refresh();
                }
                BackgroundEvent::TrayNavigate(action) => match action {
                    TrayAction::Start(id) => self.run_action(id, "start"),
                    TrayAction::Stop(id) => self.run_action(id, "stop"),
                    TrayAction::Restart(id) => self.run_action(id, "restart"),
                    TrayAction::StartAll => self.run_all(true, ctx),
                    TrayAction::StopAll => self.run_all(false, ctx),
                    TrayAction::RecorderTogglePause => self.recorder.toggle_pause(),
                    TrayAction::RecorderStop => self.recorder.request_stop(),
                    TrayAction::QuickPanel => self.open_quick(ctx),
                    TrayAction::ContextPanel => self.open_tray_context(ctx),
                    TrayAction::ShowWindow => {
                        self.quick_open = false;
                    }
                    TrayAction::Search => {
                        self.quick_open = false;
                        self.open_launcher();
                    }
                    TrayAction::Settings => {
                        self.quick_open = false;
                        self.launcher_open = false;
                        self.page = Page::Settings;
                    }
                    TrayAction::Collection(filter) => {
                        self.quick_open = false;
                        self.launcher_open = false;
                        self.page = Page::Library;
                        self.home_filter = filter;
                        self.home_category = "全部分类".into();
                        self.library_query.clear();
                    }
                    TrayAction::OpenEntry(id) => {
                        self.quick_open = false;
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
                        self.page = if tool.category() == "Java 与 JVM" {
                            Page::Java
                        } else {
                            Page::Django
                        };
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
                    self.refresh_inflight = false;
                    self.request_refresh();
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
        if self.recorder.tray_status() != crate::recorder_ui::TrayRecordingStatus::Idle {
            self.minimize_for_recording(ctx);
            return;
        }
        if !hide_main_window(self.window_handle) {
            self.minimize_for_recording(ctx);
        } else {
            ctx.request_repaint();
        }
    }
    fn minimize_for_recording(&self, ctx: &egui::Context) {
        #[cfg(windows)]
        if let Some(handle) = self.window_handle {
            use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SW_MINIMIZE, ShowWindow};
            let window = handle as windows_sys::Win32::Foundation::HWND;
            unsafe {
                if IsWindow(window) != 0 {
                    ShowWindow(window, SW_MINIMIZE);
                }
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        ctx.request_repaint();
    }

    fn open_launcher(&mut self) {
        self.launcher_open = true;
        self.launcher_focus = true;
        self.launcher_query.clear();
        self.launcher_index = 0;
    }
    fn navigate(&mut self, page: Page, kind: Option<ToolKind>) {
        self.page = page;
        if matches!(page, Page::Notes | Page::Calendar) {
            self.planner.calendar = page == Page::Calendar;
        }
        self.tool_search.clear();
        if let Some(kind) = kind {
            self.tool_state.select(kind);
        }
        self.launcher_open = false;
        let id = kind.map(|k| k.id().to_owned()).or_else(|| {
            catalog()
                .iter()
                .find(|e| e.page == page && e.kind.is_none())
                .map(|e| e.id.clone())
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

    #[cfg(feature = "ui-preview")]
    pub fn preview_date_fixture(&mut self) {
        self.calculator.preview_date_fixture();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_date_position(&self) -> egui::Pos2 {
        self.calculator.preview_date_position()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_date_check(&mut self, phase: u8) {
        self.calculator.preview_date_check(phase);
    }

    pub fn open_startup_tool(&mut self, id: &str) {
        if let Some(entry) = self.entries("").into_iter().find(|e| e.id == id) {
            self.open_entry(&entry);
        } else {
            self.page = Page::Library;
            self.toast = Some((
                format!("工具“{id}”不存在或插件尚未启用，请在工具库中选择。"),
                Instant::now(),
            ));
        }
    }

    fn entries(&self, query: &str) -> Vec<ToolEntry> {
        let plugins: Vec<_> = self
            .plugins
            .store
            .tool_refs()
            .map(|(id, tool)| ToolEntry::plugin(id, tool))
            .collect();
        registry::ranked(
            catalog().iter().chain(plugins.iter()),
            query,
            &self.preferences,
        )
    }
    fn visit(&mut self, id: &str) {
        self.preferences.visit(id);
        if let Err(e) = self.preferences.save(&self.preferences_path) {
            self.toast = Some((e.to_string(), Instant::now()));
        }
    }
    fn open_entry(&mut self, e: &ToolEntry) {
        if e.page == Page::Data {
            self.data_state.set_active_tool(&e.id);
        }
        if e.page == Page::Recorder {
            self.recorder.select_entry(&e.id);
        }
        if matches!(e.page, Page::Notes | Page::Calendar) {
            self.planner.calendar = e.page == Page::Calendar;
        }
        if e.id == "screenshot-workbench" {
            self.images.show_screenshot();
        } else if e.id == "recorder-tutorial" {
            self.recorder.show_tutorial();
        } else if e.id == "image-crop-annotate" {
            self.images.show_editor();
        } else if e.id == "image-metadata" {
            self.images.show_metadata();
        } else if e.id == "image-batch" {
            self.images.show_batch();
        } else if e.id == "csv-merge" {
            self.data_state.show_join();
        } else if e.id == "data-transform" {
            self.data_state.show_transform();
        } else if e.id == "data-sqlite-export" {
            self.data_state.show_sqlite_export();
        } else if e.id == "pipeline" {
            self.data_state.show_workflow();
        } else if e.id == "text-flow" {
            self.data_state.show_text_flow();
        } else if matches!(
            e.id.as_str(),
            "screen-recorder-audio-mix"
                | "screen-recorder-audio-gain"
                | "screen-recorder-audio-meter"
        ) {
            self.recorder
                .select_audio_mode(crate::recorder::AudioMode::SystemAndMicrophone);
        } else if let Some(tool) = crate::framework::Tool::from_id(&e.id) {
            self.frameworks.select(tool);
        } else if e.id.starts_with("plugin:") {
            self.plugins.select(&e.id);
        } else if e.page == Page::Plugins {
            self.plugins.selected = None;
        }
        if let Some(kind) = e.kind {
            self.tool_state.select(kind);
        }
        self.page = e.page;
        self.tool_search.clear();
        self.launcher_open = false;
        // Record the requested entry once, including aliases sharing a workbench.
        self.visit(&e.id);
    }
    fn launcher(&mut self, ctx: &egui::Context) {
        if !self.launcher_open {
            return;
        }
        let mut open = true;
        let mut chosen = None;
        let mut bookmark_action = None;
        let mut forget_recent = None;
        egui::Window::new("快速打开工具与流程")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 70.0])
            .show(ctx, |ui| {
                let response = ui.add_sized(
                    [ui.available_width(), 38.0],
                    egui::TextEdit::singleline(&mut self.launcher_query)
                        .hint_text("工具用途或流程名称，例如 压缩图片、每日清洗…")
                        .char_limit(160),
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
                let old_scope = self.launcher_scope;
                ui.horizontal(|ui| {
                    for (index, (scope, label, key)) in launcher::Scope::ALL.into_iter().enumerate() {
                        let _tab = ui.selectable_value(&mut self.launcher_scope, scope, label)
                            .on_hover_text(format!("Ctrl+{}切换；保留搜索词", index + 1));
                        #[cfg(feature = "ui-preview")]
                        { self.launcher_scope_buttons[index] = Some(_tab.rect.intersect(ui.clip_rect())); }
                        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, key)) {
                            self.launcher_scope = scope;
                        }
                    }
                });
                if old_scope != self.launcher_scope {
                    self.launcher_index = 0;
                    self.launcher_focus = true;
                    scroll_selection = true;
                }
                let query = self.launcher_query.to_lowercase();
                let launcher::Results {tools: entries, workflows, saved, recent} = self.launcher_results(&query);
                let count = saved.len() + recent.len() + entries.len() + workflows.entries.len();
                if self.preferences.workflow_history_pending() {
                    ui.colored_label(self.colors.amber, "最近载入记录尚未保存，保留在当前窗口并稍后重试。");
                }
                ui.label(RichText::new(format!("工具 {} · 收藏流程 {} · 最近载入 {} · 已检查流程 {}", entries.len(), saved.len(), recent.len(), workflows.total))
                    .size(12.0).color(self.colors.muted));
                if count == 0 {
                    ui.label(match self.launcher_scope {
                        launcher::Scope::Tools => "没有匹配的工具；可简化关键词或切换到全部。",
                        launcher::Scope::Favorites => "没有匹配的收藏；可在工具库收藏工具，或在流程搜索结果旁点☆。",
                        _ => "没有匹配的工具或流程；可在数据工作台选择或刷新流程文件夹，也可切换搜索范围。",
                    });
                } else {
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
                    {
                        scroll_selection = true;
                        self.launcher_index = (self.launcher_index + 1).min(count - 1);
                    }
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                        scroll_selection = true;
                        self.launcher_index = self.launcher_index.saturating_sub(1);
                    }
                    self.launcher_index = self.launcher_index.min(count - 1);
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .show(ui, |ui| {
                            for (index, saved_index) in saved.iter().enumerate() {
                                let workflow = &self.preferences.workflow_favorites[*saved_index];
                                let response = ui
                                    .horizontal(|ui| {
                                        let response = ui.selectable_label(
                                            index == self.launcher_index,
                                            format!(
                                                "{} · 收藏流程 · {}步",
                                                workflow.name, workflow.steps
                                            ),
                                        );
                                        let star = ui
                                            .small_button("★")
                                            .on_hover_text("移除收藏；保留原文件");
                                        #[cfg(feature = "ui-preview")]
                                        if index == 0 {
                                            self.workflow_bookmark_button =
                                                Some(star.rect.intersect(ui.clip_rect()));
                                        }
                                        if star.clicked() {
                                            bookmark_action = Some(Ok(workflow.clone()));
                                        }
                                        response
                                    })
                                    .inner;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{} · 摘要来自收藏时；重新读取后在当前实例确认",
                                            workflow
                                                .path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                        ))
                                        .size(12.0)
                                        .color(self.colors.muted),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(workflow.path.display().to_string());
                                if scroll_selection && index == self.launcher_index {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    chosen =
                                        Some(launcher::Choice::SavedWorkflow(workflow.clone()));
                                }
                                ui.add_space(6.0);
                            }
                            for (offset, index) in recent.iter().enumerate() {
                                let entry = &self.preferences.workflow_recent[*index];
                                let row_index = saved.len() + offset;
                                let response = ui.horizontal(|ui| {
                                    let response = ui.selectable_label(row_index == self.launcher_index,
                                        format!("{} · 最近载入 · {}步", entry.name, entry.steps));
                                    let remove = ui.small_button("×").on_hover_text("移除最近记录；保留原文件和当前数据");
                                    #[cfg(feature = "ui-preview")]
                                    if offset == 0 { self.workflow_recent_button = Some(remove.rect.intersect(ui.clip_rect())); }
                                    if remove.clicked() {forget_recent = Some(entry.clone());}
                                    response
                                }).inner;
                                ui.add(egui::Label::new(RichText::new(format!("{} · 载入记录不代表已应用；重新读到当前实例确认",
                                    entry.path.file_name().unwrap_or_default().to_string_lossy())).size(12.0).color(self.colors.muted)).truncate()).on_hover_text(entry.path.display().to_string());
                                if scroll_selection && row_index == self.launcher_index { response.scroll_to_me(Some(egui::Align::Center)); }
                                if response.clicked() { chosen = Some(launcher::Choice::SavedWorkflow(entry.clone())); }
                                ui.add_space(6.0);
                            }
                            for (i, e) in entries.iter().enumerate() {
                                let i = saved.len() + recent.len() + i;
                                let response = ui.selectable_label(
                                    i == self.launcher_index,
                                    format!("{}   ·   {}   ·   {}", e.title, e.category, e.badge()),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&e.description)
                                            .size(12.0)
                                            .color(self.colors.muted),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&e.description);
                                if !query.is_empty() {
                                    ui.small(e.match_hint(&query));
                                }
                                ui.add_space(6.0);
                                if scroll_selection && i == self.launcher_index {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    chosen = Some(launcher::Choice::Tool(e.id.clone()));
                                }
                            }
                            for (offset, workflow) in workflows.entries.iter().enumerate() {
                                let index = saved.len() + recent.len() + entries.len() + offset;
                                let response = ui
                                    .horizontal(|ui| {
                                        let response = ui.selectable_label(
                                            index == self.launcher_index,
                                            format!(
                                                "{} · 已保存流程 · {}步",
                                                workflow.name, workflow.steps
                                            ),
                                        );
                                        let favorite = self.data_state.workflow_is_bookmarked(
                                            workflow,
                                            &self.preferences.workflow_favorites,
                                        );
                                        let star = ui
                                            .small_button(if favorite { "★" } else { "☆" })
                                            .on_hover_text(if favorite {
                                                "移除收藏；保留原文件"
                                            } else {
                                                "收藏流程位置和摘要；下次直接搜索，不保存表格或授权"
                                            });
                                        #[cfg(feature = "ui-preview")]
                                        if offset == 0 {
                                            self.workflow_bookmark_button =
                                                Some(star.rect.intersect(ui.clip_rect()));
                                        }
                                        if star.clicked() {
                                            bookmark_action =
                                                Some(self.data_state.workflow_bookmark(workflow));
                                        }
                                        response
                                    })
                                    .inner;
                                let detail = format!(
                                    "{} · {} · 载入后确认，不自动执行",
                                    workflow.instance_name,
                                    workflow.file_name.to_string_lossy()
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&detail).size(12.0).color(self.colors.muted),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(detail);
                                ui.add_space(6.0);
                                if scroll_selection && index == self.launcher_index {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    chosen = Some(launcher::Choice::Workflow(workflow.clone()));
                                }
                            }
                        });
                    if workflows.total > workflows.entries.len() {
                        ui.small(format!(
                            "流程匹配{}条，仅显示前100条；请细化名称或文件名。",
                            workflows.total
                        ));
                    }
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        chosen = Some(if let Some(index) = saved.get(self.launcher_index) {
                            launcher::Choice::SavedWorkflow(
                                self.preferences.workflow_favorites[*index].clone(),
                            )
                        } else if let Some(index) = recent.get(self.launcher_index - saved.len()) {
                            launcher::Choice::SavedWorkflow(self.preferences.workflow_recent[*index].clone())
                        } else if let Some(entry) = entries.get(self.launcher_index - saved.len() - recent.len()) {
                            launcher::Choice::Tool(entry.id.clone())
                        } else {
                            launcher::Choice::Workflow(
                                workflows.entries
                                    [self.launcher_index - saved.len() - recent.len() - entries.len()]
                                .clone(),
                            )
                        });
                    }
                }
                ui.separator();
                ui.label(
                    RichText::new("↑ ↓ 选择     Enter 打开     Esc 关闭     Ctrl 1–4 切换范围")
                        .small()
                        .weak(),
                );
            });
        if !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.launcher_open = false;
        }
        if let Some(choice) = chosen {
            self.open_search_choice(choice);
        }
        if let Some(bookmark) = bookmark_action {
            match bookmark {
                Ok(bookmark) => self.toggle_workflow_bookmark(bookmark),
                Err(error) => self.toast = Some((format!("{error:#}"), Instant::now())),
            }
        }
        if let Some(entry) = forget_recent {
            self.forget_workflow_recent(entry);
        }
    }

    fn forget_workflow_recent(&mut self, entry: crate::preferences::SavedWorkflow) {
        match self
            .preferences
            .forget_workflow_load(&self.preferences_path, &entry)
        {
            Ok(()) => self.toast = Some(("已移除最近载入记录，原文件保留".into(), Instant::now())),
            Err(error) => self.toast = Some((format!("最近记录未移除：{error:#}"), Instant::now())),
        }
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
        // Leave space for actions and results in the minimum desktop viewport.
        let editor_height = ((ctx.screen_rect().height() - 440.0) / 2.0).clamp(72.0, 180.0);
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
                            !self.tool_state.input.is_empty() || self.tool_state.has_result() || !self.tool_state.message.is_empty(),
                            egui::Button::new("清空").small(),
                        )
                        .clicked()
                    {
                        self.clear_tool_confirm = true;
                    }
                });
                let paired_text = self.tool_state.has_plugin_mode() && ui.available_width() >= 520.0;
                if paired_text {
                    ui.add_space(6.0);
                } else {
                    ui.label(RichText::new(self.tool_state.selected.description()).small().color(p.muted));
                    ui.add_space(12.0);
                }
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
                                self.tool_state.apply_result(Ok(generate_uuid()));
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
                        if self.tool_state.has_plugin_mode()
                            && ui.checkbox(&mut self.tool_state.plugin_compatible, "插件兼容模式").on_hover_text("8192 UTF-8 字节；JSON 拒绝重复键、超过 64 层和不安全整数；Base64 严格校验，不忽略空白").changed() {
                                self.tool_state.clear_result();
                        }
                        if !help.is_empty() {
                            ui.label(RichText::new(help).small().color(p.muted));
                            ui.add_space(8.0);
                        }
                        self.tool_actions(ui);
                        ui.add_space(8.0);
                        if paired_text {
                            let paired_height = (editor_height - 24.0).max(64.0);
                            ui.columns(2, |columns| {
                                columns[0].horizontal(|ui| {
                                    ui.strong("输入");
                                    let over = self.tool_state.plugin_compatible && self.tool_state.input.len() > zi_text_core::TEXT_LIMIT;
                                    ui.label(RichText::new(if self.tool_state.plugin_compatible {
                                        format!("{} / {} 字节{}", self.tool_state.input.len(), zi_text_core::TEXT_LIMIT, if over { " · 超限" } else { "" })
                                    } else { format!("{} 字节", self.tool_state.input.len()) }).small().color(if over { p.red } else { p.muted }));
                                });
                                egui::ScrollArea::vertical().id_salt(("text-input", self.tool_state.selected.id())).max_height(paired_height).auto_shrink([false, false]).show(&mut columns[0], |ui| {
                                    ui.add_sized([ui.available_width(), paired_height], egui::TextEdit::multiline(&mut self.tool_state.input).font(egui::TextStyle::Monospace).hint_text("粘贴需要处理的内容…"));
                                });
                                self.tool_output(&mut columns[1], ctx, paired_height);
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.strong(if self.tool_state.selected == ToolKind::JsonDiff { "左侧 JSON" } else { "输入" });
                                ui.label(RichText::new(format!("{} 字符 · {} 字节", self.tool_state.input.chars().count(), self.tool_state.input.len())).small().color(p.muted));
                            });
                            if self.tool_state.plugin_compatible && self.tool_state.has_plugin_mode() {
                                let over = self.tool_state.input.len() > zi_text_core::TEXT_LIMIT;
                                ui.label(RichText::new(format!("{} / {} UTF-8 字节{}", self.tool_state.input.len(), zi_text_core::TEXT_LIMIT, if over { " · 请缩短输入" } else { "" })).small().color(if over { p.red } else { p.muted }));
                            }
                            ui.add_sized([ui.available_width(), if self.tool_state.selected == ToolKind::JsonDiff { 88.0 } else { editor_height }], egui::TextEdit::multiline(&mut self.tool_state.input).font(egui::TextStyle::Monospace).hint_text("在这里粘贴需要处理的内容…"));
                        }
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
                    }
                }
                if !paired_text {
                    self.tool_output(ui, ctx, editor_height);
                }
            });
    }

    fn tool_output(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, editor_height: f32) {
        let p = self.colors;
        let empty_result = self.tool_state.has_result() && self.tool_state.output.is_empty();
        ui.horizontal_wrapped(|ui| {
            ui.label("上次结果");
            let copy = ui
                .add_enabled(
                    self.tool_state.has_result(),
                    egui::Button::new(if empty_result {
                        "复制空结果"
                    } else {
                        "复制结果"
                    })
                    .small(),
                )
                .on_hover_text(if empty_result {
                    "复制空结果会将当前剪贴板内容替换为空文本"
                } else {
                    "复制完整处理结果"
                });
            #[cfg(feature = "ui-preview")]
            {
                self.preview_text_copy = Some((copy.rect, copy.enabled()));
            }
            if copy.clicked() {
                ctx.copy_text(self.tool_state.output.clone());
                self.toast = Some(("结果已复制".into(), Instant::now()));
            }
            if ui
                .add_enabled(
                    self.tool_state.has_result(),
                    egui::Button::new("交换输入 / 输出").small(),
                )
                .clicked()
            {
                std::mem::swap(&mut self.tool_state.input, &mut self.tool_state.output);
            }
        });
        if !self.tool_state.message.is_empty() {
            ui.label(RichText::new(&self.tool_state.message).color(p.red));
            return;
        }
        if self.tool_state.has_result() && self.tool_state.output.is_empty() {
            ui.label(RichText::new("处理成功 · 结果为空文本（0 字节）").color(p.muted));
            return;
        }
        let mut output = self.tool_state.output.as_str();
        egui::ScrollArea::vertical()
            .id_salt(("text-result", self.tool_state.selected.id()))
            .max_height(editor_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), editor_height],
                    egui::TextEdit::multiline(&mut output)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("处理结果将显示在这里；可以选择文本或点击复制结果"),
                );
            });
    }

    fn tool_actions(&mut self, ui: &mut egui::Ui) {
        let p = self.colors;
        let mut selected = None;
        let shortcut = (self.handoff.is_none() && !self.images.relay_active())
            && !self.launcher_open
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
            let result = self.tool_state.run(index);
            if self.tool_state.apply_result(result) {
                self.toast = Some((
                    if self.tool_state.output.is_empty() {
                        "处理完成，结果为空文本"
                    } else {
                        "处理完成"
                    }
                    .into(),
                    Instant::now(),
                ));
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

    fn check_updates(&mut self, ctx: &egui::Context, automatic: bool) {
        let mut next = self.preferences.clone();
        next.updates.last_attempt = Some(chrono::Utc::now().timestamp());
        let saved = next.save(&self.preferences_path);
        if saved.is_ok() {
            self.preferences = next;
        }
        if !automatic || saved.is_ok() {
            self.updates.begin(self.preferences.updates.channel, ctx);
        }
        if saved.is_err() {
            self.updates.message = if automatic {
                "无法保存检查时间，已跳过自动检查；可以手动重试。"
            } else {
                "本次检查时间未能保存，检查仍可继续。"
            }
            .into();
            if automatic {
                self.preferences.updates.automatic = false;
            }
        }
    }

    fn updates_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (policy, check) = self.updates.ui(ui, &self.preferences.updates);
        if let Some(policy) = policy {
            let mut next = self.preferences.clone();
            next.updates = policy;
            match next.save(&self.preferences_path) {
                Ok(()) => {
                    if self.preferences.updates.channel != next.updates.channel {
                        self.updates.clear_result();
                    }
                    self.preferences = next;
                    self.updates.message.clear();
                }
                Err(_) => {
                    self.updates.message = "设置未能保存，原设置仍生效。".into();
                }
            }
        }
        if check {
            self.check_updates(ctx, false);
        }
        let report = self
            .updates
            .result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|r| r.clone());
        let blockers: Vec<&str> = [
            (self.data_state.has_work(), "数据工作台"),
            (
                self.planner.has_unsaved() || self.planner.saving(),
                "备忘录与日程",
            ),
            (
                self.prefix.has_work(&self.preferences.command_bindings)
                    || self.prefix.files.busy(),
                "快捷指令草稿",
            ),
            (self.clock.has_work() || self.clock.saving(), "时钟检查点"),
            (self.clipboard.has_pending(), "剪贴板历史或规则"),
            (self.calculator.has_work(), "计算工作表"),
            (self.delta_update.has_work(), "更新包工作台"),
            (self.portable_update.busy(), "便携升级准备"),
            (self.msi_update.busy(), "MSI升级准备"),
            (self.updates.download_has_work(), "更新下载预览"),
            (
                self.images.screenshot_has_work() || self.images.background_active(),
                "截图与图片处理",
            ),
            (self.intake.busy(), "文件导入"),
            (
                self.statuses
                    .iter()
                    .any(|s| s.managed && s.state == ServiceState::Running),
                "运行中的托管服务",
            ),
            (
                self.recorder.tray_status() != crate::recorder_ui::TrayRecordingStatus::Idle,
                "录屏",
            ),
            (self.file_state.job.phase.active(), "文件校验"),
            (self.network_state.busy, "网络工具"),
            (self.http_state.tabs.iter().any(|t| t.busy), "HTTP请求"),
            (self.plugins.is_running(), "插件工具"),
            (self.agent.background_active(), "Agent"),
            (self.agent_records.background_active(), "Agent运行记录"),
            (self.mcp.background_active(), "MCP"),
            (self.knowledge_answer.background_active(), "知识问答"),
            (self.embedding.background_active(), "向量对比"),
            (self.knowledge_capture.background_active(), "知识采集"),
            (self.document_ingestion.background_active(), "文档入库"),
            (self.knowledge_index.background_active(), "关键词索引"),
            (self.vector_index.background_active(), "向量索引"),
            (self.knowledge_eval.background_active(), "RAG评测"),
            (self.knowledge_sources.background_active(), "知识来源"),
            (self.knowledge_search.background_active(), "知识搜索"),
            (self.hybrid_search.background_active(), "混合检索"),
            (self.checksum_manifest.background_active(), "校验清单"),
            (self.disk_inspector.background_active(), "磁盘分析"),
            (self.duplicate_finder.background_active(), "重复文件"),
            (self.directory_compare.background_active(), "目录比较"),
            (self.sqlite_browser.background_active(), "SQLite"),
            (self.frameworks.background_active(), "Java/Django"),
            (self.integrations.background_active(), "本机集成"),
        ]
        .into_iter()
        .filter_map(|(active, name)| active.then_some(name))
        .collect();
        let exit_for_upgrade = if self.msi_update.available() {
            self.msi_update
                .ui(ui, report, &blockers, &self.manager.config_snapshot().path)
        } else {
            self.portable_update
                .ui(ui, report, &blockers, &self.manager.config_snapshot().path)
        };
        self.msi_update.records_ui(ui);
        if exit_for_upgrade {
            self.quit_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading(RichText::new("设置").size(28.0));
        if ui.button("程序更新 · 版本与发布说明").clicked() {
            self.page = Page::Updates;
            self.visit("app-update-check");
        }
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
                                self.refresh_inflight = false;
                                self.request_refresh();
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
                ui.label(RichText::new("全局快捷启动器").strong());
                ui.checkbox(&mut self.hotkey_edit.enabled, "启用系统全局快捷键");
                ui.add(egui::TextEdit::singleline(&mut self.hotkey_edit.shortcut).hint_text("Ctrl+Alt+Space"));
                ui.small("支持 Ctrl / Alt / Shift 与 A–Z、F1–F12、Space；至少包含 Ctrl 或 Alt。");
                if ui.button("应用快捷键").clicked() { self.hotkey.configure(self.hotkey_edit.clone()); self.hotkey_status = "正在应用…".into(); }
                ui.label(&self.hotkey_status);
                if ui.button("打开快捷面板").clicked() { self.open_quick(ctx); }
                ui.small(&self.recorder_hotkey_status);
                ui.separator();
                ui.label(RichText::new("托盘行为").strong());
                ui.label("关闭窗口时程序继续驻留托盘，托管服务保持运行。左键打开快捷面板；右键可搜索工具、打开收藏/最近/常用、按分类访问插件和内置工具，并控制服务。");
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
        if let Some(result) = self.service_editor.poll() {
            self.notification_error = result.is_err();
            self.notification = result.err().unwrap_or_else(|| "服务定义已更新".into());
            if !self.notification_error {
                self.config_error = None;
                match TrayController::new(self.manager.config_snapshot().services.values()) {
                    Ok(tray) => self.tray = Some(tray),
                    Err(e) => {
                        self.notification = format!("定义已保存，托盘重建失败：{e}");
                        self.notification_error = true;
                    }
                }
                self.last_refresh = Instant::now() - Duration::from_secs(30);
                self.refresh_inflight = false;
                self.request_refresh();
            }
        }
        self.service_editor.ui(ctx, &self.manager);
        if let Some(service_id) = self.log_view.clone() {
            if self.log_auto
                && self.last_logs.elapsed() >= Duration::from_secs(2)
                && !self.log_inflight
            {
                self.request_logs(service_id.clone());
            }
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
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("刷新").clicked() {
                            self.request_logs(service_id.clone());
                        }
                        if ui
                            .button(if self.log_clear_confirm {
                                "确认清空"
                            } else {
                                "清空日志…"
                            })
                            .clicked()
                        {
                            if self.log_clear_confirm {
                                self.clear_logs(service_id.clone());
                                self.log_clear_confirm = false;
                            } else {
                                self.log_clear_confirm = true;
                            }
                        }
                        if self.log_clear_confirm && ui.small_button("取消清空").clicked() {
                            self.log_clear_confirm = false;
                        }
                        if ui.button("复制全部预览").clicked() {
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
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.log_auto, "自动刷新（2 秒）");
                        ui.checkbox(&mut self.log_follow, "跟随末尾");
                        if self.log_inflight {
                            ui.spinner();
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.log_filter)
                                .hint_text("筛选日志行…")
                                .char_limit(160)
                                .desired_width(200.0),
                        );
                    });
                    ui.label(
                        RichText::new("预览末尾最多 5000 行 / 4 MiB；原始文件不截断")
                            .small()
                            .color(p.muted),
                    );
                    ui.separator();
                    self.log_lines.update(&self.log_text, &self.log_filter);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "匹配 {} / {} 行",
                            self.log_lines.count(),
                            self.log_lines.total()
                        ));
                        if ui
                            .add_enabled(!self.log_filter.is_empty(), egui::Button::new("清除筛选"))
                            .clicked()
                        {
                            self.log_filter.clear();
                            self.log_lines.update(&self.log_text, "");
                        }
                        if ui
                            .add_enabled(
                                self.log_lines.count() > 0,
                                egui::Button::new("复制筛选结果"),
                            )
                            .clicked()
                        {
                            ctx.copy_text(self.log_lines.copy_matches(&self.log_text));
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let ready = !self.log_inflight && !self.log_error && self.handoff.is_none();
                        let send_all = ui.add_enabled(
                            ready && !self.log_text.is_empty(),
                            egui::Button::new("发送全部预览到工具…"),
                        );
                        #[cfg(feature = "ui-preview")]
                        self.preview_services
                            .insert("log-send-all", (send_all.rect, ui.clip_rect()));
                        if send_all.clicked() {
                            self.send_service_logs(&service_id, false);
                        }
                        let send_filtered = ui.add_enabled(
                            ready && self.log_lines.count() > 0,
                            egui::Button::new("发送筛选结果到工具…"),
                        );
                        #[cfg(feature = "ui-preview")]
                        self.preview_services
                            .insert("log-send-filtered", (send_filtered.rect, ui.clip_rect()));
                        if send_filtered.clicked() {
                            self.send_service_logs(&service_id, true);
                        }
                        ui.small("发送时捕获快照；最大 2 MiB；选择目标后确认接收");
                    });
                    if self.log_lines.count() == 0 {
                        ui.label(if self.log_lines.total() == 0 {
                            "日志尚无内容"
                        } else {
                            "没有匹配的日志行，请调整筛选条件"
                        });
                    }
                    service_logs::show(ui, &self.log_lines, &self.log_text, self.log_follow);
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

fn start_tray_bridge(
    _manager: Arc<ServiceManager>,
    tx: Sender<BackgroundEvent>,
    ctx: egui::Context,
    window_handle: Option<isize>,
    stop: Arc<AtomicBool>,
    exit_requested: Arc<AtomicBool>,
    quick_active: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            if quick_active.load(Ordering::Acquire) {
                wake_main_window(window_handle, &ctx);
            }
            for action in TrayController::poll_actions() {
                match action {
                    TrayAction::QuickPanel | TrayAction::ContextPanel => {
                        let _ = tx.send(BackgroundEvent::TrayNavigate(action));
                        wake_main_window(window_handle, &ctx);
                    }
                    TrayAction::ShowWindow => {
                        let _ = tx.send(BackgroundEvent::TrayNavigate(TrayAction::ShowWindow));
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
                    action @ (TrayAction::RecorderTogglePause | TrayAction::RecorderStop) => {
                        let _ = tx.send(BackgroundEvent::TrayNavigate(action));
                        ctx.request_repaint();
                    }
                    TrayAction::Exit => {
                        exit_requested.store(true, Ordering::Release);
                        restore_main_window(window_handle, &ctx);
                    }
                    action => {
                        let _ = tx.send(BackgroundEvent::TrayNavigate(action));
                        wake_main_window(window_handle, &ctx);
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

fn wake_main_window(window_handle: Option<isize>, ctx: &egui::Context) {
    ctx.request_repaint_of(egui::ViewportId::ROOT);
    #[cfg(windows)]
    if let Some(handle) = window_handle {
        // Trigger the event loop without restoring or resizing the hidden workbench.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                handle as _,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_PAINT,
                0,
                0,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = window_handle;
}
fn panel_origin(pixels_per_point: f32) -> Option<(egui::Pos2, egui::Vec2)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            },
            UI::WindowsAndMessaging::GetCursorPos,
        };
        let mut point = POINT { x: 0, y: 0 };
        let mut monitor: MONITORINFO = unsafe { std::mem::zeroed() };
        monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        unsafe {
            if GetCursorPos(&mut point) == 0
                || GetMonitorInfoW(
                    MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                ) == 0
            {
                return None;
            }
        }
        let scale = pixels_per_point.max(0.5);
        let rect = monitor.rcWork;
        let size = egui::vec2(
            460.0_f32.min((rect.right - rect.left) as f32 / scale - 16.0),
            620.0_f32.min((rect.bottom - rect.top) as f32 / scale - 16.0),
        );
        let x = (point.x as f32 / scale - size.x / 2.0).clamp(
            rect.left as f32 / scale + 8.0,
            (rect.right as f32 / scale - size.x - 8.0).max(rect.left as f32 / scale + 8.0),
        );
        let y = (point.y as f32 / scale - size.y).clamp(
            rect.top as f32 / scale + 8.0,
            (rect.bottom as f32 / scale - size.y - 8.0).max(rect.top as f32 / scale + 8.0),
        );
        Some((egui::pos2(x, y), size))
    }
    #[cfg(not(windows))]
    {
        let _ = pixels_per_point;
        None
    }
}

fn restore_main_window(window_handle: Option<isize>, ctx: &egui::Context) {
    #[cfg(windows)]
    if let Some(handle) = window_handle {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsIconic, IsWindow, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
        };
        let window = handle as windows_sys::Win32::Foundation::HWND;
        // The handle comes from this app's eframe CreationContext and is checked before use.
        unsafe {
            if IsWindow(window) != 0 {
                set_main_window_cloaked(window_handle, false);
                ShowWindow(
                    window,
                    if IsIconic(window) != 0 {
                        SW_RESTORE
                    } else {
                        SW_SHOW
                    },
                );
                SetForegroundWindow(window);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.request_repaint();
                return;
            }
        }
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
}

fn hide_main_window(window_handle: Option<isize>) -> bool {
    #[cfg(windows)]
    if let Some(handle) = window_handle {
        return set_main_window_cloaked(Some(handle), true);
    }
    #[cfg(not(windows))]
    let _ = window_handle;
    false
}

#[cfg(windows)]
fn set_main_window_cloaked(window_handle: Option<isize>, cloaked: bool) -> bool {
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::{
        Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute},
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, IsWindow, SW_HIDE, SW_SHOWNA, SWP_FRAMECHANGED,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
            ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        },
    };
    static ORIGINAL_STYLE: OnceLock<Mutex<Option<(isize, isize)>>> = OnceLock::new();
    let Some(handle) = window_handle else {
        return false;
    };
    let window = handle as windows_sys::Win32::Foundation::HWND;
    let value = u32::from(cloaked);
    let Ok(mut saved) = ORIGINAL_STYLE.get_or_init(|| Mutex::new(None)).lock() else {
        return false;
    };
    if unsafe { IsWindow(window) } == 0 {
        return false;
    }
    if cloaked {
        if saved.as_ref().is_some_and(|(hwnd, _)| *hwnd == handle) {
            return true;
        }
        let style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        let hidden_style = (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
        unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, hidden_style) };
        if unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } != hidden_style {
            return false;
        }
        let ok = unsafe {
            DwmSetWindowAttribute(
                window,
                DWMWA_CLOAK as u32,
                (&raw const value).cast(),
                std::mem::size_of_val(&value) as u32,
            ) >= 0
                && SetWindowPos(
                    window,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
                ) != 0
        };
        if ok {
            // Force the shell to refresh taskbar membership after switching to a tool window.
            // Show it immediately while DWM-cloaked so eframe continues processing tray events.
            unsafe {
                ShowWindow(window, SW_HIDE);
                ShowWindow(window, SW_SHOWNA);
            }
            *saved = Some((handle, style));
            return true;
        }
        unsafe {
            SetWindowLongPtrW(window, GWL_EXSTYLE, style);
            let zero = 0u32;
            DwmSetWindowAttribute(
                window,
                DWMWA_CLOAK as u32,
                (&raw const zero).cast(),
                std::mem::size_of_val(&zero) as u32,
            );
        }
        false
    } else {
        let ok = unsafe {
            DwmSetWindowAttribute(
                window,
                DWMWA_CLOAK as u32,
                (&raw const value).cast(),
                std::mem::size_of_val(&value) as u32,
            ) >= 0
        };
        if let Some((hwnd, style)) = saved.take() {
            if hwnd == handle {
                unsafe {
                    SetWindowLongPtrW(window, GWL_EXSTYLE, style);
                    SetWindowPos(
                        window,
                        std::ptr::null_mut(),
                        0,
                        0,
                        0,
                        0,
                        SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
                    );
                }
            } else {
                *saved = Some((hwnd, style));
            }
        }
        ok
    }
}

#[cfg(windows)]
fn main_window_cloaked(window_handle: Option<isize>) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    let Some(handle) = window_handle else {
        return false;
    };
    let mut value = 0u32;
    unsafe {
        DwmGetWindowAttribute(
            handle as _,
            DWMWA_CLOAKED as u32,
            (&raw mut value).cast(),
            std::mem::size_of_val(&value) as u32,
        ) >= 0
            && value != 0
    }
}

impl Drop for DevToolsApp {
    fn drop(&mut self) {
        self.refresh_cancel.store(true, Ordering::Release);
        self.tray_bridge_stop.store(true, Ordering::Release);
    }
}

impl eframe::App for DevToolsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_preferences_refresh.elapsed() >= Duration::from_secs(2) {
            self.last_preferences_refresh = Instant::now();
            if self.preferences.workflow_history_pending()
                && self.last_workflow_retry.elapsed() >= Duration::from_secs(30)
            {
                self.last_workflow_retry = Instant::now();
                if let Err(error) = self.preferences.save(&self.preferences_path) {
                    self.toast = Some((format!("最近载入记录尚未保存：{error:#}"), Instant::now()));
                }
            }
            if self
                .preferences
                .refresh_discovery(&self.preferences_path)
                .unwrap_or(false)
            {
                ctx.request_repaint();
            }
        }
        self.clock
            .persistence_tick(Instant::now(), chrono::Utc::now());
        if self.clock.poll(Instant::now(), chrono::Utc::now()) {
            self.quick_open = false;
            restore_main_window(self.window_handle, ctx);
        }
        if self.planner.poll(ctx) {
            self.quick_open = false;
            restore_main_window(self.window_handle, ctx);
        }
        self.clipboard.poll(ctx);
        self.calculator.poll(ctx);
        self.planner_active.store(
            self.planner.needs_clock() || self.clock.needs_clock() || self.clipboard.needs_clock(),
            Ordering::Release,
        );
        self.mcp.tick(ctx);
        if self.recorder.poll() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.poll_tasks(ctx);
        if let Some(message) = self.frameworks.poll() {
            self.toast = Some((message, Instant::now()));
        }
        if self.frameworks.is_running() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if !matches!(self.page, Page::Java | Page::Django) {
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
        let hotkey_events: Vec<_> = self.hotkey.events.try_iter().collect();
        for event in hotkey_events {
            match event {
                crate::hotkey::Event::Triggered if !self.images.relay_active() => {
                    self.open_quick(ctx);
                    self.prefix.open();
                    self.quick_focus = true;
                }
                crate::hotkey::Event::Triggered => {
                    restore_main_window(self.window_handle, ctx);
                }
                crate::hotkey::Event::Configured(setting, result) => match result {
                    Ok(()) => {
                        self.hotkey_status = if setting.enabled {
                            format!("已启用 {} · 可在其他应用中唤起", setting.shortcut)
                        } else {
                            "全局快捷键已关闭；仍可从托盘打开".into()
                        };
                        self.preferences.hotkey = setting;
                        if let Err(error) = self.preferences.save(&self.preferences_path) {
                            self.hotkey_status = format!("快捷键已生效，但保存失败：{error}");
                        }
                    }
                    Err(error) => self.hotkey_status = error,
                },
            }
        }
        let recorder_events: Vec<_> = self.recorder_hotkeys.events.try_iter().collect();
        for event in recorder_events {
            match event {
                crate::hotkey::RecorderEvent::Registration(status) => {
                    self.recorder_hotkey_status = status;
                }
                crate::hotkey::RecorderEvent::Action(crate::hotkey::RecorderAction::Start) => {
                    self.page = Page::Recorder;
                    if !self.recorder.request_start(self.tray.is_some()) {
                        restore_main_window(self.window_handle, ctx);
                    }
                }
                crate::hotkey::RecorderEvent::Action(
                    crate::hotkey::RecorderAction::TogglePause,
                ) => {
                    self.recorder.toggle_pause();
                }
                crate::hotkey::RecorderEvent::Action(crate::hotkey::RecorderAction::Stop) => {
                    self.recorder.request_stop();
                }
            }
        }
        if let Some(result) = self.intake.poll() {
            match result.and_then(|value| self.apply_import(value).map_err(|e| e.to_string())) {
                Ok(()) => self.toast = Some(("文件已导入".into(), Instant::now())),
                Err(error) => {
                    self.intake.message = error;
                    self.page = Page::Intake;
                }
            }
        }
        if self.intake.busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.updates.poll();
        self.portable_update.poll();
        self.msi_update.poll();
        self.delta_update.poll(ctx);
        let now = chrono::Utc::now().timestamp();
        if self.updates.automatic_due(&self.preferences.updates, now) {
            self.check_updates(ctx, true);
        }
        if self.preferences.updates.automatic {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
        self.prefix.files.poll();
        self.images.poll_screenshot(ctx);
        if let Some(picture) = self.clipboard.take_image_relay() {
            match self.images.start_clipboard_relay(ctx, picture.png.clone()) {
                Ok(()) => {
                    self.page = Page::Images;
                    self.quick_open = false;
                }
                Err(error) => self.toast = Some((error.to_string(), Instant::now())),
            }
        }

        if self.images.take_screenshot_capture_request() {
            self.quick_open = false;
            if hide_main_window(self.window_handle) {
                self.images
                    .start_screenshot_capture(ctx, self.window_handle);
            } else {
                self.images.screenshot_capture_failed();
            }
        }
        if self.images.screenshot_overlay_ui(ctx) {
            self.page = Page::Images;
            self.images.show_screenshot();
            self.quick_open = false;
            restore_main_window(self.window_handle, ctx);
        }
        if self.prefix.files.busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.receive_drop(ctx);
        self.quick_panel(ctx);
        if self.clock.window_ui(ctx) {
            self.page = Page::Clock;
            restore_main_window(self.window_handle, ctx);
        }
        self.quick_active.store(self.quick_open, Ordering::Release);
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
        if let Some(tray) = &mut self.tray {
            tray.update_recorder(self.recorder.tray_status());
            if let Err(error) = tray.sync_navigation(model) {
                self.notification = format!("托盘快捷菜单更新失败：{error}");
                self.notification_error = true;
            }
        }

        if self.tray_exit_requested.swap(false, Ordering::AcqRel) {
            if self.data_state.has_work()
                || self.planner.has_unsaved()
                || self.prefix.has_work(&self.preferences.command_bindings)
                || self.clock.has_work()
                || self.clock.saving()
                || self.clipboard.has_pending()
                || self.calculator.has_work()
                || self.service_editor.has_work()
                || self.service_batch.busy()
                || self.delta_update.has_work()
                || self.updates.download_has_work()
                || self.portable_update.busy()
                || self.msi_update.busy()
                || self.images.screenshot_has_work()
            {
                self.workspace_exit_confirm = true;
                restore_main_window(self.window_handle, ctx);
            } else {
                self.quit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
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
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quit_requested
            && self.tray.is_none()
            && (self.data_state.has_work()
                || self.planner.has_unsaved()
                || self.prefix.has_work(&self.preferences.command_bindings)
                || self.clock.has_work()
                || self.clock.saving()
                || self.clipboard.has_pending()
                || self.calculator.has_work()
                || self.service_editor.has_work()
                || self.service_batch.busy()
                || self.delta_update.has_work()
                || self.updates.download_has_work()
                || self.portable_update.busy()
                || self.msi_update.busy()
                || self.images.screenshot_has_work())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.workspace_exit_confirm = true;
        }
        if self.workspace_exit_confirm {
            egui::Modal::new(egui::Id::new("workspace-exit-confirm")).show(ctx, |ui| {
                ui.set_max_width(620.0);
                ui.heading("退出前保留工作");
                ui.label("数据工作实例、流程步骤、备忘录和日程需要手动保存。流程请单独保存为文件，实例保存不包含步骤。已保存内容会保留，未保存修改会丢失。完全退出后日程不再弹出提醒。");
                ui.label("快捷键草稿和待确认导入也需要保存；配置读写进行中时请等待完成。");
                if self.clock.has_work() || self.clock.saving(){ui.label("时钟可主动开启本机保存并立即保存最新检查点。未保存的会话修改会清空；完全退出后不弹提醒。后台保存中需要等待。");}
                if self.calculator.has_work(){ui.label("计算工作表有未保存内容、待读取确认或后台任务；请返回另存/恢复基线或放弃本次读取。后台读写中须等待，完全退出会丢失未保存工作。");}
                if self.clipboard.has_pending(){ui.label("剪贴板历史尚未保存、规则草稿未应用或后台任务进行中，请返回处理。放弃未保存修改不会清除此前保存的旧历史。");}
                ui.horizontal_wrapped(|ui| {
                    if self.calculator.has_work() && ui.button("返回计算器工作表").clicked(){self.workspace_exit_confirm=false;self.page=Page::Calculator;}
                    if self.clipboard.has_pending() && ui.button("返回剪贴板保存").clicked(){self.workspace_exit_confirm=false;self.page=Page::Clipboard;}
                    if self.updates.download_has_work() && ui.button("返回更新下载").clicked() { self.workspace_exit_confirm=false;self.page=Page::Updates; }
                    if self.delta_update.has_work() && ui.button("返回更新包工作台").clicked() {self.workspace_exit_confirm=false;self.page=Page::DeltaUpdate;}
                    if self.images.screenshot_has_work() && ui.button("返回截图保存").clicked() {self.workspace_exit_confirm=false;self.page=Page::Images;self.images.show_screenshot();}
                    if (self.clock.has_work() || self.clock.saving()) && ui.button("返回时钟工作台").clicked(){self.workspace_exit_confirm=false;self.page=Page::Clock;}
                    if self.prefix.has_work(&self.preferences.command_bindings) && ui.button("返回快捷指令保存").clicked() { self.workspace_exit_confirm=false; self.page=Page::Commands; }
                    if self.data_state.has_work() && ui.button("返回数据工作台保存").clicked() { self.workspace_exit_confirm=false; self.page=Page::Data; }
                    if self.service_editor.has_work() && ui.button("返回本地服务保存").clicked() {self.workspace_exit_confirm=false;self.page=Page::Services;}
                    if self.service_batch.busy() {
                        ui.label("批量服务任务正在执行，请等待完成，或取消剩余操作后退出。");
                        if ui.button("返回本地服务任务").clicked() { self.workspace_exit_confirm=false; self.page=Page::Services; }
                    }
                    if self.planner.has_unsaved() && ui.button("返回备忘 / 日程保存").clicked() { self.workspace_exit_confirm=false; self.page=if self.planner.calendar { Page::Calendar } else { Page::Notes }; }
                    if ui.add_enabled(!self.data_state.has_active_tasks() && !self.planner.saving() && !self.prefix.files.busy() && !self.clock.saving() && !self.clipboard.saving() && !self.calculator.busy() && !self.service_editor.busy() && !self.service_batch.busy() && !self.images.screenshot_busy() && !self.delta_update.busy() && !self.updates.download_busy() && !self.portable_update.busy() && !self.msi_update.busy(), egui::Button::new("放弃未保存修改并退出")).clicked() {
                        self.workspace_exit_confirm=false;self.quit_requested=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                if self.data_state.has_active_tasks() || self.planner.saving() { ui.label("正在处理任务，请等待完成，或从后台任务取消可取消的操作后退出。"); }
                if self.updates.download_has_work() { ui.label("下载内容尚未保存或后台任务进行中；请返回保存、丢弃预览，或取消后台任务后退出。"); }
                if self.delta_update.has_work() { ui.label("更新包或重建结果未保存会丢失；后台任务请等待完成或返回取消后退出。"); }
                if self.images.screenshot_has_work() { ui.label("截图结果未保存会在退出后丢失；截图后台任务进行中需等待完成。"); }
            });
        }

        #[cfg(windows)]
        if (self.quick_open || self.images.screenshot_capture_active())
            && main_window_cloaked(self.window_handle)
        {
            return;
        }

        if (self.handoff.is_none() && !self.images.relay_active())
            && !self.workspace_exit_confirm
            && !self.data_state.modal_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K))
        {
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
        self.handoff_bar(ctx);
        if matches!(self.page, Page::Notes | Page::Calendar) && self.planner.editor_open() {
            let enabled = !self.launcher_open
                && (self.handoff.is_none() && !self.images.relay_active())
                && !self.workspace_exit_confirm;
            egui::TopBottomPanel::bottom("planner-editor-actions")
                .frame(
                    egui::Frame::new()
                        .fill(self.colors.panel)
                        .inner_margin(egui::Margin::symmetric(24, 10)),
                )
                .show(ctx, |ui| self.planner.editor_actions_ui(ui, enabled));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.colors.bg).inner_margin(24.0))
            .show(ctx, |ui| match self.page {
                Page::Tasks => {
                    self.tasks_page(ui);
                }
                Page::Notes | Page::Calendar => {
                    let conflict_review_open = self.planner.conflict_review_open();
                    let mut scroll = egui::ScrollArea::vertical().id_salt("planner-page");
                    if let Some(offset) = self.planner.take_page_navigation() {
                        scroll = scroll.vertical_scroll_offset(offset);
                    }
                    scroll.show(ui, |ui| self.planner.ui(ui));
                    let save_shortcut = !conflict_review_open
                        && !self.launcher_open
                        && (self.handoff.is_none() && !self.images.relay_active())
                        && !self.workspace_exit_confirm
                        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S));
                    self.planner.finish_editor_actions(save_shortcut);
                    let page = if self.planner.calendar {
                        Page::Calendar
                    } else {
                        Page::Notes
                    };
                    if self.page != page {
                        self.visit(if self.planner.calendar {
                            "calendar-planner"
                        } else {
                            "memos"
                        });
                    }
                    self.page = page;
                }
                Page::Intake => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.intake.ui(ui));
                }
                Page::Library => self.library_page(ui),
                Page::Home => {
                    egui::ScrollArea::vertical()
                        .id_salt("home-scroll")
                        .show(ui, |ui| self.start_page(ui));
                }
                Page::Data => {
                    self.data_state
                        .workflow_folder_settings(&mut self.preferences, &self.preferences_path);
                    egui::ScrollArea::vertical()
                        .id_salt("data-page")
                        .show(ui, |ui| self.data_state.ui(ui, ctx));
                    self.data_state
                        .workflow_folder_settings(&mut self.preferences, &self.preferences_path);
                    if let Some(path) = self.data_state.take_sqlite_open_request() {
                        match self.sqlite_browser.open_path(path) {
                            Ok(()) => {
                                self.page = Page::SqliteBrowser;
                                self.visit("sqlite");
                            }
                            Err(error) => self.data_state.message = error.to_string(),
                        }
                    }
                    if let Some(text) = self.data_state.take_text_flow_send() {
                        match handoff::Transfer::new("文本工具流程结果".into(), &text) {
                            Ok(transfer) => self.handoff = Some(transfer),
                            Err(error) => self.data_state.message = error.to_string(),
                        }
                    }
                    if let Some(data) = self.data_state.take_text_flow_table() {
                        match self.data_state.import_table(data, "流程表格结果") {
                            Ok(()) => self.visit("data"),
                            Err(error) => {
                                self.data_state.text_flow_transfer_failed(error.to_string())
                            }
                        }
                    }
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
                Page::DeltaUpdate => {
                    egui::ScrollArea::vertical()
                        .id_salt("delta-update-page")
                        .show(ui, |ui| self.delta_update.ui(ui, ctx));
                }
                Page::Updates => {
                    egui::ScrollArea::vertical()
                        .id_salt("updates-page")
                        .show(ui, |ui| self.updates_page(ui, ctx));
                }
                Page::Settings => {
                    egui::ScrollArea::vertical()
                        .id_salt("settings-scroll")
                        .show(ui, |ui| self.settings_page(ui, ctx));
                }
                Page::Plugins => {
                    let previous = self.plugins.selected.clone();
                    egui::ScrollArea::vertical()
                        .id_salt("plugins-page")
                        .show(ui, |ui| {
                            self.plugins.ui(
                                ui,
                                !self.launcher_open
                                    && (self.handoff.is_none() && !self.images.relay_active()),
                            )
                        });
                    if self.plugins.selected != previous
                        && let Some(id) = self.plugins.selected.clone()
                    {
                        self.visit(&id);
                    }
                }
                Page::Mcp => {
                    egui::ScrollArea::vertical()
                        .id_salt("mcp-page")
                        .show(ui, |ui| self.mcp.ui(ui));
                }
                Page::Agent => {
                    if ui.button("查看已导出的 Agent 记录 →").clicked() {
                        self.page = Page::AgentRecords;
                    }
                    let scroll = egui::ScrollArea::vertical().id_salt("agent-page");
                    #[cfg(feature = "ui-preview")]
                    let scroll = scroll.stick_to_bottom(self.agent.preview_scroll_bottom());
                    scroll.show(ui, |ui| self.agent.ui(ui));
                }
                Page::AgentRecords => {
                    if ui.button("← 返回本机 Agent 任务").clicked() {
                        self.page = Page::Agent;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("agent-records-page")
                        .show(ui, |ui| self.agent_records.ui(ui));
                }
                Page::Recorder => {
                    let previous_auto_minimize = self.recorder.auto_minimize();
                    let previous_auto_stop = self.recorder.auto_stop_minutes();
                    let previous_quality = self.recorder.quality();
                    let footer_height = if self.recorder.has_output_file() {
                        160.0
                    } else {
                        130.0
                    };
                    let height = (ui.available_height() - footer_height).max(120.0);
                    egui::ScrollArea::vertical()
                        .id_salt("recorder-page")
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.recorder.ui(ui, self.tray.is_some()));
                    ui.separator();
                    self.recorder.controls_ui(ui, self.tray.is_some());
                    if self.recorder.auto_minimize() != previous_auto_minimize
                        || self.recorder.auto_stop_minutes() != previous_auto_stop
                        || self.recorder.quality() != previous_quality
                    {
                        self.preferences.recorder_auto_minimize = self.recorder.auto_minimize();
                        self.preferences.recorder_auto_stop_minutes =
                            self.recorder.auto_stop_minutes();
                        self.preferences.recorder_quality = self.recorder.quality();
                        if let Err(error) = self.preferences.save(&self.preferences_path) {
                            self.toast = Some((error.to_string(), Instant::now()));
                        }
                    }
                }
                Page::Images => {
                    egui::ScrollArea::vertical()
                        .id_salt("image-tools-page")
                        .show(ui, |ui| self.images.ui(ui));
                }
                Page::Markdown => {
                    egui::ScrollArea::vertical()
                        .id_salt("markdown-page")
                        .show(ui, |ui| self.markdown.ui(ui));
                }
                Page::FileEncoding => {
                    egui::ScrollArea::vertical()
                        .id_salt("file-encoding-page")
                        .show(ui, |ui| self.file_encoding.ui(ui));
                }
                Page::ChecksumManifest => {
                    egui::ScrollArea::vertical()
                        .id_salt("checksum-manifest-page")
                        .show(ui, |ui| self.checksum_manifest.ui(ui));
                }
                Page::DiskInspector => {
                    egui::ScrollArea::vertical()
                        .id_salt("disk-inspector-page")
                        .show(ui, |ui| self.disk_inspector.ui(ui));
                }
                Page::DuplicateFinder => {
                    egui::ScrollArea::vertical()
                        .id_salt("duplicate-finder-page")
                        .show(ui, |ui| self.duplicate_finder.ui(ui));
                }
                Page::DirectoryCompare => {
                    egui::ScrollArea::vertical()
                        .id_salt("directory-compare-page")
                        .show(ui, |ui| self.directory_compare.ui(ui));
                }
                Page::SqliteBrowser => {
                    egui::ScrollArea::vertical()
                        .id_salt("sqlite-browser-page")
                        .show(ui, |ui| self.sqlite_browser.ui(ui));
                    if let Some((name, table)) = self.sqlite_browser.take_workbench_transfer() {
                        match self.data_state.import_table(table, &name) {
                            Ok(()) => {
                                self.page = Page::Data;
                                self.visit("data");
                            }
                            Err(error) => self
                                .sqlite_browser
                                .transfer_failed(format!("接力未完成：{error:#}")),
                        }
                    }
                }
                Page::AsciiCodes => {
                    egui::ScrollArea::vertical()
                        .id_salt("ascii-codes-page")
                        .show(ui, |ui| self.ascii_codes.ui(ui));
                }
                Page::Clock => {
                    egui::ScrollArea::vertical()
                        .id_salt("clock-workbench-page")
                        .show(ui, |ui| {
                            self.clock.ui(
                                ui,
                                catalog()
                                    .iter()
                                    .find(|e| e.id == "clock-workbench")
                                    .and_then(|e| e.version.as_deref())
                                    .unwrap_or("未声明"),
                            )
                        });
                }
                Page::Clipboard => {
                    egui::ScrollArea::vertical()
                        .id_salt("clipboard-workbench-page")
                        .show(ui, |ui| self.clipboard.ui(ui));
                }
                Page::Calculator => {
                    egui::ScrollArea::vertical()
                        .id_salt("calculator-page")
                        .show(ui, |ui| {
                            self.calculator.ui(
                                ui,
                                catalog()
                                    .iter()
                                    .find(|entry| entry.id == "advanced-calculator")
                                    .and_then(|entry| entry.version.as_deref())
                                    .unwrap_or("未声明"),
                            )
                        });
                    self.finish_date_transfer();
                }
                Page::Commands => {
                    egui::ScrollArea::vertical()
                        .id_salt("commands-page")
                        .show(ui, |ui| self.commands_page(ui, ctx));
                }
                Page::Symbols => {
                    egui::ScrollArea::vertical()
                        .id_salt("symbols-page")
                        .show(ui, |ui| self.symbols.ui(ui));
                }
                Page::AsciiArt => {
                    egui::ScrollArea::vertical()
                        .id_salt("ascii-art-page")
                        .show(ui, |ui| self.ascii_art.ui(ui));
                }
                Page::KnowledgeSources => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-sources-page")
                        .show(ui, |ui| self.knowledge_sources.ui(ui));
                }
                Page::DocumentIngestion => {
                    egui::ScrollArea::vertical()
                        .id_salt("document-ingestion-page")
                        .show(ui, |ui| {
                            self.document_ingestion
                                .ui(ui, self.knowledge_sources.sources())
                        });
                }
                Page::KnowledgeIndex => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-index-page")
                        .show(ui, |ui| {
                            self.knowledge_index.ui(
                                ui,
                                self.knowledge_sources.sources(),
                                self.knowledge_sources.is_locked(),
                            )
                        });
                }
                Page::VectorIndex => {
                    egui::ScrollArea::vertical()
                        .id_salt("vector-index-page")
                        .show(ui, |ui| {
                            self.vector_index.ui(
                                ui,
                                self.knowledge_sources.sources(),
                                self.knowledge_sources.is_locked(),
                            )
                        });
                }
                Page::KnowledgeSearch => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-search-page")
                        .show(ui, |ui| {
                            self.knowledge_search
                                .ui(ui, self.knowledge_sources.sources())
                        });
                }
                Page::HybridSearch => {
                    egui::ScrollArea::vertical()
                        .id_salt("hybrid-search-page")
                        .show(ui, |ui| {
                            self.hybrid_search.ui(
                                ui,
                                self.knowledge_sources.sources(),
                                self.knowledge_sources.is_locked(),
                            )
                        });
                }
                Page::KnowledgeAnswer => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-answer-page")
                        .show(ui, |ui| {
                            self.knowledge_answer
                                .ui(ui, self.knowledge_sources.sources())
                        });
                }
                Page::Embedding => {
                    egui::ScrollArea::vertical()
                        .id_salt("embedding-page")
                        .show(ui, |ui| self.embedding.ui(ui));
                }
                Page::KnowledgeEval => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-eval-page")
                        .show(ui, |ui| {
                            self.knowledge_eval.ui(ui, self.knowledge_sources.sources())
                        });
                }
                Page::KnowledgeCapture => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-capture-page")
                        .show(ui, |ui| {
                            self.knowledge_capture.ui(ui, &mut self.knowledge_sources)
                        });
                }
                Page::KnowledgeMcp => {
                    egui::ScrollArea::vertical()
                        .id_salt("knowledge-mcp-page")
                        .show(ui, crate::knowledge_mcp::ui);
                }
                Page::Java | Page::Django => {
                    let category = if self.page == Page::Java {
                        "Java 与 JVM"
                    } else {
                        "Python 与 Django"
                    };
                    if self.frameworks.selected.category() != category {
                        self.frameworks.select(if self.page == Page::Java {
                            crate::framework::Tool::JavaEnvironment
                        } else {
                            crate::framework::Tool::PythonEnvironment
                        });
                    }
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
                        .show(ui, |ui| {
                            self.frameworks.ui(
                                ui,
                                !self.launcher_open
                                    && (self.handoff.is_none() && !self.images.relay_active()),
                                category,
                            )
                        });
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
        if let Some(id) = self.images.take_relay_route() {
            self.visit(id);
        }
        self.handoff_dialog(ctx);
        self.overlays(ctx);
        self.launcher(ctx);
        self.clock.notice_ui(ctx);
        if self.planner.reminder_ui(ctx) {
            self.navigate(Page::Calendar, None);
        }
        self.recorder.selection_overlay(ctx);
        if self.recorder.take_restore_request() {
            restore_main_window(self.window_handle, ctx);
        }
        if self.recorder.take_minimize_request() {
            self.minimize_for_recording(ctx);
        }
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
                            self.tool_state.clear_result();
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
    if let Ok(bytes) = fs::read(r"C:\Windows\Fonts\seguiemj.ttf") {
        fonts.font_data.insert(
            "windows-emoji".to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("windows-emoji".to_owned());
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
    visuals.warn_fg_color = p.amber;
    visuals.error_fg_color = p.red;
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
        assert!(ServiceFilter::Managed.matches(ServiceState::Running, true, false));
        assert!(!ServiceFilter::Managed.matches(ServiceState::External, false, false));
        assert!(ServiceFilter::External.matches(ServiceState::PortOpen, false, false));
        assert!(!ServiceFilter::External.matches(ServiceState::Stopped, false, false));
        assert!(ServiceFilter::Stopped.matches(ServiceState::Stopped, false, false));
        for state in [
            ServiceState::Running,
            ServiceState::External,
            ServiceState::PortOpen,
            ServiceState::Stopped,
        ] {
            assert!(ServiceFilter::Attention.matches(state, false, true));
            assert!(!ServiceFilter::Attention.matches(state, false, false));
        }
    }
}

#[cfg(all(test, windows))]
mod native_window_tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, IsIconic, IsWindowVisible,
        IsZoomed, SW_MAXIMIZE, SW_MINIMIZE, ShowWindow, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
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
                WS_OVERLAPPEDWINDOW,
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
        let original_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        assert!(hide_main_window(Some(window as isize)));
        assert!(main_window_cloaked(Some(window as isize)));
        assert_ne!(
            unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } & WS_EX_TOOLWINDOW as isize,
            0
        );
        unsafe {
            ShowWindow(window, SW_MAXIMIZE);
        }
        assert_ne!(unsafe { IsZoomed(window) }, 0);
        assert!(hide_main_window(Some(window as isize)));
        restore_main_window(Some(window as isize), &egui::Context::default());
        assert_eq!(
            unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) },
            original_style
        );
        assert_ne!(
            unsafe { IsZoomed(window) },
            0,
            "Opening a tool must preserve maximization"
        );
        unsafe {
            ShowWindow(window, SW_MINIMIZE);
        }
        assert_ne!(unsafe { IsIconic(window) }, 0);
        restore_main_window(Some(window as isize), &egui::Context::default());
        assert_eq!(unsafe { IsIconic(window) }, 0);
        assert_ne!(
            unsafe { IsZoomed(window) },
            0,
            "Restoring minimized maximized window keeps its placement"
        );
        unsafe { DestroyWindow(window) };
    }
}
