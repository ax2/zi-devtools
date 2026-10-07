use super::*;
impl DevToolsApp {
    pub fn preview_date_handoff_position(&self, send: bool) -> egui::Pos2 {
        if send {
            self.calculator.preview_date_send_position()
        } else {
            self.handoff.as_ref().unwrap().preview_rects[1]
                .unwrap()
                .center()
        }
    }
    pub fn preview_date_handoff_action(&mut self, phase: u8) {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        match phase {
            0 => self.planner.preview(false, false),
            1 => {
                let transfer = self.handoff.as_ref().expect("date snapshot confirmation");
                assert_eq!(transfer.event_date, Some(day));
                assert_eq!(transfer.target, Target::Event);
                assert!(transfer.text.contains("3 工作日") && transfer.text.contains("不含节假日"));
                assert!(!transfer.source.is_empty());
                assert_eq!(self.calculator.preview_date_snapshot().0, day);
            }
            2 => {
                assert!(self.handoff.is_none() && self.page == Page::Calendar);
                let (date, text) = self.calculator.preview_date_snapshot();
                self.planner.preview_date_received(date, &text);
                self.calculator.preview_date_check(1);
                let transfer = Transfer::dated(
                    "second calculation".into(),
                    day.succ_opt().unwrap(),
                    "do not replace",
                )
                .unwrap();
                self.handoff = Some(transfer);
                self.apply_handoff();
                assert!(self.handoff.as_ref().unwrap().error.contains("当前编辑"));
                self.planner.preview_date_received(date, &text);
                self.handoff = None;
                println!(
                    "PASS native date send/confirmation/all-day draft: exact calculated day, full conditions, reminder off, no database write, source ans/history preserved, dirty target refused"
                );
            }
            _ => panic!("unknown date handoff phase"),
        }
    }
}
