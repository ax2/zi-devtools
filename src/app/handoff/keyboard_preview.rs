use super::*;

impl DevToolsApp {
    pub fn preview_handoff_arrows_check(&self, ctx: &egui::Context, phase: u8) {
        let transfer = self
            .handoff
            .as_ref()
            .expect("arrows never receive on their own");
        assert_eq!(self.data_state.instances.len(), 1);
        assert_eq!(self.data_state.input, "preserve existing data draft");
        assert_eq!(self.data_state.output, "preserve existing export");
        assert!(!self.data_state.busy() && !self.launcher_open);
        assert_eq!(self.preferences.recent, ["memos", "data"]);
        assert_eq!(self.preferences.usage["data"], 10);
        assert_eq!(transfer.text, self.tool_state.output);
        let expected = match phase {
            2 | 4 => Target::Csv,
            3 => Target::Memo,
            9 => {
                let recommended: Vec<_> =
                    transfer.recommendations.iter().map(|r| r.target).collect();
                let order = keyboard::ordered(&transfer.matches, &recommended);
                assert!(order.len() > 20);
                let (rect, clip) = ctx
                    .data(|d| {
                        d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new(
                            "handoff-selected-row-proof",
                        ))
                    })
                    .unwrap();
                println!(
                    "ARROW selected={} rect={rect:?} clip={clip:?} scroll={:?}",
                    transfer.target.label(),
                    ctx.data(|d| d.get_temp::<(egui::Rect, f32)>(egui::Id::new(
                        "handoff-target-scroll-proof"
                    )))
                );
                assert!(
                    rect.intersect(clip).height() >= rect.height() * 0.8,
                    "last selected row must scroll into view"
                );
                assert!(ctx.screen_rect().contains(rect.center()));
                *order.last().unwrap()
            }
            _ => Target::Tool(ToolKind::Json),
        };
        assert_eq!(transfer.target, expected, "arrow phase {phase}");
        if matches!(phase, 5 | 9) {
            assert_eq!(transfer.navigation.cursor, Some(expected));
        }
        if matches!(phase, 0 | 1 | 4 | 8) {
            assert!(transfer.navigation.cursor.is_none());
        }
        if phase == 1 {
            assert!(transfer.query.is_empty());
        }
        if phase == 8 {
            assert!(transfer.matches.is_empty());
        }
        if phase == 6 {
            let id = ctx
                .data(|d| d.get_temp::<egui::Id>(egui::Id::new("handoff-apply-focus-proof")))
                .unwrap();
            assert!(
                ctx.memory(|m| m.has_focus(id)),
                "Tab must go directly to confirmation"
            );
        }
        let confirm = self.preview_handoff_discovery_position(1);
        assert!(ctx.screen_rect().contains(confirm));
    }

    pub fn preview_handoff_keyboard_check(
        &self,
        ctx: &egui::Context,
        query: Option<&str>,
        focused: Option<bool>,
        popup: bool,
    ) {
        assert_eq!(
            self.handoff.is_some(),
            query.is_some(),
            "handoff visibility"
        );
        assert!(!self.launcher_open, "handoff must not open global search");
        assert_eq!(egui::Popup::is_any_open(ctx), popup, "category popup");
        assert_eq!(self.page, Page::Data);
        assert_eq!(self.data_state.instances.len(), 1);
        assert_eq!(self.data_state.input, "preserve existing data draft");
        assert_eq!(self.data_state.output, "preserve existing export");
        assert!(!self.data_state.busy());
        assert_eq!(self.preferences.recent, ["memos", "data"]);
        assert_eq!(self.preferences.usage["data"], 10);
        if let Some(query) = query {
            let transfer = self.handoff.as_ref().unwrap();
            assert_eq!(transfer.query, query);
            if query == "JSON 工具" {
                assert_eq!(
                    transfer.matches.first(),
                    Some(&Target::Tool(ToolKind::Json)),
                    "named receiver must lead incidental mentions"
                );
            }
            assert_eq!(transfer.target, Target::Tool(ToolKind::Json));
            assert_eq!(transfer.text, self.tool_state.output);
        }
        if let Some(focused) = focused {
            assert_eq!(
                ctx.memory(|m| m.has_focus(egui::Id::new("handoff-query-input"))),
                focused,
                "search focus"
            );
        }
    }

    pub fn preview_handoff_keyboard_reopen(&mut self) {
        assert!(self.handoff.is_none());
        self.handoff = Some(Transfer::new("合成表格结果".into(), &self.tool_state.output).unwrap());
    }

    pub fn preview_handoff_keyboard_received(&self) {
        assert!(self.handoff.is_none());
        assert_eq!(self.tool_state.selected, ToolKind::Json);
        assert_eq!(self.tool_state.input, "name,count\nZi,2\n工具,3");
        assert!(
            self.tool_state.output.is_empty(),
            "receive does not run JSON"
        );
        assert_eq!(self.data_state.input, "preserve existing data draft");
        assert_eq!(self.data_state.output, "preserve existing export");
        assert_eq!(self.data_state.instances.len(), 1);
        assert_eq!(self.preferences.usage["json"], 1);
        assert_eq!(self.preferences.usage["data"], 10);
        assert_eq!(
            self.preferences.recent.first().map(String::as_str),
            Some("json")
        );
        assert_eq!(Preferences::load(&self.preferences_path).usage["json"], 1);
    }
}
