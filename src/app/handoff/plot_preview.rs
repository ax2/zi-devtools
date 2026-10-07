use super::*;
impl DevToolsApp {
    pub fn preview_plot_fixture(&mut self, ready: bool) {
        self.page = Page::Calculator;
        self.calculator.preview_plot_fixture(ready);
        self.data_state.input = "id\nexisting preserved".into();
        self.data_state.output = "old export preserved".into();
    }
    pub fn preview_plot_position(&self, index: usize) -> egui::Pos2 {
        if index == 3 {
            self.calculator.preview_numeric_send.unwrap().center()
        } else if index == 4 {
            self.handoff.as_ref().unwrap().preview_rects[1]
                .unwrap()
                .center()
        } else {
            self.calculator.preview_plot_position(index)
        }
    }
    pub fn preview_plot_ready(&self) -> bool {
        self.calculator.preview_plot_ready()
    }
    pub fn preview_plot_check(&self, phase: u8) {
        self.calculator.preview_plot_check(phase);
    }
    pub fn preview_plot_handoff_check(&self) {
        let transfer = self.handoff.as_ref().unwrap();
        assert_eq!(transfer.target, Target::Csv);
        assert!(transfer.new_data_instance);
        assert!(transfer.numeric.is_none());
        assert_eq!(transfer.text, self.calculator.plot_csv().unwrap());
    }
    pub fn preview_plot_received(&self) -> bool {
        if self.data_state.busy() {
            return false;
        }
        assert!(self.handoff.is_none() && self.page == Page::Data);
        assert_eq!(self.data_state.instances.len(), 2);
        assert_eq!(
            self.data_state.instances[0].state.input,
            "id\nexisting preserved"
        );
        assert_eq!(
            self.data_state.instances[0].state.output,
            "old export preserved"
        );
        let exported = self.data_state.preview_numeric_export();
        let rows: serde_json::Value = serde_json::from_str(&exported).unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 129);
        let mut csv = csv::Reader::from_reader(self.calculator.plot_csv().unwrap().as_bytes());
        for (row, expected) in rows.iter().zip(csv.records()) {
            let expected = expected.unwrap();
            for (col, name) in ["x", "y1", "y2"].iter().enumerate() {
                assert_eq!(row[*name].as_str().unwrap(), &expected[col]);
            }
        }
        self.calculator.preview_plot_check(3);
        true
    }
    pub fn preview_plot_svg(&self) -> Vec<u8> {
        self.calculator.preview_plot_svg_bytes()
    }
}
