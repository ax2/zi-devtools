use super::*;

impl DevToolsApp {
    pub fn preview_boundary_scene(&mut self) {
        self.preview_boundary_fixture();
        let table = self.data_state.preview_boundary_request();
        let mut transfer = Transfer::numeric("8×8数值边界合成夹具".into(), table).unwrap();
        transfer.target = Target::Calculator;
        transfer.matrix_slot = MatrixSlot::B;
        transfer.refresh_numeric();
        self.handoff = Some(transfer);
        self.apply_handoff();
        assert!(self.handoff.as_ref().unwrap().error.contains("未保存"));
    }
    pub fn preview_boundary_fixture(&mut self) {
        self.preview_mapping_scene(false);
        self.data_state.preview_numeric_boundary_fixture();
    }
    pub fn preview_boundary_position(&self, index: usize) -> egui::Pos2 {
        self.data_state.preview_numeric_boundary_position(index)
    }
    pub fn preview_boundary_check(&self, phase: u8) {
        self.data_state.preview_numeric_boundary_order(phase >= 1);
        assert_eq!(self.calculator.expression, "preserve_price=19.90");
        if phase == 2 {
            assert!(self.handoff.is_none() && self.page == Page::Calculator);
            self.calculator.preview_boundary_check();
        }
    }
    pub fn preview_mapping_scene(&mut self, modal: bool) {
        self.page = Page::Data;
        self.calculator = Default::default();
        self.calculator.expression = "preserve_price=19.90".into();
        self.data_state.preview_mapping_fixture();
        if modal {
            let (source, table) = self.data_state.preview_mapping_request();
            let mut transfer = Transfer::numeric(source, table).unwrap();
            transfer.target = Target::Calculator;
            transfer.matrix_slot = MatrixSlot::B;
            self.handoff = Some(transfer);
        }
    }
    pub fn preview_mapping_position(&self, index: usize) -> egui::Pos2 {
        if index == 4 || index == 5 {
            return self.data_state.preview_numeric_entry_position(index == 5);
        }
        if index == 0 {
            self.data_state.preview_mapping_position()
        } else {
            let transfer = self.handoff.as_ref().unwrap();
            match index {
                1 => transfer.matrix_rects[1].unwrap().center(),
                2 => transfer.matrix_rects[2].unwrap().center(),
                3 => transfer.preview_rects[1].unwrap().center(),
                _ => panic!("mapping control"),
            }
        }
    }
    pub fn preview_numeric_entry_cancelled(&self) {
        self.data_state.preview_numeric_entry_cancelled();
        assert!(self.handoff.is_none());
        assert_eq!(self.calculator.expression, "preserve_price=19.90");
    }
    pub fn preview_mapping_check(&self, received: bool) {
        assert_eq!(self.data_state.output, "preserved export");
        assert_eq!(self.calculator.expression, "preserve_price=19.90");
        if !received {
            assert!(self.handoff.as_ref().unwrap().error.contains("未保存"));
        } else {
            assert!(self.handoff.is_none() && self.page == Page::Calculator);
            self.calculator.preview_mapping_check();
        }
    }
    pub fn preview_numeric_position(&self, index: usize) -> egui::Pos2 {
        if index == 4 {
            return self.calculator.preview_numeric_send.unwrap().center();
        }
        let transfer = self.handoff.as_ref().unwrap();
        if index == 3 {
            transfer.preview_rects[1].unwrap().center()
        } else {
            transfer.numeric_mode_rects[index].unwrap().center()
        }
    }
    pub fn preview_numeric_scene(&mut self, received: bool) {
        self.calculator.preview_numeric_fixture();
        self.page = Page::Calculator;
        let snapshot = self.calculator.numeric_result().unwrap();
        if received {
            self.calculator = Default::default();
            self.calculator
                .receive_numeric(&snapshot.json(Representation::Typed).unwrap())
                .unwrap();
        } else {
            self.handoff =
                Some(Transfer::numeric("计算器矩阵快照 · v0.8.0".into(), snapshot).unwrap());
        }
    }
    pub fn preview_numeric_smoke(&mut self, phase: u8) {
        match phase {
            0 => {
                self.handoff = None;
                self.page = Page::Calculator;
                self.calculator.preview_numeric_fixture();
                self.data_state.input = "id\nexisting preserved".into();
                self.data_state.output = "old export preserved".into();
            }
            1 => {
                let transfer = self.handoff.as_ref().unwrap();
                assert_eq!(transfer.representation, Representation::Text);
                assert!(transfer.text.contains(&i128::MAX.to_string()));
                assert!(NumericTable::read_json(&transfer.text).is_err());
            }
            2 => {
                let transfer = self.handoff.as_ref().unwrap();
                assert_eq!(transfer.representation, Representation::Approximate);
                let value: serde_json::Value = serde_json::from_str(&transfer.text).unwrap();
                assert!(value[0]["c1"].is_number());
            }
            3 => {
                let transfer = self.handoff.as_ref().unwrap();
                assert_eq!(transfer.representation, Representation::Typed);
                let table = NumericTable::read_json(&transfer.text).unwrap();
                assert_eq!(table, self.calculator.numeric_result().unwrap());
            }
            4 => {
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
                let text = self.data_state.preview_numeric_export();
                let table = NumericTable::read_json(&text).unwrap();
                assert_eq!(table, self.calculator.numeric_result().unwrap());
                self.data_state.output = text.clone();
                let mut transfer = Transfer::new("数据工作台导出 · v1.2.0".into(), &text).unwrap();
                transfer.target = Target::Calculator;
                self.handoff = Some(transfer);
                self.apply_handoff();
                assert!(self.handoff.as_ref().unwrap().error.contains("未保存"));
                assert_eq!(table, self.calculator.numeric_result().unwrap());
                // Fixture simulates a separate clean calculator worksheet; never discards user work.
                self.calculator = Default::default();
                self.handoff.as_mut().unwrap().error.clear();
            }
            5 => {
                assert!(self.handoff.is_none() && self.page == Page::Calculator);
                assert!(self.calculator.dirty() && self.calculator.numeric_result().is_err());
                assert_eq!(self.calculator.expression, "0.1 + 0.2");
                self.calculator.preview_numeric_compute();
                let table = self.calculator.numeric_result().unwrap();
                assert_eq!(table.cells[0], crate::calculator::Value::Exact(1, 3));
                assert_eq!(
                    table.cells[1],
                    crate::calculator::Value::Exact(i128::MAX, 1)
                );
                assert!(
                    matches!(table.cells[2],crate::calculator::Value::Approx(x) if x.to_bits()==1)
                );
                assert!(
                    matches!(table.cells[3],crate::calculator::Value::Approx(x) if x.to_bits()==(-0.0f64).to_bits())
                );
                assert_eq!(
                    self.data_state.instances[0].state.output,
                    "old export preserved"
                );
            }
            _ => panic!("numeric smoke phase"),
        }
    }
}
