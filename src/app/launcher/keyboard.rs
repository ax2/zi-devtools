//! Keep search shortcuts out of IME candidate selection and modified text keys.
use eframe::egui;
#[derive(Default)]
pub(in crate::app) struct Guard {
    composing: bool,
}
impl Guard {
    pub fn allow_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        ctx.input(|input| {
            let mut changed = false;
            for event in &input.events {
                if let egui::Event::Ime(event) = event {
                    changed = true;
                    self.composing = match event {
                        egui::ImeEvent::Preedit(text) => !text.is_empty(),
                        egui::ImeEvent::Enabled
                        | egui::ImeEvent::Commit(_)
                        | egui::ImeEvent::Disabled => false,
                    };
                }
            }
            // Commit/disable can share a frame with the key that accepted or
            // dismissed the candidate. Only the following frame may navigate.
            !self.composing && !changed
        })
    }
}
pub(in crate::app) fn consume_plain(ctx: &egui::Context, key: egui::Key) -> bool {
    ctx.input_mut(|input| {
        let mut consumed = false;
        input.events.retain(|event| {
            let matches = matches!(event, egui::Event::Key { key: pressed_key, pressed: true, modifiers, .. } if *pressed_key == key && modifiers.is_none());
            consumed |= matches;
            !matches
        });
        consumed
    })
}

/// Keep editing when Enter/Escape is reserved for IME or an invalid selection.
/// Pointer-driven focus changes remain unaffected.
pub(in crate::app) fn preserve_guarded_focus(
    ctx: &egui::Context,
    response: &egui::Response,
    allowed: bool,
) {
    if !allowed
        && response.lost_focus()
        && ctx.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Escape)
        })
    {
        response.request_focus();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composition_spans_frames_and_keeps_commit_and_cancel_keys() {
        let ctx = egui::Context::default();
        let mut guard = Guard::default();
        for event in [
            egui::ImeEvent::Enabled,
            egui::ImeEvent::Preedit("候选".into()),
        ] {
            ctx.input_mut(|i| i.events = vec![egui::Event::Ime(event)]);
            assert!(!guard.allow_shortcuts(&ctx));
        }
        ctx.input_mut(|i| i.events.clear());
        assert!(!guard.allow_shortcuts(&ctx));
        for event in [
            egui::ImeEvent::Commit("图片".into()),
            egui::ImeEvent::Disabled,
        ] {
            ctx.input_mut(|i| i.events = vec![egui::Event::Ime(event)]);
            assert!(!guard.allow_shortcuts(&ctx));
            ctx.input_mut(|i| i.events.clear());
            assert!(guard.allow_shortcuts(&ctx));
        }
    }
    #[test]
    fn idle_enabled_input_method_and_cleared_preedit_do_not_lock_search() {
        let ctx = egui::Context::default();
        let mut guard = Guard::default();
        for event in [
            egui::ImeEvent::Enabled,
            egui::ImeEvent::Preedit(String::new()),
        ] {
            ctx.input_mut(|i| i.events = vec![egui::Event::Ime(event)]);
            assert!(!guard.allow_shortcuts(&ctx));
            ctx.input_mut(|i| i.events.clear());
            assert!(guard.allow_shortcuts(&ctx));
        }
    }
    #[test]
    fn plain_navigation_preserves_modified_text_keys_and_releases() {
        let ctx = egui::Context::default();
        for key in [
            egui::Key::ArrowDown,
            egui::Key::ArrowUp,
            egui::Key::Enter,
            egui::Key::Escape,
        ] {
            for modifiers in [
                egui::Modifiers::NONE,
                egui::Modifiers::SHIFT,
                egui::Modifiers::ALT,
                egui::Modifiers::CTRL,
                egui::Modifiers::COMMAND,
            ] {
                ctx.input_mut(|i| {
                    i.events = vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }]
                });
                assert_eq!(consume_plain(&ctx, key), modifiers.is_none());
                assert_eq!(
                    ctx.input(|i| i.events.len()),
                    usize::from(!modifiers.is_none())
                );
            }
            ctx.input_mut(|i| {
                i.events = vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            });
            assert!(!consume_plain(&ctx, key));
        }
    }
}
