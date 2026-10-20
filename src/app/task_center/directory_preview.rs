use super::*;
use std::path::Path;

impl DevToolsApp {
    pub fn preview_directory_transfer_prepare(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        light: bool,
    ) {
        self.preview_directory_layout_prepare(ctx, root, light);
        self.data_state.instances.clear();
        self.data_state.create("保留已有工作").unwrap();
        self.data_state.input = "id,value\n001,original".into();
        self.data_state.output = "preserved export".into();
    }
    pub fn preview_directory_transfer_copy(&mut self) {
        self.page = Page::DirectoryCompare;
        self.directory_compare.preview_result_review_plan();
    }
    pub fn preview_directory_transfer_check(&self, phase: u8) {
        assert_eq!(
            self.data_state.instances[0].state.input,
            "id,value\n001,original"
        );
        assert_eq!(
            self.data_state.instances[0].state.output,
            "preserved export"
        );
        let expected_instances = match phase {
            0 | 1 => 1,
            2 | 3 => 2,
            4 => 3,
            _ => panic!("transfer phase"),
        };
        assert_eq!(self.data_state.instances.len(), expected_instances);
        assert_eq!(
            self.directory_compare.preview_transfer_review_rows(),
            match phase {
                1 => Some(205),
                3 => Some(6),
                _ => None,
            }
        );
        if phase == 2 || phase == 4 {
            assert_eq!(self.page, Page::Data);
            let data: crate::workbench::Dataset =
                serde_json::from_str(&self.data_state.input).unwrap();
            assert_eq!(data.rows.len(), if phase == 2 { 205 } else { 6 });
            assert!(
                !self
                    .data_state
                    .autosave_enabled(self.data_state.active_id())
            );
            if phase == 2 {
                assert_eq!(data.rows[0][0], serde_json::json!("file-0000.txt"));
                assert_eq!(data.rows[0][4], serde_json::json!(11));
                assert!(data.rows[0][5].is_null());
                assert_eq!(data.rows[204][0], serde_json::json!("file-0204.txt"));
            } else {
                assert_eq!(data.rows.iter().filter(|r| r[1] == "conflict").count(), 5);
                assert_eq!(data.rows.iter().filter(|r| r[1] == "failed").count(), 1);
                assert!(data.rows.iter().all(|r| r[6] == false));
            }
        } else {
            assert_eq!(self.page, Page::DirectoryCompare);
        }
    }
    pub fn preview_directory_result_review_prepare(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        light: bool,
    ) {
        self.preview_directory_layout_prepare(ctx, root, light);
        self.directory_compare.preview_result_review_plan();
    }
    pub fn preview_directory_result_review_ready(&self) -> bool {
        self.directory_compare.preview_result_review_ready()
    }
    pub fn preview_directory_result_review_check(&self, ctx: &egui::Context, step: u8) {
        assert_eq!(self.page, Page::DirectoryCompare);
        self.directory_compare.preview_result_review_check(step);
        let reported_clip = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("directory-review-clip")))
            .unwrap();
        let page_clip = ctx
            .data(|d| {
                d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("directory-layout-bounds"))
            })
            .unwrap()
            .1;
        let clip = reported_clip
            .intersect(page_clip)
            .intersect(ctx.screen_rect());
        let header = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("directory-review-header")))
            .unwrap();
        println!(
            "review geometry step={step}: header={header:?}, local_clip={reported_clip:?}, page_clip={page_clip:?}, screen={:?}",
            ctx.screen_rect()
        );
        for id in [
            "directory-review-header",
            "directory-review-query",
            "directory-review-all",
            "directory-review-uncopied",
            "directory-review-copied",
            "directory-review-conflicts",
            "directory-review-failed",
            "directory-review-pending",
        ] {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(id)))
                .unwrap();
            assert!(
                clip.contains_rect(rect),
                "review control is clipped: {id}, rect={rect:?}, clip={clip:?}"
            );
        }
        assert_eq!(
            ctx.data(|d| d.get_temp::<bool>(egui::Id::new("directory-consumed-plan-expanded"))),
            Some(false)
        );
        let expected = match step {
            0 => (0, 6, 6),
            1 => (0, 5, 5),
            2 | 4 => (0, 1, 1),
            3 => (200, 205, 205),
            _ => unreachable!(),
        };
        assert_eq!(
            ctx.data(|d| d
                .get_temp::<(usize, usize, usize)>(egui::Id::new("directory-copy-results-range"))),
            Some(expected)
        );
    }

    pub fn preview_directory_pagination_check(&self, ctx: &egui::Context, step: u8) {
        assert_eq!(self.page, Page::DirectoryCompare);
        self.directory_compare.preview_pagination_check(step);
        let expected = match step {
            0 => (0, 100, 205),
            1 => (200, 205, 205),
            2 => (100, 200, 205),
            3 => (0, 1, 1),
            _ => unreachable!(),
        };
        let range = |id: &str| {
            ctx.data(|d| d.get_temp::<(usize, usize, usize)>(egui::Id::new(id)))
                .unwrap()
        };
        assert_eq!(range("directory-results-range"), expected);
        if step >= 2 {
            assert_eq!(range("directory-copy-files-range"), (100, 105, 105));
            assert_eq!(range("directory-copy-directories-range"), (100, 105, 105));
        }
    }

    pub fn preview_directory_layout_prepare(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        light: bool,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.directory_compare
            .preview_copy_fixture(
                &root.join("left"),
                &root.join("right"),
                &root.join("output"),
            )
            .unwrap();
        self.page = Page::DirectoryCompare;
    }

    pub fn preview_directory_layout_check(&self, ctx: &egui::Context, planned: bool) {
        assert_eq!(self.page, Page::DirectoryCompare);
        let (content, clip) = ctx
            .data(|d| {
                d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("directory-layout-bounds"))
            })
            .unwrap();
        assert!(
            content.left() >= clip.left() - 1.0 && content.right() <= clip.right() + 1.0,
            "directory horizontal overflow: content={content:?}, clip={clip:?}"
        );
        let state = self
            .directory_compare
            .preview_copy_state(Path::new("not-selected"));
        assert_eq!(state.1, planned);
        assert!(!state.2 && !state.3);
    }

    pub fn preview_directory_progress_prepare(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        light: bool,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.tasks = Default::default();
        self.directory_compare
            .preview_copy_fixture(
                &root.join("left"),
                &root.join("right"),
                &root.join("output"),
            )
            .unwrap();
        self.directory_compare.preview_copy_progress_start();
        self.page = Page::Tasks;
    }

    pub fn preview_directory_progress_check(&self, running: bool, returned: bool) {
        let snapshot = self.directory_compare.task_snapshot().unwrap();
        assert_eq!(
            snapshot.phase,
            if running {
                Phase::Running
            } else {
                Phase::Cancelled
            }
        );
        assert_eq!(
            self.page,
            if returned {
                Page::DirectoryCompare
            } else {
                Page::Tasks
            }
        );
        assert!(
            self.tasks
                .rows
                .iter()
                .any(|r| r.run_id() == snapshot.run_id() && r.phase == snapshot.phase)
        );
        if running {
            assert!((snapshot.progress.unwrap() - 0.25).abs() < 0.0001);
            assert!(snapshot.summary.contains("已处理 0 / 2 项"));
            assert!(snapshot.summary.contains("已提交 0 B"));
            assert!(snapshot.summary.contains("暂存 64.0 KiB / 128.0 KiB"));
        } else {
            assert!(!self.directory_compare.background_active());
            assert!(snapshot.summary.contains("写入 0 B"));
        }
    }

    pub fn preview_directory_tasks_prepare(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        light: bool,
    ) {
        self.set_theme(ctx, if light { Theme::Light } else { Theme::Dark });
        self.startup_warning = None;
        self.tasks = Default::default();
        self.directory_compare
            .preview_copy_fixture(
                &root.join("left"),
                &root.join("right"),
                &root.join("output"),
            )
            .unwrap();
        self.directory_compare.preview_wait_for_cancel();
        self.page = Page::Tasks;
    }

    pub fn preview_directory_tasks_copy(&mut self) {
        assert_eq!(
            self.directory_compare.task_snapshot().unwrap().phase,
            Phase::Cancelled
        );
        self.directory_compare.preview_retry_plan();
        self.directory_compare.preview_task_execute();
        assert!(self.directory_compare.background_active());
        assert_eq!(self.page, Page::Tasks);
    }

    pub fn preview_directory_tasks_check(&self, phase: Phase, returned: bool) {
        let snapshot = self.directory_compare.task_snapshot().unwrap();
        assert_eq!(snapshot.phase, phase);
        assert!(
            self.tasks
                .rows
                .iter()
                .any(|r| r.run_id() == snapshot.run_id() && r.phase == phase)
        );
        assert_eq!(
            self.page,
            if returned {
                Page::DirectoryCompare
            } else {
                Page::Tasks
            }
        );
        if phase == Phase::Failed {
            assert!(snapshot.summary.contains("成功 1，冲突 1"));
            assert!(
                self.directory_compare
                    .preview_copy_state(Path::new("a.txt"))
                    .3
            );
        }
    }
}
