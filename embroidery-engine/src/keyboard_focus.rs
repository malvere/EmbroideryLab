//! Keyboard shortcuts belong to the focused editor, not every focused widget.
use eframe::egui;

pub(super) fn text_input_active(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.focused())
        .is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_text_state_does_not_claim_a_focused_button() {
        let ctx = egui::Context::default();
        let text = egui::Id::new("editor");
        let button = egui::Id::new("button");
        egui::text_edit::TextEditState::default().store(&ctx, text);
        ctx.memory_mut(|memory| memory.request_focus(text));
        assert!(text_input_active(&ctx));
        ctx.memory_mut(|memory| memory.request_focus(button));
        assert!(!text_input_active(&ctx));
        assert!(egui::TextEdit::load_state(&ctx, text).is_some());
    }
}
