use super::*;
use crate::tasks::Phase;
impl Workspace {
    pub(crate) fn preview_tasks_fixture(&mut self, ctx: &egui::Context) {
        *self = Default::default();
        self.instances[0].title = "教程图 A".into();
        let first = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            20,
            16,
            image::Rgba([100, 50, 150, 100]),
        )));
        self.source = Some(first.clone());
        let def = Definition::default();
        self.launch(ctx, Kind::Run, move |cancel| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !cancel.load(Ordering::Relaxed) {
                ensure!(std::time::Instant::now() < deadline, "fixture timeout");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(Reply::Run(execute(&def, first, cancel)?))
        });
        self.create().unwrap();
        self.instances[1].title = "教程图 B".into();
        let image = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            24,
            10,
            image::Rgba([20, 100, 200, 200]),
        )));
        self.source = Some(image.clone());
        let def = self.definition.clone();
        self.launch(ctx, Kind::Run, move |cancel| {
            Ok(Reply::Run(execute(&def, image, cancel)?))
        });
    }
    pub(crate) fn preview_tasks_ready(&self) -> bool {
        self.snapshots().len() == 2 && self.snapshots().iter().all(|r| !r.phase.active())
    }
    pub(crate) fn preview_tasks_check(&self, phase: u8) {
        if phase == 3 {
            assert_eq!(self.instances.len(), 1);
            assert_eq!(self.instances[0].title, "教程图 B");
            return;
        }
        assert_eq!(self.instances[0].state.job.phase, Phase::Cancelled);
        assert_eq!(self.instances[1].state.job.phase, Phase::Done);
        assert_eq!(
            self.instances[0]
                .state
                .source
                .as_ref()
                .unwrap()
                .dimensions(),
            (20, 16)
        );
        assert_eq!(
            self.instances[1].state.run.as_ref().unwrap().outputs[1]
                .image
                .dimensions(),
            (24, 10)
        );
        assert_eq!(self.active, if phase == 1 { 1 } else { 0 });
    }
}
