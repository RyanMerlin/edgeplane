use ratatui::style::{Color, Modifier, Style};

// Dark background, rust orange accent (matches clix palette)
pub const BG: Color = Color::Rgb(13, 17, 23);
pub const PANEL_BORDER: Color = Color::Rgb(60, 60, 60);
pub const TEXT: Color = Color::Rgb(201, 209, 217);
pub const TEXT_DIM: Color = Color::Rgb(120, 120, 120);
pub const TEXT_MUTED: Color = Color::Rgb(150, 150, 150);
pub const ACCENT: Color = Color::Rgb(231, 91, 42);   // rust orange
pub const SELECTED_BG: Color = Color::Rgb(50, 25, 15); // dark rust tint
pub const OK: Color = Color::Rgb(63, 185, 80);
pub const WARN: Color = Color::Rgb(210, 153, 34);
pub const ERR: Color = Color::Rgb(248, 81, 73);
pub const PURPLE: Color = Color::Rgb(188, 140, 255);

pub fn normal() -> Style { Style::default().fg(TEXT).bg(BG) }
pub fn dim() -> Style { Style::default().fg(TEXT_DIM).bg(BG) }
pub fn muted() -> Style { Style::default().fg(TEXT_MUTED).bg(BG) }
pub fn accent() -> Style { Style::default().fg(ACCENT) }
pub fn accent_bold() -> Style { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) }
pub fn ok() -> Style { Style::default().fg(OK) }
pub fn warn() -> Style { Style::default().fg(WARN) }
pub fn err() -> Style { Style::default().fg(ERR) }
pub fn purple() -> Style { Style::default().fg(PURPLE) }
pub fn danger() -> Style { err() }
pub fn inactive() -> Style { muted() }

pub fn panel_title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(SELECTED_BG)
        .add_modifier(Modifier::BOLD)
}

pub fn border_focused() -> Style { Style::default().fg(ACCENT) }
pub fn border_normal() -> Style { Style::default().fg(PANEL_BORDER) }
pub fn border_for(focused: bool) -> Style {
    if focused { border_focused() } else { border_normal() }
}
