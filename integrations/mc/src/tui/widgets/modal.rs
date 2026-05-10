use crossterm::event::KeyCode;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use super::theme;

/// Outcome of a key event delivered to a modal.
#[derive(Debug, Clone, PartialEq)]
pub enum ModalAction {
    /// Modal didn't recognize the key; let other handlers see it.
    Passthrough,
    /// Modal handled the key, no further action.
    Handled,
    /// User pressed y/Enter — caller should run the bound action and close the modal.
    Confirmed,
    /// User pressed n/Esc — caller should close the modal without running the action.
    Cancelled,
}

/// A two-button confirmation dialog used for destructive actions.
///
/// `danger=true` paints the border and confirm key red; `danger=false` keeps
/// the orange accent for non-destructive but still-need-an-extra-tap flows.
#[derive(Debug, Clone)]
pub struct ConfirmModal {
    pub title: String,
    pub message: String,
    pub danger: bool,
}

impl ConfirmModal {
    pub fn handle_key(&self, key: KeyCode) -> ModalAction {
        match key {
            KeyCode::Char('y') | KeyCode::Enter => ModalAction::Confirmed,
            KeyCode::Char('n') | KeyCode::Esc => ModalAction::Cancelled,
            // Swallow everything else so arrow keys/tabs don't leak past an
            // open dialog and trigger nav side-effects.
            _ => ModalAction::Handled,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        // Sized to fit the message with reasonable padding, clamped to area.
        let want_w = (self.message.len() as u16 + 8).max(40);
        let width = want_w.min(area.width.saturating_sub(4)).max(20);
        let height: u16 = 5;
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let dialog = Rect { x, y, width, height };

        Clear.render(dialog, buf);

        let border_color = if self.danger { theme::ERR } else { theme::ACCENT };
        let title_style = Style::default().fg(border_color).add_modifier(Modifier::BOLD);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(format!(" {} ", self.title), title_style))
            .style(theme::normal());
        let inner = block.inner(dialog);
        block.render(dialog, buf);

        let confirm_style = if self.danger { theme::err() } else { theme::accent() };
        let lines = vec![
            Line::from(Span::styled(self.message.clone(), theme::normal())),
            Line::from(""),
            Line::from(vec![
                Span::styled("  y ", confirm_style),
                Span::styled("confirm  ", theme::dim()),
                Span::styled("  n ", theme::ok()),
                Span::styled("cancel  ", theme::dim()),
                Span::styled("  esc ", theme::muted()),
                Span::styled("cancel", theme::dim()),
            ]),
        ];
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(theme::normal())
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modal() -> ConfirmModal {
        ConfirmModal { title: "T".into(), message: "M".into(), danger: true }
    }

    #[test]
    fn y_confirms() { assert_eq!(modal().handle_key(KeyCode::Char('y')), ModalAction::Confirmed); }
    #[test]
    fn enter_confirms() { assert_eq!(modal().handle_key(KeyCode::Enter), ModalAction::Confirmed); }
    #[test]
    fn n_cancels() { assert_eq!(modal().handle_key(KeyCode::Char('n')), ModalAction::Cancelled); }
    #[test]
    fn esc_cancels() { assert_eq!(modal().handle_key(KeyCode::Esc), ModalAction::Cancelled); }
    #[test]
    fn other_keys_swallowed_not_passed_through() {
        assert_eq!(modal().handle_key(KeyCode::Down), ModalAction::Handled);
        assert_eq!(modal().handle_key(KeyCode::Char('q')), ModalAction::Handled);
    }
}
