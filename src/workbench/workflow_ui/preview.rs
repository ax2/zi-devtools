//! Isolated native UI fixtures; never included in production builds.
use super::*;

impl DataState {
    pub fn preview_workflow_bookmark_state(&mut self, phase: u8) {
        match phase {
            0 => assert!(self.workflow.files.listing.is_none()),
            1 => assert!(self.workflow.files.review.is_none()),
            _ => panic!("unknown bookmark state phase"),
        }
    }
    pub fn preview_workflow_search_check(&self, phase: u8) {
        let original = Dataset::parse(
            "编号,名称,数量\n001, Zi Tools ,2\n002, Local Notes ,3",
            DataFormat::Csv,
            b',',
        )
        .unwrap();
        assert_eq!(self.dataset.as_ref().unwrap(), &original);
        match phase {
            1 => {
                assert_eq!(
                    self.workflow.files.review.as_ref().unwrap().name,
                    "每日资料清洗"
                );
                assert_eq!(self.workflow.definition.name, "表格清洗");
                assert!(self.workflow.proposal.is_some());
            }
            2 => {
                assert!(self.workflow.files.review.is_none());
                assert_eq!(self.workflow.definition.name, "表格清洗");
                assert!(self.workflow.proposal.is_some());
            }
            3 => {
                assert!(self.workflow.files.review.is_none());
                assert_eq!(self.workflow.definition.name, "每日资料清洗");
                assert!(self.workflow.proposal.is_none());
                assert!(!self.can_undo_transform());
            }
            _ => panic!("unknown workflow search phase"),
        }
    }
    pub fn preview_workflow_memory_position(&self, index: usize) -> egui::Pos2 {
        let rect = self.workflow.files.memory_buttons[index].expect("memory button missing");
        assert!(rect.width() > 0.0 && rect.height() > 0.0);
        rect.center()
    }
    pub fn preview_workflow_memory_check(&self, remembered: bool) -> PathBuf {
        let files = &self.workflow.files;
        let folder = files.folder.clone().unwrap();
        assert_eq!(
            files.remembered_folder,
            remembered.then_some(folder.clone())
        );
        assert_eq!(files.listing.as_ref().unwrap().entries.len(), 2);
        assert!(files.review.is_none());
        assert!(!self.busy());
        assert_eq!(self.dataset.as_ref().unwrap().rows.len(), 2);
        assert_eq!(self.workflow.definition.steps.len(), 2);
        assert!(self.workflow.proposal.is_some());
        folder
    }
    pub fn preview_workflow_inspection(&mut self, phase: u8) {
        assert_eq!(self.dataset.as_ref().unwrap().rows.len(), 4);
        assert_eq!(
            self.workflow.proposal.as_ref().unwrap().result.rows.len(),
            2
        );
        match phase {
            0 => {
                assert!(self.workflow.inspect.open);
                assert!(self.start_workflow().is_err());
                assert!(self.apply_workflow().is_err());
            }
            1 => {
                assert!(self.workflow.inspect.open);
                assert_eq!(self.workflow.inspect.selection(), Some((0, 0)));
            }
            2 => assert!(!self.workflow.inspect.open),
            _ => panic!("unknown inspection phase"),
        }
        assert!(!self.can_undo_transform());
    }
    pub fn preview_workflow_rows(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.workflow.files.review = None;
                self.input = "编号,名称,数量\n001,Zi,2\n002,Zi,3\n003,Other,9\n004,Zip,1".into();
                self.dataset = Some(Dataset::parse(&self.input, DataFormat::Csv, b',').unwrap());
                self.workflow.definition = Definition {
                    version: 2,
                    name: "教程：筛选排序去重".into(),
                    steps: vec![
                        Step::Column {
                            column: "数量".into(),
                            operation: ColumnOperation::ToInteger,
                            value: String::new(),
                        },
                        Step::Filter {
                            column: "名称".into(),
                            predicate: Predicate::Contains,
                            value: "Zi".into(),
                            case_sensitive: true,
                        },
                        Step::Sort {
                            keys: vec![SortKey {
                                column: "数量".into(),
                                descending: true,
                            }],
                        },
                        Step::Deduplicate {
                            columns: vec!["名称".into()],
                        },
                    ],
                };
                self.workflow.invalidate();
                self.show_workflow();
            }
            1 => {
                if self.busy() || self.workflow.proposal.is_none() {
                    return false;
                }
                assert_eq!(self.dataset.as_ref().unwrap().rows.len(), 4);
                let preview = self.workflow.proposal.as_ref().unwrap();
                assert_eq!(preview.steps.len(), 4);
                assert_eq!(
                    preview.result.rows[0],
                    [
                        serde_json::json!("002"),
                        serde_json::json!("Zi"),
                        serde_json::json!(3)
                    ]
                );
                assert_eq!(preview.result.rows.len(), 2);
            }
            2 => {
                assert!(self.workflow.proposal.is_none());
                assert!(self.can_undo_transform());
                assert_eq!(self.dataset.as_ref().unwrap().rows.len(), 2);
                assert_eq!(self.dataset.as_ref().unwrap().rows[0][0], "002");
            }
            3 => {
                assert_eq!(self.dataset.as_ref().unwrap().rows.len(), 4);
                assert_eq!(self.dataset.as_ref().unwrap().rows[0][2], "2");
                assert!(!self.can_undo_transform());
            }
            _ => panic!("unknown row workflow phase"),
        }
        assert!(self.input.contains("001,Zi,2"));
        true
    }
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
