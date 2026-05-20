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

/// A single-button informational dialog. Used for things like "you're not
/// signed in — here's how to fix it" where there's no choice to make, only
/// an acknowledgement.
#[derive(Debug, Clone)]
pub struct InfoModal {
    pub title: String,
    pub lines: Vec<String>,
}

impl InfoModal {
    pub fn handle_key(&self, key: KeyCode) -> ModalAction {
        match key {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(' ') => {
                ModalAction::Cancelled
            }
            _ => ModalAction::Handled,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let widest = self.lines.iter().map(|l| l.len()).max().unwrap_or(40);
        let want_w = (widest as u16 + 8).max(48);
        let width = want_w.min(area.width.saturating_sub(4)).max(24);
        let height: u16 = (self.lines.len() as u16) + 4;
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let dialog = Rect { x, y, width, height };

        Clear.render(dialog, buf);

        let border_color = theme::ACCENT;
        let title_style = Style::default().fg(border_color).add_modifier(Modifier::BOLD);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(format!(" {} ", self.title), title_style))
            .style(theme::normal());
        let inner = block.inner(dialog);
        block.render(dialog, buf);

        let mut lines: Vec<Line<'static>> = self
            .lines
            .iter()
            .map(|s| Line::from(Span::styled(s.clone(), theme::normal())))
            .collect();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Enter/Esc ", theme::accent()),
            Span::styled("close", theme::dim()),
        ]));
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(theme::normal())
            .render(inner, buf);
    }
}

/// State of the in-TUI OIDC login flow dialog.
pub enum OidcLoginState {
    /// Contacting the server to obtain an authorization URL.
    Initiating,
    /// URL is ready; waiting for the user to complete sign-in in their browser.
    AwaitingBrowser { authorize_url: String, started: std::time::Instant },
    /// Poll timed out before the browser flow completed.
    TimedOut,
    /// Flow failed with an error message.
    Failed { error: String },
}

/// An in-TUI OIDC login dialog. Displayed when the user presses `L` while
/// the server is reachable but they have no valid session.
pub struct OidcLoginModal {
    pub state: OidcLoginState,
}

impl OidcLoginModal {
    pub fn handle_key(&self, key: KeyCode) -> ModalAction {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => ModalAction::Cancelled,
            _ => ModalAction::Handled,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let width: u16 = 72_u16.min(area.width.saturating_sub(4)).max(40);
        let height: u16 = match &self.state {
            OidcLoginState::Initiating => 5,
            OidcLoginState::AwaitingBrowser { .. } => 9,
            OidcLoginState::TimedOut => 7,
            OidcLoginState::Failed { .. } => 7,
        };
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let dialog = Rect { x, y, width, height };

        Clear.render(dialog, buf);

        let title_style = Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::ACCENT))
            .title(Span::styled(" Sign in to MissionControl ", title_style))
            .style(theme::normal());
        let inner = block.inner(dialog);
        block.render(dialog, buf);

        let lines: Vec<Line<'static>> = match &self.state {
            OidcLoginState::Initiating => vec![
                Line::from(""),
                Line::from(Span::styled("Connecting to server...", theme::muted())),
                Line::from(""),
            ],
            OidcLoginState::AwaitingBrowser { authorize_url, started } => {
                let elapsed = started.elapsed().as_secs();
                // Truncate URL to fit inside the dialog border (inner width = width - 2)
                let max_url = (width.saturating_sub(2)) as usize;
                let url_display = if authorize_url.len() > max_url {
                    format!("{}…", &authorize_url[..max_url.saturating_sub(1)])
                } else {
                    authorize_url.clone()
                };
                vec![
                    Line::from(""),
                    Line::from(Span::styled("Open this URL in your browser:", theme::dim())),
                    Line::from(Span::styled(url_display, Style::default().fg(ratatui::style::Color::Cyan))),
                    Line::from(""),
                    Line::from(Span::styled(
                        format!("Waiting for authentication... ({elapsed}s)"),
                        theme::muted(),
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("  esc ", theme::accent()),
                        Span::styled("cancel", theme::dim()),
                    ]),
                ]
            }
            OidcLoginState::TimedOut => vec![
                Line::from(""),
                Line::from(Span::styled("Browser auth timed out.", theme::err())),
                Line::from(""),
                Line::from(Span::styled(
                    "Visit the URL above and complete sign-in,",
                    theme::muted(),
                )),
                Line::from(Span::styled("then press R to retry.", theme::muted())),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  esc ", theme::accent()),
                    Span::styled("close", theme::dim()),
                ]),
            ],
            OidcLoginState::Failed { error } => {
                let max_err = (width.saturating_sub(4)) as usize;
                let err_display = if error.len() > max_err {
                    format!("{}…", &error[..max_err.saturating_sub(1)])
                } else {
                    error.clone()
                };
                vec![
                    Line::from(""),
                    Line::from(Span::styled("Authentication failed:", theme::err())),
                    Line::from(Span::styled(err_display, theme::muted())),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("  esc ", theme::accent()),
                        Span::styled("close", theme::dim()),
                    ]),
                ]
            }
        };

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
