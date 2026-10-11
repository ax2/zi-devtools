//! Checks the actual startup/intake/application routes, without bypassing UI import.
use super::*;
use crate::intake::Target;

impl DevToolsApp {
    pub fn preview_launch_prepare(
        &mut self,
        ctx: &egui::Context,
        args: &crate::launch::Arguments,
        light: bool,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.tool_state.input = "preserved tool draft".into();
        self.tool_state.output = "preserved tool result".into();
        self.data_state.input = "preserved data draft".into();
        self.open_startup_arguments(args).unwrap();
    }

    pub fn preview_launch_pending(&self, paths: &[PathBuf], target: Target) {
        assert_eq!(self.page, Page::Intake);
        assert_eq!(self.intake.paths, paths);
        assert_eq!(self.intake.target, target);
        assert!(!self.intake.busy());
        assert!(self.intake.message.is_empty());
        assert_eq!(self.tool_state.input, "preserved tool draft");
        assert_eq!(self.tool_state.output, "preserved tool result");
        assert_eq!(self.data_state.input, "preserved data draft");
        assert_eq!(self.data_state.instances.len(), 1);
    }

    pub fn preview_launch_applied(&self, path: &Path, target: Target, expected: &str) -> bool {
        if self.page == Page::Intake {
            assert!(self.intake.message.is_empty(), "{}", self.intake.message);
            return false;
        }
        assert!(!self.intake.busy());
        if !matches!(target, Target::Csv | Target::Tsv | Target::JsonData) {
            assert_eq!(self.data_state.instances.len(), 1);
            assert_eq!(self.data_state.input, "preserved data draft");
        }
        match target {
            Target::Files => {
                assert_eq!(self.page, Page::Files);
                assert!(
                    self.file_state
                        .paths
                        .lines()
                        .any(|line| Path::new(line) == path)
                );
                assert_eq!(self.tool_state.input, "preserved tool draft");
            }
            Target::Csv | Target::Tsv | Target::JsonData => {
                assert_eq!(self.page, Page::Data);
                assert_eq!(self.data_state.input, expected);
                assert_eq!(self.data_state.instances.len(), 2);
                assert_eq!(
                    self.data_state.instances[0].state.input,
                    "preserved data draft"
                );
                assert_eq!(self.data_state.active_id(), self.data_state.instances[1].id);
                assert_eq!(self.tool_state.input, "preserved tool draft");
            }
            Target::Json | Target::Java | Target::Python => {
                assert_eq!(
                    self.tool_state.selected,
                    match target {
                        Target::Json => ToolKind::Json,
                        Target::Java => ToolKind::JavaTrace,
                        _ => ToolKind::DjangoTrace,
                    }
                );
                assert_eq!(self.tool_state.input, expected);
                assert!(self.tool_state.output.is_empty());
            }
            target => {
                use crate::framework::Tool;
                self.frameworks.preview_imported_draft(
                    match target {
                        Target::Threads => Tool::Threads,
                        Target::Gc => Tool::Gc,
                        Target::Sql => Tool::Sql,
                        _ => Tool::Celery,
                    },
                    expected,
                );
            }
        }
        true
    }
}
