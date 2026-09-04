use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

#[derive(Debug, Clone, Default)]
pub struct StatusBar {
    pub version: String,
    pub base_url: String,
    pub extra: Option<String>,
}

impl StatusBar {
    pub fn new(version: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            base_url: base_url.into(),
            extra: None,
        }
    }

    pub fn with_extra(mut self, extra: impl Into<String>) -> Self {
        self.extra = Some(extra.into());
        self
    }
}

impl Widget for StatusBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Color::Rgb(22, 27, 34);
        let fg = Color::Rgb(110, 118, 129);
        let accent = Color::Rgb(88, 166, 255);
        let sep = Color::Rgb(48, 54, 61);

        // Clear the row with dark bg. cell_mut returns None when (x, y) lies
        // outside the buffer (e.g. on a 0-width terminal); skip silently then.
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.top())) {
                cell.set_char(' ');
                cell.set_bg(bg);
            }
        }

        let mut spans: Vec<Span> = vec![
            Span::styled(
                format!(" edgeplane {}", self.version),
                Style::default()
                    .fg(accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(sep).bg(bg)),
            Span::styled(self.base_url.clone(), Style::default().fg(fg).bg(bg)),
        ];
        if let Some(extra) = &self.extra {
            spans.push(Span::styled(" · ", Style::default().fg(sep).bg(bg)));
            spans.push(Span::styled(extra.clone(), Style::default().fg(fg).bg(bg)));
        }

        let line = Line::from(spans);
        let line_width = line.width() as u16;
        let x = area.left();
        let y = area.top();
        buf.set_line(x, y, &line, line_width.min(area.width));
    }
}
