//! Renders synthetic, credential-free fixtures through the real eframe renderer.
use eframe::egui;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
use zi_devtools::recorder::{self, AudioGains, AudioMode, Event, Region, Session};

const NAMES: [&str; 436] = [
    "home-dark",
    "home-light",
    "yaml-dark",
    "yaml-light-compact",
    "data-dark",
    "data-light",
    "files-dark",
    "files-light-compact",
    "launcher-light",
    "json-path-light",
    "json-path-dark",
    "json-diff-light",
    "json-diff-dark",
    "quality-light",
    "quality-dark",
    "cron-light",
    "cron-dark",
    "random-light",
    "random-dark",
    "unicode-light",
    "unicode-dark",
    "plugins-light",
    "plugins-dark",
    "category-light",
    "recent-dark",
    "plugin-tool-light",
    "plugin-search-dark",
    "java-trace-light",
    "java-trace-dark",
    "django-trace-light",
    "django-trace-dark",
    "directory-page-two-light",
    "java-environment-dark",
    "java-environment-light",
    "django-environment-dark",
    "django-environment-light",
    "java-threads-dark",
    "java-threads-light",
    "dependencies-dark",
    "dependencies-light",
    "migrations-dark",
    "migrations-light",
    "sql-dark",
    "sql-light",
    "gc-dark",
    "gc-light",
    "jfr-dark",
    "jfr-light",
    "spring-config-dark",
    "spring-config-light",
    "actuator-dark",
    "actuator-light",
    "django-urls-dark",
    "django-urls-light",
    "drf-dark",
    "drf-light",
    "checks-dark",
    "checks-light",
    "celery-dark",
    "celery-light",
    "tray-settings-dark",
    "tray-favorites-light",
    "tray-recent-dark",
    "tray-frequent-light",
    "file-intake-dark",
    "file-intake-light",
    "handoff-dark",
    "handoff-light",
    "transform-dark",
    "transform-light",
    "types-dark",
    "types-light",
    "columns-dark",
    "columns-light",
    "join-dark",
    "join-light",
    "tasks-dark",
    "tasks-light",
    "credentials-dark",
    "credentials-light",
    "connection-profiles-dark",
    "connection-profiles-light",
    "model-discovery-dark",
    "model-discovery-light",
    "conversation-dark",
    "conversation-light",
    "stream-dark",
    "stream-light",
    "import-conversation-dark",
    "import-conversation-light",
    "attachments-dark",
    "attachments-light",
    "library-dark",
    "library-light",
    "prompts-dark",
    "prompts-light",
    "prompt-editor-dark",
    "prompt-editor-light",
    "mcp-dark",
    "mcp-light",
    "recorder-dark",
    "recorder-light",
    "recorder-interrupted-dark",
    "recorder-interrupted-light",
    "images-dark",
    "images-light",
    "image-batch-dark",
    "image-batch-light",
    "image-metadata-dark",
    "image-metadata-light",
    "image-editor-dark",
    "image-editor-light",
    "markdown-dark",
    "markdown-light",
    "file-encoding-dark",
    "file-encoding-light",
    "checksum-manifest-dark",
    "checksum-manifest-light",
    "knowledge-sources-dark",
    "knowledge-sources-light",
    "document-ingestion-dark",
    "document-ingestion-light",
    "knowledge-index-dark",
    "knowledge-index-light",
    "knowledge-search-dark",
    "knowledge-search-light",
    "knowledge-answer-dark",
    "knowledge-answer-light",
    "knowledge-eval-dark",
    "knowledge-eval-light",
    "knowledge-capture-dark",
    "knowledge-capture-light",
    "knowledge-mcp-dark",
    "knowledge-mcp-light",
    "mcp-export-dark",
    "mcp-export-light",
    "mcp-review-dark",
    "mcp-review-light",
    "embedding-dark",
    "embedding-light",
    "vector-index-dark",
    "vector-index-light",
    "hybrid-search-dark",
    "hybrid-search-light",
    "hybrid-answer-dark",
    "hybrid-answer-light",
    "rag-compare-dark",
    "rag-compare-light",
    "mcp-permissions-dark",
    "mcp-permissions-light",
    "agent-plan-dark",
    "agent-result-light",
    "agent-record-dark",
    "agent-viewer-light",
    "agent-viewer-dark",
    "disk-inspector-light",
    "disk-inspector-dark",
    "duplicate-finder-light",
    "duplicate-finder-dark",
    "ascii-codes-light",
    "ascii-codes-dark",
    "symbols-light",
    "symbols-dark",
    "ascii-art-light",
    "ascii-art-dark",
    "directory-compare-light",
    "directory-compare-dark",
    "sqlite-browser-light",
    "sqlite-browser-dark",
    "mcp-connected-light",
    "mcp-http-dark",
    "mcp-http-light",
    "mcp-http-auth-dark",
    "mcp-http-auth-light",
    "mcp-oauth-metadata-dark",
    "mcp-oauth-metadata-light",
    "mcp-oauth-refresh-dark",
    "mcp-oauth-refresh-light",
    "mcp-oauth-auto-dark",
    "mcp-oauth-auto-light",
    "mcp-oauth-register-dark",
    "mcp-oauth-register-light",
    "mcp-oauth-revoke-dark",
    "mcp-oauth-revoke-light",
    "tool-library-dark",
    "tool-library-light",
    "task-search-dark",
    "task-search-light",
    "tool-library-compact-dark",
    "tool-library-compact-light",
    "workspace-data-dark",
    "workspace-data-light",
    "workspace-library-dark",
    "workspace-library-light",
    "workspace-save-dark",
    "workspace-save-light",
    "workspace-tasks-dark",
    "workspace-tasks-light",
    "memos-dark",
    "memos-light",
    "calendar-dark",
    "calendar-light",
    "reminder-dark",
    "reminder-light",
    "calendar-compact-dark",
    "calendar-compact-light",
    "calendar-editor-compact-dark",
    "calendar-editor-compact-light",
    "memo-handoff-dark",
    "memo-handoff-light",
    "memo-received-dark",
    "memo-received-light",
    "planner-trash-dark",
    "planner-trash-light",
    "planner-purge-dark",
    "planner-purge-light",
    "planner-import-dark",
    "planner-import-light",
    "planner-export-dark",
    "planner-export-light",
    "planner-backup-dark",
    "planner-backup-light",
    "planner-restore-merge-dark",
    "planner-restore-merge-light",
    "planner-restore-replace-dark",
    "planner-restore-replace-light",
    "planner-agenda-dark",
    "planner-agenda-light",
    "planner-agenda-compact-dark",
    "planner-agenda-compact-light",
    "sidebar-minimum-dark",
    "sidebar-minimum-light",
    "sidebar-long-title-dark",
    "sidebar-long-title-light",
    "planner-monthly-dark",
    "planner-monthly-light",
    "planner-yearly-dark",
    "planner-yearly-light",
    "planner-interval-dark",
    "planner-interval-light",
    "planner-allday-dark",
    "planner-allday-light",
    "planner-interval-agenda-dark",
    "planner-interval-agenda-light",
    "planner-interval-compact-dark",
    "planner-interval-compact-light",
    "planner-ics-export-dark",
    "planner-ics-export-light",
    "planner-ics-import-dark",
    "planner-ics-import-light",
    "planner-ics-compact-dark",
    "planner-ics-compact-light",
    "planner-cutoff-dark",
    "planner-cutoff-light",
    "planner-cutoff-compact-dark",
    "planner-cutoff-compact-light",
    "planner-actions-memo-dark",
    "planner-actions-memo-light",
    "planner-actions-calendar-dark",
    "planner-actions-calendar-light",
    "planner-actions-memo-min-dark",
    "planner-actions-memo-min-light",
    "planner-actions-calendar-min-dark",
    "planner-actions-calendar-min-light",
    "planner-actions-error-min-dark",
    "planner-actions-error-min-light",
    "planner-snooze-dark",
    "planner-snooze-light",
    "planner-snooze-editing-dark",
    "planner-snooze-editing-light",
    "planner-list-many-dark",
    "planner-list-many-light",
    "planner-list-pinned-dark",
    "planner-list-pinned-light",
    "planner-copy-memo-dark",
    "planner-copy-memo-light",
    "planner-copy-calendar-dark",
    "planner-copy-calendar-light",
    "planner-event-to-memo-dark",
    "planner-event-to-memo-light",
    "planner-memo-to-event-dark",
    "planner-memo-to-event-light",
    "planner-week-dark",
    "planner-week-light",
    "planner-week-small-dark",
    "planner-week-small-light",
    "event-handoff-dark",
    "event-handoff-light",
    "handoff-discovery-dark",
    "handoff-discovery-light",
    "sqlite-export-review-dark",
    "sqlite-export-review-light",
    "sqlite-export-review-small-dark",
    "sqlite-export-review-small-light",
    "sqlite-export-discovery-dark",
    "sqlite-export-discovery-light",
    "planner-discovery-dark",
    "planner-discovery-light",
    "workflow-preview-dark",
    "workflow-preview-light",
    "workflow-import-dark",
    "workflow-import-light",
    "workflow-import-small-dark",
    "workflow-import-small-light",
    "workflow-empty-dark",
    "workflow-empty-light",
    "workflow-empty-import-dark",
    "workflow-empty-import-light",
    "workflow-empty-small-dark",
    "workflow-empty-small-light",
    "calculator-dark",
    "calculator-light",
    "calculator-small-dark",
    "calculator-small-light",
    "commands-dark",
    "commands-light",
    "binding-editor-dark",
    "binding-editor-light",
    "binding-editor-small-dark",
    "binding-editor-small-light",
    "profile-import-dark",
    "profile-import-light",
    "profile-import-small-dark",
    "profile-import-small-light",
    "profile-conflict-dark",
    "profile-conflict-light",
    "clock-world-dark",
    "clock-world-light",
    "clock-stopwatch-dark",
    "clock-stopwatch-light",
    "clock-timers-dark",
    "clock-timers-light",
    "clock-alarms-dark",
    "clock-alarms-light",
    "clock-focus-dark",
    "clock-focus-light",
    "clock-world-small-dark",
    "clock-world-small-light",
    "clock-timers-small-dark",
    "clock-timers-small-light",
    "clock-alarms-small-dark",
    "clock-alarms-small-light",
    "clock-audio-dark",
    "clock-audio-light",
    "clock-audio-quiet-small-dark",
    "clock-audio-quiet-small-light",
    "clock-audio-failure-dark",
    "clock-audio-failure-light",
    "screenshot-lasso-dark",
    "screenshot-lasso-light",
    "recorder-tutorial-dark",
    "recorder-tutorial-light",
    "recorder-tutorial-preview-dark",
    "recorder-tutorial-preview-light",
    "updates-dark",
    "updates-light",
    "updates-offline-dark",
    "updates-offline-light",
    "delta-package-dark",
    "delta-package-light",
    "delta-rebuild-dark",
    "delta-rebuild-light",
    "update-integrity-dark",
    "update-integrity-light",
    "update-integrity-failure-dark",
    "update-integrity-failure-light",
    "update-download-dark",
    "update-download-light",
    "update-download-failure-dark",
    "update-download-failure-light",
    "update-signed-dark",
    "update-signed-light",
    "update-signature-failure-dark",
    "update-signature-failure-light",
    "update-signed-report-dark",
    "update-signed-report-light",
    "portable-update-ready-dark",
    "portable-update-ready-light",
    "portable-update-restored-dark",
    "portable-update-restored-light",
    "msi-update-ready-dark",
    "msi-update-ready-light",
    "msi-update-cancelled-dark",
    "msi-update-cancelled-light",
    "delta-update-ready-dark",
    "delta-update-ready-light",
    "delta-update-complete-dark",
    "delta-update-complete-light",
    "tutorial-spotlight-dark",
    "tutorial-spotlight-light",
    "clipboard-history-dark",
    "clipboard-history-light",
    "clipboard-storage-dark",
    "clipboard-storage-light",
    "clipboard-retention-dark",
    "clipboard-retention-light",
    "clipboard-policy-dark",
    "clipboard-policy-light",
    "image-relay-dark",
    "image-relay-light",
    "clipboard-image-dark",
    "clipboard-image-light",
    "calculator-matrix-dark",
    "calculator-matrix-light",
    "calculator-matrix-small-dark",
    "calculator-matrix-small-light",
    "calculator-sheet-dark",
    "calculator-sheet-light",
    "calculator-sheet-small-dark",
    "calculator-sheet-small-light",
    "numeric-handoff-dark",
    "numeric-handoff-light",
    "numeric-handoff-small-dark",
    "numeric-handoff-small-light",
    "numeric-received-dark",
    "numeric-received-light",
    "numeric-received-small-dark",
    "numeric-received-small-light",
    "table-mapping-dark",
    "table-mapping-light",
    "table-mapping-small-dark",
    "table-mapping-small-light",
    "matrix-target-dark",
    "matrix-target-light",
    "matrix-target-small-dark",
    "matrix-target-small-light",
    "numeric-entry-small-dark",
    "numeric-entry-small-light",
    "numeric-picker-eight-dark",
    "numeric-picker-eight-light",
    "numeric-boundary-target-dark",
    "numeric-boundary-target-light",
    "calculator-plot-dark",
    "calculator-plot-light",
    "calculator-plot-small-dark",
    "calculator-plot-small-light",
];

