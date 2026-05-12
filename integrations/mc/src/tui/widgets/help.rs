use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use super::theme;

/// A row in the help overlay — one key/chord and its description.
pub struct HelpEntry {
    pub keys: &'static str,
    pub desc: &'static str,
}

/// Centered help overlay listing keybindings for the current screen.
/// Rendered on top of everything else; dismissed by any key.
pub struct HelpOverlay<'a> {
    pub title: &'a str,
    pub entries: &'a [HelpEntry],
    pub global: &'a [HelpEntry],
}

impl<'a> Widget for HelpOverlay<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let want_w = 56u16;
        let need_h = (self.entries.len() + self.global.len()) as u16 + 7;
        let width = want_w.min(area.width.saturating_sub(4)).max(40);
        let height = need_h.min(area.height.saturating_sub(2));
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let dialog = Rect { x, y, width, height };

        Clear.render(dialog, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::border_focused())
            .title(Span::styled(format!(" Help · {} ", self.title), theme::accent_bold()))
            .style(theme::normal());
        let inner = block.inner(dialog);
        block.render(dialog, buf);

        let mut lines: Vec<Line> = Vec::with_capacity(need_h as usize);
        for e in self.entries {
            lines.push(entry_line(e.keys, e.desc));
        }
        if !self.global.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Global",
                Style::default().fg(theme::TEXT_MUTED).add_modifier(Modifier::BOLD),
            )));
            for e in self.global {
                lines.push(entry_line(e.keys, e.desc));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("press any key to close", theme::dim())));

        Paragraph::new(lines).style(theme::normal()).render(inner, buf);
    }
}

fn entry_line(keys: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<12}", keys), theme::accent()),
        Span::styled(desc.to_string(), theme::normal()),
    ])
}

/// Default global keybindings shown in every Help overlay.
pub const GLOBAL_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "Tab/S+Tab", desc: "next/prev tab" },
    HelpEntry { keys: "a m f p s c", desc: "jump to Agents/Missions/Feed/Approvals/Secrets/Config" },
    HelpEntry { keys: "L", desc: "identity / sign-in instructions" },
    HelpEntry { keys: "R", desc: "refresh panel + re-check session on disk" },
    HelpEntry { keys: "?", desc: "toggle this help" },
    HelpEntry { keys: "Ctrl+Q / Ctrl+C", desc: "quit" },
];
