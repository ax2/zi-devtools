//! Isolated native UI fixtures; never included in production builds.
use super::*;

impl DataState {
    pub fn preview_workflow_empty(&mut self, review: bool) {
        if review {
            self.preview_workflow_import();
        } else {
            self.preview_workflow();
            self.workflow.definition.steps[1] = Step::SelectColumns {
                columns: vec!["数量".into()],
            };
        }
        self.format = DataFormat::Csv;
        self.dataset = None;
        self.workflow.invalidate();
        self.show_workflow();
    }

    pub fn preview_workflow_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = self.workflow.buttons[index].expect("workflow button rendered");
        assert!(
            clip.contains(rect.center()),
            "workflow button outside visible viewport"
        );
        rect.center()
    }

    pub fn preview_workflow_check(&mut self, phase: u8) -> bool {
        assert!(
            self.input.contains("001, Zi Tools ,2"),
            "original input retained"
        );
        match phase {
            0 => self.preview_workflow_empty(true),
            1 => {
                assert!(self.workflow.files.review.is_none());
                assert!(self.dataset.is_none());
                assert_eq!(self.workflow.definition.steps.len(), 2);
                assert!(self.workflow.proposal.is_none());
                self.preview_workflow_empty(true);
            }
            2 => {
                assert!(self.workflow.files.review.is_none());
                assert_eq!(self.workflow.definition.name, "每日资料整理");
                assert_eq!(self.workflow.definition.steps.len(), 3);
                assert!(self.workflow.proposal.is_none());
                assert!(self.dataset.is_none());
                self.parse();
                self.show_workflow();
            }
            3 => {
                if self.busy() || self.workflow.proposal.is_none() {
                    return false;
                }
                assert_eq!(self.workflow.job.phase, Phase::Done);
                let data = self.dataset.as_ref().unwrap();
                assert_eq!(data.rows[0][1], " Zi Tools ");
                assert_eq!(data.rows[0][2], "2");
            }
            4 => {
                let data = self.dataset.as_ref().unwrap();
                assert_eq!(data.rows[0][1], "Zi Tools");
                assert_eq!(data.rows[0][2], serde_json::json!(2));
                assert!(self.workflow.proposal.is_none());
                assert!(self.can_undo_transform());
                self.show_workflow();
            }
            5 => {
                let data = self.dataset.as_ref().unwrap();
                assert_eq!(data.rows[0][1], " Zi Tools ");
                assert_eq!(data.rows[0][2], "2");
                assert!(!self.can_undo_transform());
                assert!(self.workflow.proposal.is_none());
            }
            _ => panic!("invalid workflow smoke phase"),
        }
        true
    }
}
