use super::{
    State, Value,
    plot::{self, Curve, Key},
    plot_render::Bounds,
};
use eframe::egui;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

impl State {
    pub fn preview_plot_fixture(&mut self, ready: bool) {
        *self = Self::default();
        self.plot_mode = true;
        self.expression = "preserve_price=19.90".into();
        self.variables = BTreeMap::from([
            ("ans".into(), Value::Exact(7, 1)),
            ("x".into(), Value::Exact(99, 1)),
        ]);
        self.history = vec![("retained=9".into(), Value::Exact(9, 1))];
        self.plot.saved.x_min = -4.0;
        self.plot.saved.x_max = 4.0;
        self.plot.saved.points = 129;
        self.plot.saved.curves = vec![
            Curve {
                expression: "sin(x)".into(),
                enabled: true,
            },
            Curve {
                expression: "cos(x)".into(),
                enabled: true,
            },
        ];
        if ready {
            self.plot.output = Some(
                plot::sample(
                    Key::new(&self.plot.saved, &self.variables, self.degrees),
                    &AtomicBool::new(false),
                )
                .unwrap(),
            );
        }
    }
    pub fn preview_plot_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = self.plot.controls[index].expect("plot control rendered");
        assert!(
            clip.contains(rect.center()),
            "plot control not visible: {index} / {rect:?} / {clip:?}"
        );
        rect.center()
    }
    pub fn preview_plot_ready(&self) -> bool {
        !self.busy() && self.plot_description().is_ok()
    }
    pub fn preview_plot_check(&self, phase: u8) {
        assert_eq!(self.expression, "preserve_price=19.90");
        assert_eq!(self.variables["ans"], Value::Exact(7, 1));
        assert_eq!(self.variables["x"], Value::Exact(99, 1));
        assert!(!self.variables.contains_key("retained"));
        assert!(!self.variables.contains_key("preserve_price"));
        assert_eq!(self.history.len(), 1);
        assert_eq!(self.plot.saved.x_min, -4.0);
        assert_eq!(self.plot.saved.x_max, 4.0);
        if phase == 1 {
            let view = self.plot.view.unwrap();
            assert!(
                view.x_max - view.x_min < 8.0,
                "native wheel did not zoom {view:?}"
            );
            assert!(
                (view.x_min + view.x_max).abs() > 1e-6,
                "native drag did not pan {view:?}"
            );
        }
        if phase == 2 {
            assert_eq!(self.plot.saved.curves[0].expression, "sin(x)*3");
            assert!(self.plot_csv().is_err());
            assert_eq!(
                self.plot.output.as_ref().unwrap().key.curves[0].expression,
                "sin(x)"
            );
        } else {
            assert!(self.plot_csv().is_ok());
        }
    }
    pub fn preview_plot_svg_bytes(&self) -> Vec<u8> {
        let output = self.plot.output.as_ref().unwrap();
        super::plot_render::svg(
            output,
            self.plot
                .view
                .unwrap_or_else(|| Bounds::fit(output, &self.plot.saved)),
        )
        .into_bytes()
    }
}