struct Capture {
    clock_window_handle: isize,
    app: DevToolsApp,
    folder: PathBuf,
    fixture: PathBuf,
    scene: usize,
    frames: usize,
    pending: bool,
    started: Instant,
    recorder_smoke: Option<Session>,
    recorder_smoke_started: Option<Instant>,
    recorder_smoke_size: Option<(u32, u32)>,
    auto_minimize_smoke_at: Option<Instant>,
    auto_minimize_recording_at: Option<Instant>,
    auto_minimize_smoke_stop_requested: bool,
    quick_smoke_phase: u8,
    sqlite_input_frame: Option<usize>,
    workflow_input_frame: Option<usize>,
}
impl Capture {
    fn pointer_click_waiting(&self) -> bool {
        let seconds = match self.frames {
            20 => 1,
            40 => 3,
            60 => 5,
            90 => 8,
            110 => 10,
            _ => 0,
        };
        self.started.elapsed() < Duration::from_secs(seconds)
    }
}
impl eframe::App for Capture {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if std::env::args().nth(3).as_deref() == Some("screenshot-overlay-smoke") {
            input.focused = true;
            if let Some(viewport) = input.viewports.get_mut(&input.viewport_id) {
                viewport.focused = Some(true);
            }
            if let Some((key, pressed)) = match self.frames {
                5 => Some((egui::Key::S, true)),
                6 => Some((egui::Key::S, false)),
                8 => Some((egui::Key::C, true)),
                9 => Some((egui::Key::C, false)),
                _ => None,
            } {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        input.events.push(egui::Event::PointerGone);
        if std::env::args().nth(3).as_deref() == Some("clock-audio-smoke") {
            let index = match self.frames {
                20 | 21 => Some(0),
                30 | 31 => Some(1),
                50 | 51 | 70 | 71 => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self.app.preview_clock_audio_position(index);
                assert!(pos.is_finite());
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames % 10 == 0,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if matches!(
            std::env::args().nth(3).as_deref(),
            Some("clock-save-smoke" | "clock-restore-smoke")
        ) && self.quick_smoke_phase > 0
            && matches!(self.frames, 40 | 41)
        {
            let saving = std::env::args().nth(3).as_deref() == Some("clock-save-smoke");
            let pos = self
                .app
                .preview_clock_storage_position(if saving { 0 } else { 1 });
            assert!(pos.is_finite());
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frames == 40,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("clock-smoke") {
            let index = match self.frames {
                20 | 21 | 40 | 41 => Some(0),
                30 | 31 => Some(1),
                60 | 61 => Some(2),
                70 | 71 => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self.app.preview_clock_position(index);
                assert!(pos.is_finite());
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames % 10 == 0,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("profile-smoke") {
            input.focused = true;
            if let Some(v) = input.viewports.get_mut(&input.viewport_id) {
                v.focused = Some(true);
            }
            let pos = match self.frames {
                20 | 21 => Some(self.app.preview_profile_position(1)),
                40 | 41 => Some(self.app.preview_profile_position(2)),
                50 | 51 => Some(self.app.preview_profile_position(0)),
                70 | 71 => Some(self.app.preview_binding_position(3)),
                _ => None,
            };
            if let Some(pos) = pos {
                assert!(pos.is_finite());
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames % 10 == 0,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("bindings-smoke") {
            input.focused = true;
            if let Some(v) = input.viewports.get_mut(&input.viewport_id) {
                v.focused = Some(true);
            }
            let click = match self.frames {
                20 | 21 => Some(0),
                30 | 31 => Some(1),
                60 | 61 | 90 | 91 => Some(2),
                70 | 71 | 130 | 131 => Some(3),
                100 | 101 => Some(4),
                110 | 111 => Some(5),
                _ => None,
            };
            if let Some(index) = click {
                let pos = self.app.preview_binding_position(index);
                assert!(pos.is_finite());
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames % 10 == 0,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let key = match self.frames {
                80 | 81 => Some(egui::Key::Q),
                85 | 86 => Some(egui::Key::A),
                _ => None,
            };
            if let Some(key) = key {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: matches!(self.frames, 80 | 85),
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 40 {
                let modifiers = egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                };
                input.modifiers = modifiers;
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
            }
            if self.frames == 41 {
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 50 {
                input.events.push(egui::Event::Text("Q A".into()));
            }
        }
        if std::env::args().nth(3).as_deref() == Some("prefix-smoke") {
            // This native fixture supplies controlled focus states; it does not
            // prove Windows foreground acquisition or a physical global hotkey.
            input.focused = self.frames != 65;
            if let Some(viewport) = input.viewports.get_mut(&input.viewport_id) {
                viewport.focused = Some(input.focused);
            }
            if self.frames == 82 {
                input.events.push(egui::Event::Ime(egui::ImeEvent::Enabled));
            }
            if self.frames == 84 {
                input
                    .events
                    .push(egui::Event::Ime(egui::ImeEvent::Disabled));
            }
            if self.frames == 20 {
                println!(
                    "prefix native focus: raw={}, viewport={:?}",
                    input.focused,
                    input
                        .viewports
                        .get(&input.viewport_id)
                        .and_then(|v| v.focused)
                );
            }
            let key = match self.frames {
                20 => Some(egui::Key::R),
                30 => Some(egui::Key::O),
                50 => Some(egui::Key::C),
                62 => Some(egui::Key::T),
                65 | 82 => Some(egui::Key::C),
                70 => Some(egui::Key::Z),
                86 => Some(egui::Key::Escape),
                90 => Some(egui::Key::K),
                _ => None,
            };
            if let Some(key) = key {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let released = match self.frames {
                21 => Some(egui::Key::R),
                31 => Some(egui::Key::O),
                51 => Some(egui::Key::C),
                63 => Some(egui::Key::T),
                66 | 83 => Some(egui::Key::C),
                71 => Some(egui::Key::Z),
                87 => Some(egui::Key::Escape),
                91 => Some(egui::Key::K),
                _ => None,
            };
            if let Some(key) = released {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }

        if std::env::args().nth(3).as_deref() == Some("boundary-smoke") {
            if matches!(self.frames, 20 | 25) {
                input.events.push(egui::Event::PointerMoved(
                    self.app.preview_boundary_position(0),
                ));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -900.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let action = match self.frames {
                10 | 11 => Some((false, 4)),
                30 | 31 => Some((true, 1)),
                40 | 41 => Some((true, 2)),
                50 | 51 => Some((false, 0)),
                65 | 66 => Some((false, 1)),
                75 | 76 | 95 | 96 => Some((false, 3)),
                85 | 86 => Some((false, 2)),
                _ => None,
            };
            if let Some((boundary, index)) = action {
                let pos = if boundary {
                    self.app.preview_boundary_position(index)
                } else {
                    self.app.preview_mapping_position(index)
                };
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 10 | 30 | 40 | 50 | 65 | 75 | 85 | 95),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("entry-smoke") {
            let index = match self.frames {
                20 | 21 | 40 | 41 => Some(4),
                30 | 31 => Some(5),
                50 | 51 => Some(0),
                65 | 66 => Some(1),
                75 | 76 | 95 | 96 => Some(3),
                85 | 86 => Some(2),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self.app.preview_mapping_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 20 | 30 | 40 | 50 | 65 | 75 | 85 | 95),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let mapping_mode = std::env::args().nth(3);
        let mapping_small = mapping_mode.as_deref() == Some("mapping-small-smoke");
        if mapping_mode.as_deref() == Some("mapping-smoke") && self.frames == 8 {
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(760.0, 620.0)));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -240.0),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if mapping_small && matches!(self.frames, 8 | 11 | 14 | 17) {
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(760.0, 620.0)));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -560.0),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if mapping_small || mapping_mode.as_deref() == Some("mapping-smoke") {
            let index = match self.frames {
                20 | 21 => Some(0),
                35 | 36 => Some(1),
                45 | 46 | 65 | 66 => Some(3),
                55 | 56 => Some(2),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self.app.preview_mapping_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 20 | 35 | 45 | 55 | 65),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("numeric-smoke") {
            let index = match self.frames {
                20 | 21 => Some(4),
                43 | 44 => Some(1),
                53 | 54 => Some(2),
                63 | 64 => Some(0),
                73 | 74 | 120 | 121 => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self.app.preview_numeric_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 20 | 43 | 53 | 63 | 73 | 120),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("plot-smoke") {
            let click = match self.frames {
                10 | 11 | 90 | 91 => Some(1),
                70 | 71 => Some(0),
                110 | 111 => Some(3),
                125 | 126 => Some(4),
                _ => None,
            };
            if let Some(index) = click {
                let pos = self.app.preview_plot_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 10 | 70 | 90 | 110 | 125),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 40 {
                let pos = self.app.preview_plot_position(2);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 140.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if matches!(self.frames, 55..=57) {
                let start = self.app.preview_plot_position(2);
                let pos = if self.frames == 55 {
                    start
                } else {
                    start + egui::vec2(30.0, 18.0)
                };
                input.events.push(egui::Event::PointerMoved(pos));
                if self.frames != 56 {
                    input.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: self.frames == 55,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            if self.frames == 73 {
                let modifiers = egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                };
                input.modifiers = modifiers;
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
            }
            if self.frames == 74 {
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 76 {
                input.events.push(egui::Event::Text("sin(x)*3".into()));
            }
        }
        if std::env::args().nth(3).as_deref() == Some("worksheet-smoke")
            && matches!(self.frames, 70 | 71 | 80 | 81)
        {
            let pos = self.app.preview_sheet_position(self.frames >= 80);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 70 | 80),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("matrix-smoke")
            && matches!(self.frames, 20 | 21)
        {
            let pos = self.app.preview_matrix_position();
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frames == 20,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("calculator-smoke") {
            let released = match self.frames {
                31 | 71 => Some(egui::Key::A),
                51 | 91 => Some(egui::Key::Enter),
                _ => None,
            };
            if let Some(key) = released {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if matches!(self.frames, 20 | 21) {
                let pos = self.app.preview_calculator_position();
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames == 20,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if matches!(self.frames, 30 | 70) {
                let modifiers = egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                };
                input.modifiers = modifiers;
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
            }
            if matches!(self.frames, 40 | 80) {
                input.events.push(egui::Event::Text(
                    if self.frames == 40 { "0.1+0.2" } else { "1/0" }.into(),
                ));
            }
            if matches!(self.frames, 50 | 90) {
                input.events.push(egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if matches!(
            std::env::args().nth(3).as_deref(),
            Some("workflow-smoke" | "workflow-light-smoke" | "workflow-small-smoke")
        ) {
            let click = match self.frames {
                20 | 21 => Some((0, self.frames == 20)),
                40 | 41 => Some((1, self.frames == 40)),
                60 | 61 => Some((2, self.frames == 60)),
                90 | 91 => Some((3, self.frames == 90)),
                110 | 111 => Some((4, self.frames == 110)),
                _ => None,
            };
            if let Some((index, pressed)) = click.filter(|_| !self.pointer_click_waiting()) {
                self.workflow_input_frame = Some(self.frames);
                let pos = self.app.preview_workflow_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("sqlite-export-smoke") {
            let click = match self.frames {
                20 => Some((1, true)),
                21 => Some((1, false)),
                40 => Some((0, true)),
                41 => Some((0, false)),
                60 => Some((2, true)),
                61 => Some((2, false)),
                90 => Some((3, true)),
                91 => Some((3, false)),
                _ => None,
            };
            if let Some((index, pressed)) = click.filter(|_| !self.pointer_click_waiting()) {
                self.sqlite_input_frame = Some(self.frames);
                let pos = self.app.preview_sqlite_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("snooze-smoke")
            && matches!(self.frames, 25 | 26 | 35 | 36 | 45 | 46)
        {
            let index = if self.frames >= 45 {
                2
            } else if self.frames >= 35 {
                1
            } else {
                0
            };
            let pos = self.app.preview_snooze_position(index);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 25 | 35 | 45),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("listing-smoke")
            && matches!(self.frames, 25 | 26 | 35 | 36 | 45 | 46 | 55 | 56 | 75 | 76)
        {
            let index = if self.frames >= 75 {
                3
            } else if self.frames >= 55 || self.frames < 35 {
                0
            } else if self.frames >= 45 {
                2
            } else {
                1
            };
            let pos = self.app.preview_listing_position(index);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 25 | 35 | 45 | 55 | 75),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("listing-smoke")
            && (95..=110).contains(&self.frames)
        {
            input.events.push(egui::Event::PointerMoved(
                self.app.preview_listing_position(3),
            ));
        }
        if std::env::args().nth(3).as_deref() == Some("listing-smoke") && self.frames == 95 {
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -2000.0),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(
            std::env::args().nth(3).as_deref(),
            Some("convert-event-smoke" | "convert-memo-smoke")
        ) {
            if matches!(self.frames, 25 | 26) {
                let pos = self.app.preview_convert_position();
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames == 25,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 45 {
                input.modifiers = egui::Modifiers::CTRL;
                input.events.push(egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("duplicate-smoke") {
            if matches!(self.frames, 25 | 26) {
                let pos = self.app.preview_duplicate_position();
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames == 25,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 45 {
                input.modifiers = egui::Modifiers::CTRL;
                input.events.push(egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                });
            }
        }
        if matches!(
            std::env::args().nth(3).as_deref(),
            Some("week-smoke" | "week-small-smoke")
        ) && matches!(self.frames, 15 | 16 | 20 | 21 | 25 | 26)
        {
            let index = if self.frames < 20 {
                1
            } else if self.frames < 25 {
                0
            } else {
                2
            };
            let pos = self.app.preview_week_position(index);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 15 | 20 | 25),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(
            std::env::args().nth(3).as_deref(),
            Some("calendar-navigation-smoke" | "week-navigation-smoke")
        ) && matches!(self.frames, 25 | 26 | 55 | 56)
        {
            let pos = self
                .app
                .preview_planner_navigation_position(usize::from(self.frames >= 55));
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 25 | 55),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("handoff-discovery-smoke")
            && matches!(self.frames, 20 | 21 | 35 | 36)
        {
            let pos = self
                .app
                .preview_handoff_discovery_position(usize::from(self.frames >= 35));
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 20 | 35),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("event-handoff-smoke")
            && matches!(self.frames, 20 | 21 | 35 | 36)
        {
            let pos = self
                .app
                .preview_event_handoff_position(usize::from(self.frames >= 35));
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 20 | 35),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("week-scroll-smoke") {
            if (25..45).contains(&self.frames) || (50..70).contains(&self.frames) {
                let index = usize::from(self.frames >= 50);
                input.events.push(egui::Event::PointerMoved(
                    self.app.preview_week_scroll_position(index),
                ));
            }
            if matches!(self.frames, 25 | 50) {
                let horizontal = self.frames == 50;
                let pos = self
                    .app
                    .preview_week_scroll_position(usize::from(horizontal));
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: if horizontal {
                        egui::vec2(-2000.0, 0.0)
                    } else {
                        egui::vec2(0.0, -2000.0)
                    },
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if matches!(self.frames, 75 | 76) {
                let pos = self.app.preview_week_scroll_position(2);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frames == 75,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let actions_mode = std::env::args().nth(3);
        if matches!(
            actions_mode.as_deref(),
            Some("actions-smoke" | "actions-min-smoke")
        ) {
            if matches!(self.frames, 25 | 26 | 55 | 56) {
                let pos = self
                    .app
                    .preview_actions_position(if self.frames >= 55 { 1 } else { 2 });
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 25 | 55),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 30 {
                input.events.push(egui::Event::Text("新增".into()));
                input.modifiers = egui::Modifiers::CTRL;
                input.events.push(egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                });
            }
            if self.frames == 45 {
                input
                    .events
                    .push(egui::Event::PointerMoved(egui::pos2(320.0, 180.0)));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -2000.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let cutoff_mode = std::env::args().nth(3);
        if matches!(
            cutoff_mode.as_deref(),
            Some("cutoff-smoke" | "cutoff-compact-smoke")
        ) {
            if matches!(self.frames, 25 | 26 | 35 | 36 | 55 | 56) {
                let index = if self.frames >= 55 {
                    2
                } else if self.frames >= 35 {
                    1
                } else {
                    0
                };
                let pos = self.app.preview_cutoff_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 25 | 35 | 55),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if self.frames == 38 {
                let modifiers = egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                };
                input.modifiers = modifiers;
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
            }
            if self.frames == 39 {
                input.events.push(egui::Event::Text("2026-10-09".into()));
            }
            if self.frames == 45 {
                input
                    .events
                    .push(egui::Event::PointerMoved(egui::pos2(850.0, 390.0)));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -800.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("ics-smoke") {
            let index = match self.frames {
                25 | 26 => Some(0),
                45 | 46 => Some(1),
                55 | 56 => Some(2),
                65 | 66 => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self.app.preview_ics_position(index);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 25 | 45 | 55 | 65),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let interval_mode = std::env::args().nth(3);
        let compact_interval = interval_mode.as_deref() == Some("interval-compact-smoke");
        let interval_smoke = compact_interval || interval_mode.as_deref() == Some("interval-smoke");
        if interval_smoke && !compact_interval && self.frames == 31 {
            input
                .events
                .push(egui::Event::PointerMoved(if compact_interval {
                    egui::pos2(850.0, 180.0)
                } else {
                    egui::pos2(1100.0, 390.0)
                }));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -430.0),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if interval_smoke && matches!(self.frames, 25 | 26 | 35 | 36) {
            println!(
                "interval click: frame={}, compact={compact_interval}",
                self.frames
            );
            let pos = self.app.preview_interval_position(self.frames >= 35);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 25 | 35),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("recurrence-smoke") && self.frames == 31 {
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(1100.0, 390.0)));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -430.0),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("recurrence-smoke")
            && matches!(self.frames, 25 | 26 | 35 | 36)
        {
            let pos = self
                .app
                .preview_recurrence_position(usize::from(self.frames >= 35));
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 25 | 35),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("sidebar-smoke") {
            let step = self.frames.saturating_sub(20);
            let control = match step {
                5 | 6 => Some("theme"),
                11 | 12 => Some("category"),
                22 | 23 => Some("back"),
                32 | 33 => Some("settings"),
                50 | 51 => Some("sixth"),
                94 | 95 => Some("frequent"),
                _ => None,
            };
            if let Some(control) = control {
                let pos = self.app.preview_sidebar_position(control);
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(step, 5 | 11 | 22 | 32 | 50 | 94),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if matches!(step, 40 | 41) || (60..=90).contains(&step) {
                input
                    .events
                    .push(egui::Event::PointerMoved(egui::pos2(100.0, 220.0)));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(
                        0.0,
                        if step >= 60 {
                            self.app.preview_sidebar_scroll_delta("frequent")
                        } else {
                            -1500.0
                        },
                    ),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("planner-agenda-smoke")
            && matches!(self.frames, 5 | 6)
        {
            let pos = self
                .app
                .preview_agenda_click_position()
                .expect("agenda occurrence rendered");
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frames == 5,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("planner-backup-smoke") {
            let index = match self.frames {
                5 | 6 => Some(0),
                16 | 17 => Some(1),
                22 | 23 => Some(2),
                28 | 29 => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                let pos = self
                    .app
                    .preview_backup_click_position(index)
                    .expect("backup control rendered");
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frames, 5 | 16 | 22 | 28),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if std::env::args().nth(3).as_deref() == Some("planner-files-smoke")
            && matches!(self.frames, 16 | 17)
        {
            let pos = self
                .app
                .preview_files_cancel_position()
                .expect("export cancel rendered");
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frames == 16,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("planner-trash-smoke")
            && matches!(self.frames, 5 | 6 | 16 | 17)
        {
            let pos = self
                .app
                .preview_purge_click_position(self.frames >= 16)
                .expect("purge action rendered");
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frames, 5 | 16),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if std::env::args().nth(3).as_deref() == Some("reminder-open-smoke")
            && matches!(self.frames, 5 | 6)
        {
            let pos = self
                .app
                .preview_reminder_click_position()
                .expect("reminder button rendered");
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frames == 5,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if self.scene == 64 && self.frames == 2 {
            input.dropped_files.push(egui::DroppedFile {
                path: Some(self.fixture.with_file_name("订单数据.csv")),
                ..Default::default()
            });
        }
        if self.scene != NAMES.len() {
            return;
        }
        let event = match self.frames {
            2 => Some((egui::Key::K, egui::Modifiers::CTRL)),
            4 => Some((egui::Key::ArrowDown, egui::Modifiers::NONE)),
            6 | 14 | 22 => Some((egui::Key::Enter, egui::Modifiers::NONE)),
            10 | 18 => Some((egui::Key::K, egui::Modifiers::CTRL)),
            26 => Some((egui::Key::Enter, egui::Modifiers::CTRL)),
            _ => None,
        };
        if self.frames == 12 {
            input.events.push(egui::Event::Text("保持顺序去重".into()));
        }
        if self.frames == 20 {
            input.events.push(egui::Event::Text("django-sql".into()));
        }
        if let Some((key, modifiers)) = event {
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let smoke_mode = std::env::args().nth(3);
        if smoke_mode.as_deref() == Some("clipboard-media-smoke") {
            if self.frames == 0 {
                self.app.preview_clipboard_media_smoke(ctx, 0);
                self.quick_smoke_phase = 1;
            } else if self
                .app
                .preview_clipboard_media_smoke(ctx, self.quick_smoke_phase)
            {
                if self.quick_smoke_phase == 2 {
                    println!(
                        "PASS clipboard image: actual background preview and DIB preparation, transparent pixels, typed image relay, versioned clipboard origin, source history preserved and exact recent target; synthetic only"
                    );
                    std::process::exit(0);
                }
                self.quick_smoke_phase = 2;
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(30),
                "clipboard media smoke timeout"
            );
            self.app.update(ctx, frame);
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("image-relay-smoke") {
            if self.frames == 0 {
                self.app.preview_image_relay_smoke(ctx, 0);
                self.quick_smoke_phase = 1;
            } else if self
                .app
                .preview_image_relay_smoke(ctx, self.quick_smoke_phase)
            {
                if self.quick_smoke_phase == 2 {
                    println!(
                        "PASS image relay: actual transparent selection pixels, unconfirmed targets refused, screenshot source preserved, chained conversion/editor with shared image and two versioned origins, exact recent IDs; no files written"
                    );
                    std::process::exit(0);
                }
                self.quick_smoke_phase = 2;
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(30),
                "image relay smoke timeout"
            );
            self.app.update(ctx, frame);
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }

        if smoke_mode.as_deref() == Some("screenshot-overlay-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 348, self.fixture.clone());
                self.app.preview_prefix_check(0);
                self.quick_smoke_phase = 1;
            }
            self.app.update(ctx, frame);
            let delta = self
                .frames
                .saturating_sub(self.sqlite_input_frame.unwrap_or(self.frames));
            match self.quick_smoke_phase {
                1 | 3 | 5 if self.app.preview_screenshot_overlay_active() => {
                    self.app.preview_screenshot_overlay_check(0);
                    self.sqlite_input_frame = Some(self.frames);
                    if self.quick_smoke_phase == 1 {
                        self.app.preview_screenshot_overlay_key(27, 1);
                        self.quick_smoke_phase = 2;
                    } else if self.quick_smoke_phase == 3 {
                        self.quick_smoke_phase = 4;
                    } else {
                        self.app.preview_screenshot_overlay_key(13, 28);
                        self.quick_smoke_phase = 6;
                    }
                }
                2 if delta >= 10 && !self.app.preview_screenshot_busy() => {
                    self.app.preview_screenshot_overlay_check(1);
                    self.app.preview_screenshot_overlay_check(3);
                    self.app.preview_screenshot_capture_start();
                    self.quick_smoke_phase = 3;
                }
                4 => {
                    match delta {
                        10 => self.app.preview_screenshot_overlay_pointer(200, 200, 0),
                        20 => self.app.preview_screenshot_overlay_pointer(1200, 200, 1),
                        30 => self.app.preview_screenshot_overlay_pointer(1200, 900, 1),
                        40 => self.app.preview_screenshot_overlay_pointer(750, 900, 1),
                        50 => self.app.preview_screenshot_overlay_pointer(750, 650, 1),
                        60 => self.app.preview_screenshot_overlay_pointer(200, 650, 1),
                        70 => self.app.preview_screenshot_overlay_pointer(200, 650, 2),
                        _ => {}
                    }
                    if delta >= 90 && !self.app.preview_screenshot_busy() {
                        self.app.preview_screenshot_overlay_check(2);
                        self.app.preview_screenshot_overlay_check(3);
                        self.app.preview_screenshot_capture_start();
                        self.quick_smoke_phase = 5;
                    }
                }
                6 if delta >= 15 && !self.app.preview_screenshot_busy() => {
                    self.app.preview_screenshot_overlay_check(4);
                    self.app.preview_screenshot_overlay_check(3);
                    println!(
                        "PASS synthetic S C dispatch and native primary-display overlay: root cloaked, child visible/uncloaked at physical monitor bounds; Esc retains old work; targeted Win32 mouse-message concave alpha crop; root restored; Enter full-screen source bytes preserved; unique overlays destroyed; no desktop pixels saved"
                    );
                    std::process::exit(0);
                }
                _ => {}
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(30),
                "overlay smoke timeout phase {} frame {}",
                self.quick_smoke_phase,
                self.frames
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if smoke_mode.as_deref() == Some("screenshot-desktop-smoke") {
            match zi_devtools::image_tools::verify_screenshot_capture() {
                Ok((width, height)) => println!(
                    "PASS actual eframe-DPI desktop capture {width}x{height}, concave alpha and in-memory PNG roundtrip; no desktop pixels saved"
                ),
                Err(error) => {
                    eprintln!("SCREENSHOT_CAPTURE_FAILED: {error:#}");
                    std::process::exit(2);
                }
            }
            std::process::exit(0);
        }
        #[cfg(windows)]
        if smoke_mode.as_deref() == Some("screenshot-overlay-capture") {
            if self.frames == 0 {
                self.app.preview_screenshot_overlay_prepare(ctx, self.scene);
            }
            self.app.update(ctx, frame);
            if self.frames >= 24 {
                use eframe::glow::HasContext;
                let [w, h] = self.app.preview_screenshot_overlay_dimensions();
                assert!(w > 0 && h > 0 && i64::from(w) * i64::from(h) <= 8_000_000);
                let gl = frame.gl().unwrap();
                let mut viewport = [0; 4];
                unsafe {
                    gl.get_parameter_i32_slice(eframe::glow::VIEWPORT, &mut viewport);
                }
                assert_eq!(
                    &viewport[2..],
                    &[w, h],
                    "read only the rendered child framebuffer"
                );
                let mut pixels = vec![0u8; w as usize * h as usize * 4];
                unsafe {
                    gl.read_pixels(
                        0,
                        0,
                        w,
                        h,
                        eframe::glow::RGBA,
                        eframe::glow::UNSIGNED_BYTE,
                        eframe::glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                }
                let mut flipped = Vec::with_capacity(pixels.len());
                for row in pixels.chunks_exact(w as usize * 4).rev() {
                    flipped.extend_from_slice(row);
                }
                let file = fs::File::create(
                    self.folder
                        .join(format!("screenshot-overlay-{}.png", self.scene)),
                )
                .unwrap();
                let mut encoder = png::Encoder::new(file, w as u32, h as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&flipped)
                    .unwrap();
                println!("Captured native child framebuffer: {w}x{h}");
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(20),
                "child framebuffer capture timeout"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        #[cfg(windows)]
        if smoke_mode.as_deref() == Some("clock-window-capture") {
            if self.frames == 0 {
                self.app.preview_clock_window_prepare(ctx, self.scene);
            }
            self.app.update(ctx, frame);
            if self.frames >= 24 {
                use eframe::glow::HasContext;
                let [w, h] = self.app.preview_clock_window_dimensions();
                assert!(w > 0 && h > 0 && i64::from(w) * i64::from(h) <= 8_000_000);
                let gl = frame.gl().unwrap();
                let mut viewport = [0; 4];
                unsafe {
                    gl.get_parameter_i32_slice(eframe::glow::VIEWPORT, &mut viewport);
                }
                assert_eq!(
                    &viewport[2..],
                    &[w, h],
                    "read only the rendered child framebuffer"
                );
                let mut pixels = vec![0u8; w as usize * h as usize * 4];
                unsafe {
                    gl.read_pixels(
                        0,
                        0,
                        w,
                        h,
                        eframe::glow::RGBA,
                        eframe::glow::UNSIGNED_BYTE,
                        eframe::glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                }
                let mut flipped = Vec::with_capacity(pixels.len());
                for row in pixels.chunks_exact(w as usize * 4).rev() {
                    flipped.extend_from_slice(row);
                }
                let file =
                    fs::File::create(self.folder.join(format!("clock-child-{}.png", self.scene)))
                        .unwrap();
                let mut encoder = png::Encoder::new(file, w as u32, h as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&flipped)
                    .unwrap();
                println!("Captured native child framebuffer: {w}x{h}");
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(20),
                "child framebuffer capture timeout"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        #[cfg(windows)]
        if smoke_mode.as_deref() == Some("clock-window-smoke") {
            if self.frames == 0 {
                self.app.preview_clock_window_prepare(ctx, 4);
            }
            self.app.update(ctx, frame);
            match self.frames {
                20 => {
                    self.clock_window_handle = self.app.preview_clock_window_check(ctx, 0);
                }
                30 => self.app.preview_clock_window_click(4),
                40 => {
                    self.app.preview_clock_window_check(ctx, 1);
                    self.app.preview_clock_window_key(32, 57);
                }
                50 => {
                    self.app.preview_clock_window_check(ctx, 2);
                    self.app.preview_clock_window_click(0);
                }
                65 => {
                    self.app.preview_clock_window_check(ctx, 3);
                    self.app.preview_clock_window_click(1);
                }
                90 => {
                    self.app.preview_clock_window_check(ctx, 4);
                    self.app.preview_clock_window_key(27, 1);
                }
                115 => {
                    self.app.preview_clock_window_check(ctx, 5);
                    self.app.preview_clock_window_click(2);
                }
                135 => self.app.preview_clock_window_click(4),
                155 => {
                    self.clock_window_handle = self.app.preview_clock_window_check(ctx, 6);
                    self.app.preview_clock_window_click(3);
                }
                180 => {
                    assert_eq!(
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(
                                self.clock_window_handle as _,
                            )
                        },
                        0,
                        "closed child HWND still alive"
                    );
                    self.app.preview_clock_window_check(ctx, 7);
                }
                205 => {
                    self.clock_window_handle = self.app.preview_clock_window_check(ctx, 8);
                    self.app.preview_clock_window_click(3);
                }
                230 => {
                    assert_eq!(
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(
                                self.clock_window_handle as _,
                            )
                        },
                        0
                    );
                    self.app.preview_clock_window_check(ctx, 9);
                    std::process::exit(0);
                }
                _ => {}
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(30),
                "child smoke timeout"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if smoke_mode.as_deref() == Some("clock-audio-smoke") {
            if self.frames == 0 {
                self.app.preview_clock_audio_prepare();
            }
            self.app.update(ctx, frame);
            match self.frames {
                25 => {
                    self.app.preview_clock_audio_check(0);
                }
                40 => {
                    assert!(self.app.preview_clock_audio_check(1));
                }
                60 => {
                    self.app.preview_clock_audio_check(2);
                }
                80 => {
                    self.app.preview_clock_audio_check(3);
                }
                _ if self.frames >= 90 => {
                    if self.app.preview_clock_audio_check(4) {
                        std::process::exit(0);
                    }
                }
                _ => {}
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(25),
                "audio UI test timeout"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("clock-save-smoke" | "clock-restore-smoke")
        ) {
            let saving = smoke_mode.as_deref() == Some("clock-save-smoke");
            if self.frames == 0 {
                self.app.preview_clock_storage_prepare(saving);
            }
            self.app.update(ctx, frame);
            if self.quick_smoke_phase == 0
                && self
                    .app
                    .preview_clock_storage_ready(if saving { 0 } else { 2 })
            {
                self.quick_smoke_phase = 1;
                self.frames = 10;
            }
            if self.frames >= 60
                && self
                    .app
                    .preview_clock_storage_ready(if saving { 1 } else { 3 })
            {
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(25),
                "clock storage native timed out"
            );
            if self.quick_smoke_phase > 0 {
                self.frames += 1;
            }
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if smoke_mode.as_deref() == Some("clock-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 328, self.fixture.clone());
                self.app.preview_clock_check(0);
            }
            self.app.update(ctx, frame);
            match self.frames {
                50 => self.app.preview_clock_check(1),
                65 => self.app.preview_clock_check(2),
                75 => self.app.preview_clock_check(3),
                _ if self.frames >= 150 => {
                    if self.app.preview_clock_done() {
                        std::process::exit(0);
                    }
                    assert!(
                        self.started.elapsed() < Duration::from_secs(20),
                        "clock timer did not complete in real time"
                    );
                }
                _ => {}
            };
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if smoke_mode.as_deref() == Some("profile-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 314, self.fixture.clone());
                self.app.preview_profile_check(0, &self.folder);
            }
            self.app.update(ctx, frame);
            match self.frames {
                15 => self.app.preview_profile_check(1, &self.folder),
                25 => self.app.preview_profile_check(2, &self.folder),
                45 => self.app.preview_profile_check(3, &self.folder),
                55 => self.app.preview_profile_check(4, &self.folder),
                75 => self.app.preview_profile_check(5, &self.folder),
                100 => {
                    self.app.preview_profile_check(6, &self.folder);
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if smoke_mode.as_deref() == Some("bindings-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 314, self.fixture.clone());
                self.app.preview_binding_check(0);
            }
            self.app.update(ctx, frame);
            match self.frames {
                25 => self.app.preview_binding_check(1),
                65 => self.app.preview_binding_check(2),
                75 => self.app.preview_binding_check(3),
                87 => self.app.preview_binding_check(8),
                95 => self.app.preview_binding_check(4),
                105 => self.app.preview_binding_check(5),
                115 => self.app.preview_binding_check(6),
                135 => {
                    self.app.preview_binding_check(7);
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if smoke_mode.as_deref() == Some("prefix-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 314, self.fixture.clone());
                self.app.preview_prefix_check(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            match self.frames {
                25 => self.app.preview_prefix_check(1),
                40 => self.app.preview_prefix_check(2),
                60 => self.app.preview_prefix_check(3),
                63 => self.app.preview_prefix_check(8),
                66 => self.app.preview_prefix_check(6),
                68 => self.app.preview_prefix_check(0),
                80 => self.app.preview_prefix_check(4),
                83 => self.app.preview_prefix_check(7),
                84 | 88 => self.app.preview_prefix_check(0),
                87 => self.app.preview_prefix_check(7),
                100 => {
                    self.app.preview_prefix_check(5);
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }

        if smoke_mode.as_deref() == Some("plot-smoke") {
            assert!(
                self.started.elapsed() < Duration::from_secs(60),
                "plot smoke timeout"
            );
            if self.frames == 0 {
                self.app.preview_scene(ctx, 432, self.fixture.clone());
                self.app.preview_plot_fixture(false);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            if matches!(self.frames, 20 | 100) && !self.app.preview_plot_ready() {
                ctx.request_repaint_after(Duration::from_millis(30));
                return;
            }
            if self.frames == 25 {
                self.app.preview_plot_check(0);
            }
            if self.frames == 40 {
                self.started = Instant::now();
            }
            if self.frames == 41 && self.started.elapsed() < Duration::from_millis(400) {
                ctx.request_repaint_after(Duration::from_millis(30));
                return;
            }
            if self.frames == 65 {
                self.app.preview_plot_check(1);
            }
            if self.frames == 80 {
                self.app.preview_plot_check(2);
            }
            if self.frames == 120 {
                self.app.preview_plot_handoff_check();
            }
            if self.frames == 145 {
                if !self.app.preview_plot_received() {
                    ctx.request_repaint_after(Duration::from_millis(30));
                    return;
                }
                fs::write(
                    self.folder.join("native-plot.svg"),
                    self.app.preview_plot_svg(),
                )
                .unwrap();
                println!(
                    "PASS native plot: draw two functions; actual wheel zoom and drag pan; keyboard formula edit invalidates exports; redraw; CSV handoff new instance and actual parse of all 129x3 values; original work/ans/x/history retained; SVG rendering fixture saved"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("boundary-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 420, self.fixture.clone());
                self.app.preview_boundary_fixture();
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            if matches!(self.frames, 20 | 25) {
                self.started = Instant::now();
            }
            if matches!(self.frames, 21 | 26) && self.started.elapsed() < Duration::from_millis(400)
            {
                ctx.request_repaint_after(Duration::from_millis(30));
                return;
            }
            if self.frames == 35 {
                self.app.preview_boundary_check(0);
            }
            if self.frames == 45 {
                self.app.preview_boundary_check(1);
            }
            if self.frames == 80 {
                self.app.preview_mapping_check(false);
            }
            if self.frames == 110 {
                self.app.preview_boundary_check(2);
                println!(
                    "PASS numeric boundaries: actual column-list wheel scroll; ninth column disabled and rejected at eight; native column reorder and fixed footer; 8x8/64 exact values including fraction and i128 MAX received in B after refusal and consent; A/expression/variables/source preserved; no calculation at 980x760"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("entry-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 420, self.fixture.clone());
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            if self.frames == 18 {
                assert!(
                    ctx.screen_rect()
                        .shrink2(egui::vec2(0.0, 50.0))
                        .contains(self.app.preview_mapping_position(4))
                );
            }
            if self.frames == 35 {
                self.app.preview_numeric_entry_cancelled();
            }
            if self.frames == 80 {
                self.app.preview_mapping_check(false);
            }
            if self.frames == 110 {
                self.app.preview_mapping_check(true);
                println!(
                    "PASS numeric entry: first-screen button without scrolling; independent selector, cancel preserves source and calculator, reopen and preview, B refusal/consent/receive and no execution at 980x760"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("mapping-smoke" | "mapping-small-smoke")
        ) {
            let small = smoke_mode.as_deref() == Some("mapping-small-smoke");
            if self.frames == 0 {
                self.app
                    .preview_scene(ctx, if small { 420 } else { 418 }, self.fixture.clone());
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if small {
                    egui::vec2(980.0, 760.0)
                } else {
                    egui::vec2(1280.0, 1180.0)
                }));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            if self.frames == 8 || (small && matches!(self.frames, 11 | 14 | 17)) {
                self.started = Instant::now();
            }
            if (self.frames == 9 || (small && matches!(self.frames, 12 | 15 | 18)))
                && self.started.elapsed() < Duration::from_millis(400)
            {
                ctx.request_repaint_after(Duration::from_millis(30));
                return;
            }
            if small && self.frames == 19 {
                println!(
                    "small mapping viewport={:?} send={:?}",
                    ctx.screen_rect(),
                    self.app.preview_mapping_position(0)
                );
                assert!(
                    ctx.screen_rect()
                        .shrink2(egui::vec2(0.0, 50.0))
                        .contains(self.app.preview_mapping_position(0)),
                    "scroll must reveal the send control inside the small viewport"
                );
            }
            if self.frames == 50 {
                self.app.preview_mapping_check(false);
            }
            if self.frames == 80 {
                self.app.preview_mapping_check(true);
                println!(
                    "PASS mapping native: selected sorted-view rows and reordered named columns, source preserved, B selection, dirty replacement rejected before consent, explicit checkbox and receive, A/expression/variables preserved, exact fractions, no calculation"
                );
                if small {
                    println!(
                        "PASS small mapping: actual mouse-wheel scroll reveals send control; B selection, refusal, consent and footer actions complete at 980x760"
                    );
                }
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("numeric-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 410, self.fixture.clone());
                self.app.preview_numeric_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1280.0, 900.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            match self.frames {
                50 => self.app.preview_numeric_smoke(1),
                60 => self.app.preview_numeric_smoke(2),
                70 => self.app.preview_numeric_smoke(3),
                90 => self.app.preview_numeric_smoke(4),
                150 => {
                    self.app.preview_numeric_smoke(5);
                    println!(
                        "PASS numeric native handoff: source send, text/approximate/typed selection, new data instance and actual background parse, JSON export typed roundtrip, dirty calculator rejection, native matrix receive without execution, exact i128/fraction and f64 subnormal/negative-zero preserved; synthetic only"
                    );
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("worksheet-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 406, self.fixture.clone());
                self.app.preview_sheet_io(ctx, &self.folder, 0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1280.0, 900.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            match self.frames {
                30 => self.app.preview_sheet_io(ctx, &self.folder, 1),
                50 => self.app.preview_sheet_io(ctx, &self.folder, 2),
                52 => self.app.preview_sheet_io(ctx, &self.folder, 6),
                54 => self.app.preview_sheet_io(ctx, &self.folder, 7),
                100 => self.app.preview_sheet_io(ctx, &self.folder, 3),
                110 => self.app.preview_sheet_io(ctx, &self.folder, 4),
                140 => {
                    self.app.preview_sheet_io(ctx, &self.folder, 5);
                    println!(
                        "PASS actual worksheet: background atomic save/read, later edits remain dirty, native replacement checkbox and restore button, no assignment replay, exact fractions, tray exit guard and no-overwrite failure; synthetic only"
                    );
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("matrix-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 402, self.fixture.clone());
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1280.0, 900.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            if self.frames == 40 {
                self.app.preview_matrix_check();
                println!(
                    "PASS matrix native calculate button: pivoted exact multi-RHS solution; fixture only"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("calculator-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 310, self.fixture.clone());
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 640.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.app.update(ctx, frame);
            if self.frames == 60 {
                self.app.preview_calculator_check(0);
            }
            if self.frames == 100 {
                self.app.preview_calculator_check(1);
                println!(
                    "PASS calculator native input: exact decimal Enter commit; zero division preserves ans/history"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("workflow-smoke" | "workflow-light-smoke" | "workflow-small-smoke")
        ) {
            assert!(
                self.started.elapsed() < Duration::from_secs(35),
                "workflow interaction timed out"
            );
            if self.frames == 0 {
                let scene = if smoke_mode.as_deref() == Some("workflow-light-smoke") {
                    301
                } else {
                    300
                };
                self.app.preview_scene(ctx, scene, self.fixture.clone());
                self.app.preview_workflow_check(0);
                if smoke_mode.as_deref() == Some("workflow-small-smoke") {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        980.0, 640.0,
                    )));
                }
            }
            self.app.update(ctx, frame);
            if matches!(self.frames, 20 | 40 | 60 | 90 | 110)
                && self.workflow_input_frame != Some(self.frames)
            {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            match self.frames {
                30 => {
                    self.app.preview_workflow_check(1);
                }
                50 => {
                    self.app.preview_workflow_check(2);
                }
                80 => {
                    if !self.app.preview_workflow_check(3) {
                        ctx.request_repaint_after(Duration::from_millis(60));
                        return;
                    }
                }
                100 => {
                    self.app.preview_workflow_check(4);
                }
                120 => {
                    self.app.preview_workflow_check(5);
                    println!(
                        "PASS native synthetic pointer cancel/confirm without input, background parse/preview, explicit apply and undo; original input retained"
                    );
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("sqlite-export-smoke") {
            assert!(self.started.elapsed() < Duration::from_secs(35));
            if self.frames == 0 {
                self.app.preview_scene(ctx, 0, self.fixture.clone());
                self.app.preview_sqlite_smoke(0, &self.fixture);
            }
            self.app.update(ctx, frame);
            if matches!(self.frames, 20 | 40 | 60 | 90)
                && self.sqlite_input_frame != Some(self.frames)
            {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 30 {
                self.app.preview_sqlite_smoke(1, &self.fixture);
            }
            if self.frames == 50 {
                self.app.preview_sqlite_smoke(2, &self.fixture);
            }
            if self.frames == 80 && !self.app.preview_sqlite_smoke(3, &self.fixture) {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames >= 115 && self.app.preview_sqlite_smoke(4, &self.fixture) {
                println!(
                    "PASS native back/preview/confirm/open, independent SQLite values, source and unrelated instance preserved"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("convert-event-smoke" | "convert-memo-smoke")
        ) {
            let calendar = smoke_mode.as_deref() == Some("convert-event-smoke");
            if self.frames == 0 {
                self.app.preview_scene(ctx, 276, self.fixture.clone());
                self.app.preview_convert_smoke(0, calendar);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 40 {
                self.app.preview_convert_smoke(1, calendar);
            }
            if self.frames >= 70 {
                if !self.app.preview_convert_smoke(2, calendar) {
                    ctx.request_repaint_after(Duration::from_millis(60));
                    return;
                }
                std::process::exit(0);
            }
            assert!(self.started.elapsed() < Duration::from_secs(35));
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }

        if smoke_mode.as_deref() == Some("duplicate-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 276, self.fixture.clone());
                self.app.preview_duplicate_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 40 {
                self.app.preview_duplicate_smoke(1);
            }
            if self.frames >= 70 {
                if !self.app.preview_duplicate_smoke(2) {
                    ctx.request_repaint_after(Duration::from_millis(60));
                    return;
                }
                std::process::exit(0);
            }
            assert!(self.started.elapsed() < Duration::from_secs(35));
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }

        if matches!(
            smoke_mode.as_deref(),
            Some("calendar-navigation-smoke" | "week-navigation-smoke")
        ) {
            let week = smoke_mode.as_deref() == Some("week-navigation-smoke");
            if self.frames == 0 {
                self.app.preview_planner_navigation_smoke(0, week);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 45 {
                self.app.preview_planner_navigation_smoke(1, week);
            }
            if self.frames >= 80 {
                self.app.preview_planner_navigation_smoke(2, week);
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("handoff-discovery-smoke") {
            if self.frames == 0 {
                self.app.preview_handoff_discovery_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames >= 65 {
                self.app.preview_handoff_discovery_smoke(1);
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("event-handoff-smoke") {
            if self.frames == 0 {
                self.app.preview_event_handoff_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames >= 65 {
                self.app.preview_event_handoff_smoke(1);
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("week-scroll-smoke") {
            if self.frames == 0 {
                self.app.preview_week_scroll_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 45 {
                self.app.preview_week_scroll_smoke(1);
            }
            if self.frames == 70 {
                self.app.preview_week_scroll_smoke(2);
            }
            if self.frames >= 95 {
                self.app.preview_week_scroll_smoke(3);
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("week-smoke" | "week-small-smoke")
        ) {
            if self.frames == 0 {
                self.app.preview_week_smoke(0);
                let size = if smoke_mode.as_deref() == Some("week-small-smoke") {
                    egui::vec2(980.0, 760.0)
                } else {
                    egui::vec2(1440.0, 980.0)
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            self.app.update(ctx, frame);
            if self.frames >= 40 {
                self.app.preview_week_smoke(1);
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("listing-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 270, self.fixture.clone());
                self.app.preview_listing_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 640.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 30 {
                self.app.preview_listing_smoke(1);
            }
            if self.frames == 70 {
                self.app.preview_listing_smoke(2);
            }
            if self.frames == 90 {
                self.app.preview_listing_smoke(3);
            }
            if self.frames == 95 {
                self.auto_minimize_smoke_at = Some(Instant::now());
            }
            if self.frames == 96
                && self
                    .auto_minimize_smoke_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
            {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames >= 110 {
                self.app.preview_listing_smoke(4);
                std::process::exit(0);
            }
            assert!(self.started.elapsed() < Duration::from_secs(30));
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }

        if smoke_mode.as_deref() == Some("snooze-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 266, self.fixture.clone());
                self.app.preview_snooze_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 640.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 40 {
                self.app.preview_snooze_smoke(1);
            }
            if self.frames >= 60 {
                if !self.app.preview_snooze_smoke(2) {
                    ctx.request_repaint_after(Duration::from_millis(60));
                    return;
                }
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(30),
                "snooze smoke timed out"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }

        if matches!(
            smoke_mode.as_deref(),
            Some("actions-smoke" | "actions-min-smoke")
        ) {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 256, self.fixture.clone());
                self.app.preview_actions_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    if smoke_mode.as_deref() == Some("actions-min-smoke") {
                        egui::vec2(980.0, 640.0)
                    } else {
                        egui::vec2(1280.0, 900.0)
                    },
                ));
            }
            self.app.update(ctx, frame);
            if self.frames == 40 && !self.app.preview_actions_smoke(1) {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 45 {
                self.auto_minimize_smoke_at = Some(Instant::now());
            }
            if self.frames == 46
                && self
                    .auto_minimize_smoke_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
            {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 50 {
                self.app.preview_actions_smoke(2);
            }
            if self.frames >= 70 {
                self.app.preview_actions_smoke(3);
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(40),
                "actions smoke timed out"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("cutoff-smoke" | "cutoff-compact-smoke")
        ) {
            let compact = smoke_mode.as_deref() == Some("cutoff-compact-smoke");
            if self.frames == 0 {
                self.app.preview_scene(ctx, 252, self.fixture.clone());
                self.app.preview_cutoff_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if compact {
                    egui::vec2(980.0, 760.0)
                } else {
                    egui::vec2(1280.0, 1180.0)
                }));
            }
            if compact && self.frames == 6 {
                self.app.preview_planner_editor();
                self.auto_minimize_smoke_at = Some(Instant::now());
            }
            self.app.update(ctx, frame);
            if matches!(self.frames, 7 | 46)
                && self
                    .auto_minimize_smoke_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
            {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 30 {
                self.app.preview_cutoff_smoke(1);
            }
            if self.frames == 41 {
                self.app.preview_cutoff_smoke(2);
            }
            if self.frames == 45 {
                self.auto_minimize_smoke_at = Some(Instant::now());
            }
            if self.frames >= 65 && self.app.preview_cutoff_smoke(3) {
                std::process::exit(0);
            }
            assert!(self.frames < 220, "cutoff save timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("cutoff-interop") {
            self.app.preview_cutoff_interop(&self.folder);
            std::process::exit(0);
        }
        if smoke_mode.as_deref() == Some("ics-interop") {
            self.app.preview_ics_interop(&self.folder);
            std::process::exit(0);
        }
        if smoke_mode.as_deref() == Some("ics-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 248, self.fixture.clone());
                self.app.preview_ics_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 760.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 30 {
                self.app.preview_ics_smoke(1);
            }
            if self.frames == 50 {
                self.app.preview_ics_smoke(2);
            }
            if self.frames >= 80 && self.app.preview_ics_smoke(3) {
                std::process::exit(0);
            }
            assert!(self.frames < 240, "ICS import timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("interval-smoke" | "interval-compact-smoke")
        ) {
            let compact = smoke_mode.as_deref() == Some("interval-compact-smoke");
            if self.frames == 0 {
                self.app.preview_scene(ctx, 238, self.fixture.clone());
                self.app.preview_interval_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if compact {
                    egui::vec2(980.0, 760.0)
                } else {
                    egui::vec2(1280.0, 900.0)
                }));
            }
            if compact && self.frames == 6 {
                self.app.preview_planner_editor();
            }
            self.app.update(ctx, frame);
            // A frame count alone can elapse before smooth scrolling settles.
            if compact && self.frames == 7 && self.started.elapsed() < Duration::from_secs(3) {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 30 {
                self.app.preview_interval_smoke(1);
            }
            if self.frames >= 48 && self.app.preview_interval_smoke(2) {
                std::process::exit(0);
            }
            assert!(self.frames < 200, "interval save timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("recurrence-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 234, self.fixture.clone());
                self.app.preview_recurrence_smoke(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1280.0, 1180.0)));
            }
            self.app.update(ctx, frame);
            if self.frames == 31 {
                self.auto_minimize_smoke_at = Some(Instant::now());
            }
            if self.frames == 32
                && self
                    .auto_minimize_smoke_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
            {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 30 {
                self.app.preview_recurrence_smoke(1);
            }
            if self.frames >= 48 && self.app.preview_recurrence_smoke(2) {
                std::process::exit(0);
            }
            assert!(self.frames < 200, "recurrence save timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("sidebar-smoke") {
            let step = self.frames.saturating_sub(20);
            if matches!(step, 0 | 16 | 27 | 37 | 56) {
                self.app.preview_scene(ctx, 230, self.fixture.clone());
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(980.0, 640.0)));
            }
            self.app.update(ctx, frame);
            if self.frames < 20 {
                self.frames += 1;
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            match step {
                4 | 20 | 30 => self.app.preview_sidebar_assert(0),
                9 => self.app.preview_sidebar_assert(1),
                15 => self.app.preview_sidebar_assert(2),
                26 => self.app.preview_sidebar_assert(3),
                36 => self.app.preview_sidebar_assert(4),
                49 => self.app.preview_sidebar_assert(5),
                54 => self.app.preview_sidebar_assert(6),
                99 => {
                    self.app.preview_sidebar_assert(7);
                    std::process::exit(0);
                }
                _ => {}
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("planner-agenda-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 226, self.fixture.clone());
            }
            self.app.update(ctx, frame);
            if self.frames == 12 {
                self.app.preview_agenda_assert();
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("planner-backup-smoke") {
            if self.frames == 0 {
                self.app.preview_backup_smoke(0);
            }
            if self.frames == 10 {
                self.app.preview_backup_smoke(1);
            }
            if self.frames == 20 {
                self.app.preview_backup_smoke(2);
            }
            self.app.update(ctx, frame);
            if self.frames >= 50 && self.app.preview_backup_smoke(3) {
                std::process::exit(0);
            }
            assert!(self.frames < 200, "backup restore timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("planner-files-smoke") {
            if self.frames == 0 {
                self.app.preview_files_smoke(0);
            }
            if self.frames == 12 {
                self.app.preview_files_smoke(1);
            }
            if self.frames == 24 {
                self.app.preview_files_smoke(2);
            }
            self.app.update(ctx, frame);
            if self.frames >= 40 && self.app.preview_files_smoke(3) {
                std::process::exit(0);
            }
            assert!(self.frames < 150, "file operation timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("planner-trash-smoke") {
            if self.frames == 0 {
                self.app.preview_purge_smoke(0);
            }
            if self.frames == 12 {
                self.app.preview_purge_smoke(1);
            }
            self.app.update(ctx, frame);
            if self.frames >= 30 && self.app.preview_purge_smoke(2) {
                std::process::exit(0);
            }
            assert!(self.frames < 150, "purge operation timed out");
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if smoke_mode.as_deref() == Some("reminder-open-smoke") {
            if self.frames == 0 {
                self.app.preview_reminder_click_start(ctx);
            }
            self.app.update(ctx, frame);
            if self.frames == 14 {
                self.app.preview_reminder_click_assert();
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some("planner-tray-smoke" | "planner-minimize-smoke")
        ) {
            if self.frames == 0 {
                self.app.preview_planner_timer(
                    ctx,
                    smoke_mode.as_deref() == Some("planner-minimize-smoke"),
                );
            }
            self.frames += 1;
            self.app.update(ctx, frame);
            return;
        }
        if smoke_mode.as_deref() == Some("taskbar-smoke") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 0, self.fixture.clone());
            }
            if self.frames == 5 {
                self.app.preview_hide_and_restore_taskbar(ctx, false);
            }
            self.app.update(ctx, frame);
            if self.frames == 16 {
                assert!(
                    self.app.preview_hidden() && self.app.preview_taskbar_detached(),
                    "hidden workbench must leave taskbar"
                );
                self.app.preview_hide_and_restore_taskbar(ctx, true);
            }
            if self.frames == 26 {
                assert!(
                    !self.app.preview_hidden() && !self.app.preview_taskbar_detached(),
                    "restored workbench must return to taskbar"
                );
                println!("PASS eframe taskbar style hide and restore");
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some(
                "recorder-quick-panel-preview"
                    | "recorder-quick-panel-smoke"
                    | "recorder-quick-cancel-smoke"
            )
        ) {
            let interactive = smoke_mode.as_deref() == Some("recorder-quick-panel-smoke");
            let cancel = smoke_mode.as_deref() == Some("recorder-quick-cancel-smoke");
            if self.frames == 0 {
                self.app
                    .preview_scene(ctx, self.scene, self.fixture.clone());
                self.app
                    .preview_start_recorder_auto_minimize(&self.folder, if cancel { 5 } else { 0 })
                    .unwrap();
            }
            self.app.update(ctx, frame);
            if self.quick_smoke_phase == 0
                && self.app.preview_minimized()
                && self.app.preview_recorder_status()
                    == if cancel {
                        zi_devtools::recorder_ui::TrayRecordingStatus::Countdown
                    } else {
                        zi_devtools::recorder_ui::TrayRecordingStatus::Recording
                    }
            {
                self.app.preview_open_quick(ctx);
                self.quick_smoke_phase = 1;
            }
            if self.quick_smoke_phase == 1 && self.app.preview_panel_rendered() {
                println!("READY recorder quick panel");
                self.quick_smoke_phase = 2;
                if interactive || cancel {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(300));
                        click_quick_recorder_control(cancel);
                    });
                }
            }
            if cancel
                && self.quick_smoke_phase == 2
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Idle
                && !self.app.preview_minimized()
            {
                assert!(self.app.preview_recorder_last_file().is_none());
                println!("PASS quick panel countdown cancellation restores window");
                std::process::exit(0);
            }
            if interactive
                && self.quick_smoke_phase == 2
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Paused
            {
                println!("PASS quick panel pause");
                self.quick_smoke_phase = 3;
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    click_quick_recorder_control(false);
                });
            }
            if interactive
                && self.quick_smoke_phase == 3
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Recording
            {
                println!("PASS quick panel resume");
                self.quick_smoke_phase = 4;
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    click_quick_recorder_control(true);
                });
            }
            if !interactive
                && !cancel
                && self.quick_smoke_phase == 2
                && self.started.elapsed() > Duration::from_secs(30)
            {
                self.app.preview_stop_recorder();
                self.quick_smoke_phase = 4;
            }
            if let Some(path) = self.app.preview_recorder_last_file() {
                assert!(!self.app.preview_minimized());
                if interactive {
                    assert_eq!(self.quick_smoke_phase, 4);
                }
                fs::remove_file(path).unwrap();
                println!("PASS recorder quick panel rendered and stopped recording");
                std::process::exit(0);
            }
            assert!(self.started.elapsed() < Duration::from_secs(45));
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some(
                "recorder-auto-minimize-smoke"
                    | "recorder-auto-minimize-countdown-smoke"
                    | "recorder-auto-minimize-cancel-smoke"
            )
        ) {
            let countdown = smoke_mode.as_deref() == Some("recorder-auto-minimize-countdown-smoke");
            let cancel = smoke_mode.as_deref() == Some("recorder-auto-minimize-cancel-smoke");
            if self.frames == 0 {
                self.app.preview_scene(ctx, 100, self.fixture.clone());
                self.app
                    .preview_start_recorder_auto_minimize(
                        &self.folder,
                        if cancel {
                            5
                        } else if countdown {
                            3
                        } else {
                            0
                        },
                    )
                    .unwrap();
            }
            self.app.update(ctx, frame);
            if self.app.preview_minimized() {
                self.auto_minimize_smoke_at.get_or_insert_with(Instant::now);
            }
            if self.app.preview_recorder_status()
                == zi_devtools::recorder_ui::TrayRecordingStatus::Recording
            {
                self.auto_minimize_recording_at
                    .get_or_insert_with(Instant::now);
            }
            if !self.auto_minimize_smoke_stop_requested
                && (if cancel {
                    self.auto_minimize_smoke_at
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(1))
                } else {
                    self.auto_minimize_recording_at
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(2))
                })
            {
                self.app.preview_stop_recorder();
                self.auto_minimize_smoke_stop_requested = true;
            }
            if cancel
                && self.auto_minimize_smoke_stop_requested
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Idle
                && !self.app.preview_minimized()
            {
                assert!(self.app.preview_recorder_last_file().is_none());
                println!("PASS recorder countdown cancellation restores window");
                std::process::exit(0);
            }
            if let Some(path) = self.app.preview_recorder_last_file() {
                assert!(self.auto_minimize_smoke_at.is_some());
                assert!(!self.app.preview_minimized(), "主窗口未在录制完成后恢复");
                let bytes = fs::read(&path).unwrap();
                assert!(bytes.len() > 1024 && bytes.windows(4).any(|part| part == b"moov"));
                fs::remove_file(path).unwrap();
                println!("PASS recorder auto-minimize and restore");
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(25),
                "自动最小化录屏超时"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some(
                "recorder-smoke"
                    | "recorder-fullscreen-smoke"
                    | "recorder-dynamic-smoke"
                    | "recorder-long-smoke"
                    | "recorder-long-mix-smoke"
            )
        ) {
            let dynamic = matches!(
                smoke_mode.as_deref(),
                Some("recorder-dynamic-smoke" | "recorder-long-smoke" | "recorder-long-mix-smoke")
            );
            let record_seconds = if matches!(
                smoke_mode.as_deref(),
                Some("recorder-long-smoke" | "recorder-long-mix-smoke")
            ) {
                60
            } else if dynamic {
                12
            } else {
                2
            };
            let audio_mode = if smoke_mode.as_deref() == Some("recorder-long-mix-smoke") {
                AudioMode::SystemAndMicrophone
            } else if dynamic {
                AudioMode::Microphone
            } else {
                AudioMode::None
            };
            if self.frames == 0 {
                let display = recorder::primary_display().unwrap();
                println!(
                    "eframe primary display: {}x{}",
                    display.width, display.height
                );
                let region = Region {
                    x: 0,
                    y: 0,
                    width: if smoke_mode.as_deref() == Some("recorder-fullscreen-smoke") {
                        display.width & !1
                    } else {
                        display.width.min(640) & !1
                    },
                    height: if smoke_mode.as_deref() == Some("recorder-fullscreen-smoke") {
                        display.height & !1
                    } else {
                        display.height.min(360) & !1
                    },
                };
                self.recorder_smoke_size = Some((region.width, region.height));
                self.recorder_smoke = Some(
                    recorder::start_on_display(
                        region,
                        self.folder.join(format!(
                            "recorder-eframe-smoke-{}.mp4",
                            uuid::Uuid::new_v4()
                        )),
                        audio_mode,
                        AudioGains::default(),
                        recorder::RecordingQuality::default(),
                        display,
                    )
                    .unwrap(),
                );
            }
            if let Some(session) = &self.recorder_smoke {
                for event in session.events.try_iter() {
                    match event {
                        Event::Started => self.recorder_smoke_started = Some(Instant::now()),
                        Event::Finished(result) => {
                            let path = result.unwrap();
                            let data = fs::read(&path).unwrap();
                            assert!(data.len() > 1024 && data.windows(4).any(|v| v == b"moov"));
                            let track_size = data
                                .windows(4)
                                .enumerate()
                                .find_map(|(pos, tag)| {
                                    if tag != b"tkhd" || pos < 4 {
                                        return None;
                                    }
                                    let start = pos - 4;
                                    let size = u32::from_be_bytes(data[start..pos].try_into().ok()?)
                                        as usize;
                                    let end = start.checked_add(size)?;
                                    if size < 16 || end > data.len() {
                                        return None;
                                    }
                                    let width =
                                        u32::from_be_bytes(data[end - 8..end - 4].try_into().ok()?)
                                            >> 16;
                                    let height =
                                        u32::from_be_bytes(data[end - 4..end].try_into().ok()?)
                                            >> 16;
                                    Some((width, height))
                                })
                                .expect("MP4 track dimensions");
                            assert_eq!(Some(track_size), self.recorder_smoke_size);
                            if dynamic {
                                let tracks = mp4_track_durations(&data);
                                let video =
                                    tracks.iter().find(|(kind, _)| kind == b"vide").unwrap().1;
                                let audio =
                                    tracks.iter().find(|(kind, _)| kind == b"soun").unwrap().1;
                                assert!(
                                    (video - record_seconds as f64).abs() < 1.5,
                                    "video={video:.3}s expected={record_seconds}s"
                                );
                                assert!(
                                    (audio - video).abs() < 0.75,
                                    "audio={audio:.3}s video={video:.3}s"
                                );
                                println!(
                                    "MP4 tracks: mode={audio_mode:?} video={video:.3}s audio={audio:.3}s"
                                );
                            }
                            fs::remove_file(path).unwrap();
                            println!("PASS eframe recorder smoke");
                            std::process::exit(0);
                        }
                        Event::Interrupted { reason, .. } => {
                            panic!("recorder smoke interrupted: {reason}");
                        }
                    }
                }
                if self
                    .recorder_smoke_started
                    .is_some_and(|at| at.elapsed() >= Duration::from_secs(record_seconds))
                {
                    session.stop.store(true, Ordering::Release);
                }
            }
            if self.started.elapsed() > Duration::from_secs(record_seconds + 25) {
                panic!("eframe recorder smoke timed out");
            }
            if dynamic {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    let phase = self.started.elapsed().as_secs_f32() * 2.0;
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::from_rgb(23, 31, 46));
                    let x = rect.left() + 100.0 + (phase.sin() + 1.0) * 140.0;
                    ui.painter().circle_filled(
                        egui::pos2(x, rect.top() + 120.0),
                        55.0,
                        egui::Color32::from_rgb(51, 208, 174),
                    );
                    ui.label(format!("Animated capture fixture · frame {}", self.frames));
                });
            } else {
                self.app.update(ctx, frame);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if std::env::args().nth(3).as_deref() == Some("recorder-interaction") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 100, self.fixture.clone());
                self.app.preview_begin_recorder_selection(ctx);
                std::thread::spawn(drag_recorder_region);
            }
            self.app.update(ctx, frame);
            if let Some(region) = self.app.preview_recorder_region() {
                if region.x != 180 || region.y != 120 {
                    println!("PASS recorder drag: {region:?}");
                    std::process::exit(0);
                }
            }
            if self.started.elapsed() > Duration::from_secs(30) {
                panic!("recorder drag timed out");
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if std::env::args().nth(3).as_deref() == Some("recorder-visual") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 100, self.fixture.clone());
                self.app.preview_begin_recorder_selection(ctx);
            }
            self.app.update(ctx, frame);
            if self.started.elapsed() > Duration::from_secs(60) {
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if self.started.elapsed() > Duration::from_secs(240) {
            panic!("UI capture timed out before verification completed");
        }
        let screenshots = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        for image in screenshots {
            let file =
                fs::File::create(self.folder.join(format!("{}.png", NAMES[self.scene]))).unwrap();
            let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let bytes = image
                .pixels
                .iter()
                .flat_map(|p| p.to_array())
                .collect::<Vec<_>>();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&bytes)
                .unwrap();
            println!(
                "Captured {}: {}x{}",
                NAMES[self.scene], image.size[0], image.size[1]
            );
            if std::env::args().nth(3).as_deref() == Some("single") {
                std::process::exit(0);
            }
            self.scene += 1;
            self.frames = 0;
            self.pending = false;
        }
        if self.scene == NAMES.len() {
            if self.frames == 0 {
                self.app.preview_keyboard_fixture();
                self.app.preview_scene(ctx, 0, self.fixture.clone());
            }
            self.app.update(ctx, frame);
            if self.frames == 3 {
                assert!(self.app.preview_navigation().0, "Ctrl K opens launcher");
            }
            if self.frames == 8 {
                assert_eq!(
                    self.app.preview_navigation(),
                    (false, true),
                    "ArrowDown + Enter opens Files"
                );
                println!("PASS keyboard: Ctrl K, ArrowDown, Enter navigation");
            }
            if self.frames == 16 {
                assert!(
                    self.app.preview_plugin_navigation(),
                    "Ctrl K routes installed plugin tool"
                );
                println!("PASS keyboard: plugin search and navigation");
            }
            if self.frames == 24 {
                assert!(
                    self.app.preview_framework_navigation().0,
                    "Ctrl K routes Django SQL"
                );
            }
            if self.frames == 42 {
                let (navigated, completed) = self.app.preview_framework_navigation();
                assert!(
                    navigated && completed,
                    "Ctrl Enter executes diagnostic background task"
                );
                println!(
                    "PASS keyboard: diagnostic search, navigation, Ctrl Enter background completion"
                );
                self.app.preview_catalog_routes();
                self.app.preview_memo_roundtrip();
                self.app.preview_import_routes();
                self.app.preview_instance_start();
                self.app.preview_hidden_panel(ctx, false);
            }
            if self.frames == 70 {
                self.app.preview_instance_results();
            }
            if self.frames >= 90 && std::env::args().nth(3).as_deref() == Some("panel") {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 95 {
                assert!(
                    self.app.preview_hidden(),
                    "Quick panel must keep main workbench hidden"
                );
                assert!(
                    self.app.preview_panel_rendered(),
                    "Hidden workbench wakes and renders child viewport"
                );
                self.app.preview_hidden_panel(ctx, true);
            }
            if self.frames == 145 {
                assert!(self.app.preview_hidden());
                assert!(self.app.preview_panel_rendered());
                println!(
                    "PASS hidden-workbench wake, independent quick panel dark/light, file import routes"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if self.frames == 0 {
            self.app
                .preview_scene(ctx, self.scene, self.fixture.clone());
            let size = if (308..=309).contains(&self.scene)
                || (312..=313).contains(&self.scene)
                || (318..=319).contains(&self.scene)
                || (322..=323).contains(&self.scene)
                || (336..=341).contains(&self.scene)
                || (344..=345).contains(&self.scene)
            {
                egui::vec2(980.0, 640.0)
            } else if (404..=405).contains(&self.scene) || (408..=409).contains(&self.scene) {
                egui::vec2(980.0, 760.0)
            } else if (302..=303).contains(&self.scene) {
                egui::vec2(760.0, 640.0)
            } else if (282..=283).contains(&self.scene) {
                egui::vec2(1440.0, 980.0)
            } else if (284..=285).contains(&self.scene)
                || (244..=245).contains(&self.scene)
                || (250..=251).contains(&self.scene)
                || (254..=255).contains(&self.scene)
                || (274..=279).contains(&self.scene)
            {
                egui::vec2(980.0, 760.0)
            } else if (230..=231).contains(&self.scene)
                || (260..=273).contains(&self.scene)
                || (292..=293).contains(&self.scene)
            {
                egui::vec2(980.0, 640.0)
            } else if (96..=99).contains(&self.scene)
                || (234..=237).contains(&self.scene)
                || (252..=253).contains(&self.scene)
                || (136..=137).contains(&self.scene)
                || (148..=152).contains(&self.scene)
                || (153..=154).contains(&self.scene)
                || (400..=403).contains(&self.scene)
                || (432..=433).contains(&self.scene)
                || (406..=407).contains(&self.scene)
                || matches!(self.scene, 410 | 411 | 414 | 415 | 418 | 419 | 422 | 423)
                || (280..=281).contains(&self.scene)
                || (276..=277).contains(&self.scene)
            {
                egui::vec2(1280.0, 1180.0)
            } else if matches!(
                self.scene,
                412 | 413
                    | 416
                    | 417
                    | 420
                    | 421
                    | 424
                    | 425
                    | 426
                    | 427
                    | 428
                    | 429
                    | 430
                    | 431
                    | 434
                    | 435
            ) {
                egui::vec2(980.0, 760.0)
            } else if (134..=135).contains(&self.scene) {
                egui::vec2(1280.0, 1080.0)
            } else if self.scene == 3
                || self.scene == 7
                || self.scene == 59
                || (188..=189).contains(&self.scene)
                || (204..=207).contains(&self.scene)
                || (228..=229).contains(&self.scene)
            {
                egui::vec2(980.0, 760.0)
            } else {
                egui::vec2(1280.0, 900.0)
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if ((206..=207).contains(&self.scene)
            || (244..=245).contains(&self.scene)
            || (254..=255).contains(&self.scene)
            || (262..=265).contains(&self.scene)
            || (276..=277).contains(&self.scene))
            && self.frames == 6
        {
            self.app.preview_planner_editor();
            self.auto_minimize_smoke_at = Some(Instant::now());
        }
        self.app.update(ctx, frame);
        self.frames += 1;
        let scroll_settled = !((206..=207).contains(&self.scene)
            || (244..=245).contains(&self.scene)
            || (254..=255).contains(&self.scene)
            || (262..=265).contains(&self.scene))
            || self
                .auto_minimize_smoke_at
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(1));
        if self.frames >= 18 && !self.pending && scroll_settled {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.pending = true;
        }
        ctx.request_repaint_after(Duration::from_millis(60));
    }
}

#[cfg(windows)]
fn drag_recorder_region() {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, mouse_event},
        WindowsAndMessaging::FindWindowW,
    };
    let title: Vec<u16> = "选择录制区域 · Esc 取消\0".encode_utf16().collect();
    let deadline = Instant::now() + Duration::from_secs(12);
    while unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) }.is_null() {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    unsafe {
        SetCursorPos(200, 200);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
    }
    for step in 1..=8 {
        unsafe {
            SetCursorPos(200 + step * 60, 200 + step * 35);
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    unsafe {
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
}
#[cfg(windows)]
fn click_quick_recorder_control(stop: bool) {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, mouse_event},
        WindowsAndMessaging::{FindWindowW, GetWindowRect, SetCursorPos},
    };
    let title: Vec<u16> = "Zi DevTools · 快捷面板\0".encode_utf16().collect();
    let window = unsafe { FindWindowW(std::ptr::null_mut(), title.as_ptr()) };
    assert!(!window.is_null(), "快捷面板窗口未创建");
    let mut rect = unsafe { std::mem::zeroed() };
    assert_ne!(unsafe { GetWindowRect(window, &mut rect) }, 0);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let x = rect.left + width * if stop { 87 } else { 69 } / 100;
    let y = rect.top + height * 17 / 100;
    unsafe {
        SetCursorPos(x, y);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
}
#[cfg(not(windows))]
fn click_quick_recorder_control(_stop: bool) {}
#[cfg(not(windows))]
fn drag_recorder_region() {}
fn mp4_boxes<'a>(data: &'a [u8], kind: &[u8; 4]) -> Vec<&'a [u8]> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        if size < 8 || offset + size > data.len() {
            break;
        }
        if &data[offset + 4..offset + 8] == kind {
            result.push(&data[offset + 8..offset + size]);
        }
        offset += size;
    }
    result
}
fn mp4_track_durations(data: &[u8]) -> Vec<([u8; 4], f64)> {
    let mut result = Vec::new();
    for movie in mp4_boxes(data, b"moov") {
        for track in mp4_boxes(movie, b"trak") {
            for media in mp4_boxes(track, b"mdia") {
                let Some(handler) = mp4_boxes(media, b"hdlr").into_iter().next() else {
                    continue;
                };
                let Some(header) = mp4_boxes(media, b"mdhd").into_iter().next() else {
                    continue;
                };
                if handler.len() < 12 || header.len() < 20 {
                    continue;
                }
                let kind = handler[8..12].try_into().unwrap();
                let (timescale, duration) = if header[0] == 1 && header.len() >= 32 {
                    (
                        u32::from_be_bytes(header[20..24].try_into().unwrap()),
                        u64::from_be_bytes(header[24..32].try_into().unwrap()),
                    )
                } else {
                    (
                        u32::from_be_bytes(header[12..16].try_into().unwrap()),
                        u32::from_be_bytes(header[16..20].try_into().unwrap()) as u64,
                    )
                };
                if timescale > 0 {
                    result.push((kind, duration as f64 / timescale as f64));
                }
            }
        }
    }
    result
}
fn main() -> Result<(), eframe::Error> {
    if std::env::args().nth(3).as_deref() == Some("clock-audio-device-smoke") {
        if let Err(error) = zi_devtools::clock::verify_audio_device() {
            eprintln!("AUDIO_DEVICE_FAILED: {error}");
            std::process::exit(2);
        }
        println!(
            "PASS actual default WASAPI render: short tone queued, buffer drained and stream stopped (audibility not verified)"
        );
        return Ok(());
    }
    let folder = PathBuf::from(std::env::args().nth(1).expect("capture output directory"));
    fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let fixture = folder.join("sample.txt");
    fs::write(&fixture, b"abc").unwrap();
    fs::write(folder.join("订单数据.csv"), "name,count\nexample,3\n").unwrap();
    let config = folder.join("services.yml");
    fs::write(&config,format!("state_dir: '{}'\nservices:\n  demo:\n    name: Demo fixture\n    repo: '{}'\n    command: 'echo fixture'\n",folder.join("state").display(),folder.display())).unwrap();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Zi DevTools — UI preview fixture")
        .with_inner_size([1280.0, 900.0]);
    if std::env::args().nth(3).as_deref() == Some("recorder-dynamic-smoke") {
        viewport = viewport.with_position([0.0, 0.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Zi DevTools UI capture",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Capture {
                clock_window_handle: 0,
                app: DevToolsApp::new(cc, config, false),
                folder,
                fixture,
                scene: std::env::args()
                    .nth(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                frames: 0,
                pending: false,
                started: Instant::now(),
                recorder_smoke: None,
                recorder_smoke_started: None,
                recorder_smoke_size: None,
                auto_minimize_smoke_at: None,
                auto_minimize_recording_at: None,
                auto_minimize_smoke_stop_requested: false,
                quick_smoke_phase: 0,
                sqlite_input_frame: None,
                workflow_input_frame: None,
            }))
        }),
    )
}
