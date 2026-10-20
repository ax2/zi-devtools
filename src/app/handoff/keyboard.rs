use super::Target;
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Next,
    Previous,
    FocusConfirmation,
}

#[derive(Default)]
pub(super) struct Navigation {
    pub cursor: Option<Target>,
    ime_active: bool,
}

fn consume_plain(input: &mut egui::InputState, key: egui::Key) -> bool {
    // egui's logical shortcut matching ignores extra Shift/Alt. Preserve those
    // events for text selection and reverse focus traversal instead.
    let mut consumed = false;
    input.events.retain(|event| {
        let matches = matches!(event, egui::Event::Key {key:pressed_key,pressed:true,modifiers,..} if *pressed_key==key && modifiers.is_none());
        consumed |= matches;
        !matches
    });
    consumed
}

impl Navigation {
    /// Inspect only the focused search field. IME events and its intervening
    /// composition frames keep their arrows/Tab; Enter is never a relay action.
    pub fn input(&mut self, ctx: &egui::Context, enabled: bool) -> Option<Action> {
        if !enabled {
            self.ime_active = false;
            return None;
        }
        ctx.input_mut(|input| {
            let mut ime_event = false;
            for event in &input.events {
                if let egui::Event::Ime(event) = event {
                    ime_event = true;
                    self.ime_active =
                        matches!(event, egui::ImeEvent::Enabled | egui::ImeEvent::Preedit(_));
                }
            }
            if self.ime_active || ime_event {
                return None;
            }
            if consume_plain(input, egui::Key::ArrowDown) {
                Some(Action::Next)
            } else if consume_plain(input, egui::Key::ArrowUp) {
                Some(Action::Previous)
            } else if self.cursor.is_some() && consume_plain(input, egui::Key::Tab) {
                Some(Action::FocusConfirmation)
            } else {
                None
            }
        })
    }

    pub fn step(&mut self, ordered: &[Target], backwards: bool) -> Option<Target> {
        if ordered.is_empty() {
            self.cursor = None;
            return None;
        }
        let last = ordered.len() - 1;
        let index = match self
            .cursor
            .and_then(|target| ordered.iter().position(|item| *item == target))
        {
            Some(index) if backwards => index.saturating_sub(1),
            Some(index) => (index + 1).min(last),
            None if backwards => last,
            None => 0,
        };
        self.cursor = Some(ordered[index]);
        self.cursor
    }
}

/// Use the exact displayed sequence: compatible recommendations, then matches.
pub(super) fn ordered(targets: &[Target], recommendations: &[Target]) -> Vec<Target> {
    let mut ordered = Vec::with_capacity(targets.len());
    for target in recommendations.iter().chain(targets) {
        if targets.contains(target) && !ordered.contains(target) {
            ordered.push(*target);
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_order_deduplicates_and_excludes_unavailable_recommendations() {
        assert_eq!(
            ordered(
                &[Target::Memo, Target::Csv, Target::Tsv],
                &[Target::Csv, Target::Csv, Target::Event]
            ),
            [Target::Csv, Target::Memo, Target::Tsv]
        );
        assert_eq!(
            ordered(&[Target::Tsv, Target::Csv], &[]),
            [Target::Tsv, Target::Csv]
        );
    }

    #[test]
    fn arrows_clamp_and_recover_from_empty_or_changed_results() {
        let mut nav = Navigation::default();
        let order = [Target::Csv, Target::Memo, Target::Event];
        assert_eq!(nav.step(&order, false), Some(Target::Csv));
        assert_eq!(nav.step(&order, false), Some(Target::Memo));
        assert_eq!(nav.step(&order, false), Some(Target::Event));
        assert_eq!(nav.step(&order, false), Some(Target::Event));
        assert_eq!(nav.step(&order, true), Some(Target::Memo));
        assert_eq!(nav.step(&[Target::Tsv], true), Some(Target::Tsv));
        assert_eq!(nav.step(&[], false), None);
        assert_eq!(nav.step(&order, true), Some(Target::Event));
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn only_focused_non_composing_search_consumes_navigation_keys() {
        let ctx = egui::Context::default();
        let mut nav = Navigation::default();
        let down = || key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        ctx.input_mut(|i| i.events = vec![down()]);
        assert_eq!(nav.input(&ctx, false), None);
        assert!(ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)));
        assert_eq!(nav.input(&ctx, true), Some(Action::Next));
        ctx.input_mut(|i| {
            i.events = vec![
                egui::Event::Ime(egui::ImeEvent::Preedit("测试".into())),
                down(),
            ]
        });
        assert_eq!(nav.input(&ctx, true), None);
        ctx.input_mut(|i| i.events = vec![down()]);
        assert_eq!(nav.input(&ctx, true), None);
        ctx.input_mut(|i| {
            i.events = vec![
                egui::Event::Ime(egui::ImeEvent::Commit("测试".into())),
                down(),
            ]
        });
        assert_eq!(nav.input(&ctx, true), None);
        ctx.input_mut(|i| i.events = vec![down()]);
        assert_eq!(nav.input(&ctx, true), Some(Action::Next));
        for modifiers in [
            egui::Modifiers::CTRL,
            egui::Modifiers::SHIFT,
            egui::Modifiers::ALT,
        ] {
            ctx.input_mut(|i| i.events = vec![key(egui::Key::ArrowDown, modifiers)]);
            assert_eq!(nav.input(&ctx, true), None);
            assert!(ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)));
        }
        ctx.input_mut(|i| i.events = vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        assert_eq!(nav.input(&ctx, true), None);
        nav.cursor = Some(Target::Csv);
        assert_eq!(nav.input(&ctx, true), Some(Action::FocusConfirmation));
        ctx.input_mut(|i| i.events = vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)]);
        assert_eq!(nav.input(&ctx, true), None);
        ctx.input_mut(|i| i.events = vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(nav.input(&ctx, true), None);
    }
}
