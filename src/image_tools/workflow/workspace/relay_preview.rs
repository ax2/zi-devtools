use super::*;
impl Workspace {
    pub(crate) fn preview_relay_instances(&mut self) {
        *self = Self::default();
        self.instances[0].title = "教程原图 A".into();
        self.source = Some(Arc::new(DynamicImage::ImageRgba8(
            image::RgbaImage::from_pixel(20, 16, image::Rgba([210, 30, 40, 120])),
        )));
        let run = execute_budgeted(
            &self.definition,
            self.source.clone().unwrap(),
            &AtomicBool::new(false),
            &self.memory,
        )
        .unwrap();
        assert!(run.failure.is_none());
        self.run = Some(run);
        self.selected = 2;
        self.create().unwrap();
        self.instances[1].title = "接收工作 B".into();
        self.source = Some(Arc::new(DynamicImage::ImageRgba8(
            image::RgbaImage::from_pixel(12, 8, image::Rgba([30, 40, 210, 255])),
        )));
        self.definition.steps = vec![Step::Info { version: 1 }];
        self.select(0).unwrap();
    }
    pub(crate) fn preview_relay_target_id(&self) -> String {
        self.instances[1].id.clone()
    }
    pub(crate) fn preview_relay_check(&self, phase: u8) {
        let a = &self.instances[0].state;
        let b = &self.instances[1].state;
        assert_eq!(
            a.source.as_ref().unwrap().to_rgba8().get_pixel(0, 0).0,
            [210, 30, 40, 120]
        );
        let result = &a.run.as_ref().unwrap().outputs[2].image;
        assert!(a.run.as_ref().unwrap().failure.is_none());
        assert_eq!(b.definition.steps, vec![Step::Info { version: 1 }]);
        assert!(b.run.is_none());
        match phase {
            1 | 2 | 5 => {
                assert_eq!(self.instances.len(), 2);
                assert_eq!(self.active, 0);
                assert_eq!(b.source.as_ref().unwrap().dimensions(), (12, 8));
            }
            3 => {
                assert_eq!(self.instances.len(), 2);
                assert_eq!(self.active, 1);
                assert_eq!(b.source.as_ref().unwrap().to_rgba8(), result.to_rgba8());
                assert!(!b.busy());
            }
            4 => {
                assert_eq!(self.instances.len(), 3);
                assert_eq!(self.active, 2);
                let c = &self.instances[2].state;
                assert_eq!(c.source.as_ref().unwrap().to_rgba8(), result.to_rgba8());
                assert!(c.run.is_none());
                assert!(!c.busy());
                assert_eq!(b.source.as_ref().unwrap().dimensions(), (12, 8));
            }
            _ => panic!("unknown relay phase"),
        }
    }
}

impl Workspace {
    pub(crate) fn preview_budget_fixture(&mut self) {
        *self = Self::default();
        let source = Arc::new(DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
            4096,
            2304,
            image::Rgba([50000u16, 10000, 20000, 60000]),
        )));
        self.memory
            .share(&source, super::super::super::memory::pixels(&source))
            .unwrap();
        self.source = Some(source);
        self.name = "72MiB真实16位图片".into();
    }
    pub(crate) fn preview_budget_check(&self, phase: u8) {
        let expected = if phase <= 2 { 128 } else { 256 };
        assert_eq!(self.memory_limit_mb, expected);
        assert_eq!(
            self.memory.snapshot().unwrap(),
            (72 * 1048576, 0, expected as usize * 1048576)
        );
        assert_eq!(self.source.as_ref().unwrap().dimensions(), (4096, 2304));
        assert_eq!(
            &self.source.as_ref().unwrap().as_bytes()[..8],
            &[80, 195, 16, 39, 32, 78, 96, 234]
        );
        assert!(self.run.is_none());
        if phase != 2 {
            assert!(self.memory_message.is_empty());
        }
        if phase == 2 {
            assert!(self.memory_message.contains("低于当前占用"));
        }
    }
}
