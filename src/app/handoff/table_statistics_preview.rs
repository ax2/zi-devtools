use super::*;
impl DevToolsApp {
    pub fn preview_table_statistic_fixture(&mut self, ready: bool) {
        self.page = Page::Calculator;
        self.calculator = Default::default();
        self.calculator.expression = "preserve_price=19.90".into();
        self.calculator.preview_statistic_state(ready);
        if !ready {
            let (_, table) = self.data_state.preview_statistic_source();
            let mut transfer = Transfer::numeric("合成8×8已选区域".into(), table).unwrap();
            transfer.target = Target::Calculator;
            self.handoff = Some(transfer);
        }
    }
    pub fn preview_table_statistic_position(&self, index: usize) -> egui::Pos2 {
        match index {
            0 | 4 => self.handoff.as_ref().unwrap().preview_rects[1]
                .unwrap()
                .center(),
            1 => self.handoff.as_ref().unwrap().matrix_rects[2]
                .unwrap()
                .center(),
            2 => self.calculator.preview_statistic_operation_position(0),
            3 => self.calculator.preview_statistic_operation_position(1),
            7 => self.calculator.preview_statistic_operation_position(2),
            5 => self.calculator.preview_matrix_position(),
            6 => self.calculator.preview_numeric_send.unwrap().center(),
            _ => panic!("statistic control"),
        }
    }
    pub fn preview_table_statistic_check(&self, phase: u8) {
        assert_eq!(self.calculator.expression, "preserve_price=19.90");
        self.calculator.preview_statistic_state_check(phase >= 3);
        if phase == 0 {
            assert!(self.handoff.as_ref().unwrap().error.contains("未保存"));
        }
        if phase == 1 {
            assert!(self.handoff.is_none() && self.page == Page::Calculator);
        }
        if phase == 4 {
            let transfer = self.handoff.as_ref().unwrap();
            assert_eq!(transfer.target, Target::JsonData);
            assert_eq!(transfer.representation, Representation::Typed);
            assert!(transfer.new_data_instance);
            assert_eq!(
                transfer.numeric.as_ref().unwrap().cells,
                vec![crate::calculator::Value::Exact(65, 2)]
            );
        }
    }
    pub fn preview_table_statistic_received(&self) -> bool {
        if self.data_state.busy() {
            return false;
        }
        assert!(self.handoff.is_none() && self.page == Page::Data);
        assert_eq!(self.data_state.instances.len(), 2);
        let old = &self.data_state.instances[0].state;
        assert!(old.input.starts_with("列1,列2,列3"));
        assert_eq!(old.output, "preserved export");
        let parsed: serde_json::Value =
            serde_json::from_str(&self.data_state.preview_numeric_export()).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([{"c1":{"kind":"exact","numerator":"65","denominator":"2"}}])
        );
        self.calculator.preview_statistic_state_check(true);
        true
    }
}
