//! Terminal UI helpers: ANSI colors, styled output macros, launch banner.

// ── ANSI color / style codes ─────────────────────────────────────────────────

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";

/// Brand: deep rust-orange balance (#C25A12 ≈ 256-color 166)
pub const ORANGE: &str = "\x1b[38;5;166m";
pub const ORANGE_BOLD: &str = "\x1b[1;38;5;166m";

pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const RED: &str = "\x1b[31m";
pub const CYAN: &str = "\x1b[36m";
pub const GRAY: &str = "\x1b[90m";

// ── Output macros ─────────────────────────────────────────────────────────────

/// Dim gray `mc:` prefix, normal body — routine info.
#[macro_export]
macro_rules! mc_info {
    ($($arg:tt)*) => {
        eprintln!("{}mc:{} {}", $crate::ui::GRAY, $crate::ui::RESET, format_args!($($arg)*))
    };
}

/// Green check — something completed successfully.
#[macro_export]
macro_rules! mc_ok {
    ($($arg:tt)*) => {
        eprintln!("{}✓{} {}", $crate::ui::GREEN, $crate::ui::RESET, format_args!($($arg)*))
    };
}

/// Yellow — needs attention but not fatal.
#[macro_export]
macro_rules! mc_warn {
    ($($arg:tt)*) => {
        eprintln!("{}⚑ {}{}", $crate::ui::YELLOW, $crate::ui::RESET, format_args!($($arg)*))
    };
}

/// Red — error / fatal.
#[macro_export]
macro_rules! mc_err {
    ($($arg:tt)*) => {
        eprintln!("{}✗ {}{}", $crate::ui::RED, $crate::ui::RESET, format_args!($($arg)*))
    };
}

// ── Banner ────────────────────────────────────────────────────────────────────

/// Print the Mission Control launch banner to stderr.
///
/// Shows the brand logo, the agent being launched, and a link to the web UI.
pub fn print_banner(base_url: &str, agent_label: &str, version: &str) {
    let ui_url = format!("{}/ui/", base_url.trim_end_matches('/'));

    // Box inner width (visible chars between the ║ borders)
    const W: usize = 54;

    // Pad a string to exactly W visible chars (truncates if over).
    let pad = |s: &str| -> String {
        let len = s.chars().count();
        if len >= W {
            s.chars().take(W).collect()
        } else {
            format!("{}{}", s, " ".repeat(W - len))
        }
    };

    let logo_1 = pad("  ███╗   ███╗ ██████╗");
    let logo_2 = pad("  ████╗ ████║██╔════╝   MISSION CONTROL");
    let logo_3 = pad("  ██╔████╔██║██║         AI Operations");
    let logo_4 = pad("  ██║╚██╔╝██║██╚═══██╗");
    let logo_5 = pad("  ██║ ╚═╝ ██║╚██████╔╝");
    let logo_6 = pad("  ╚═╝     ╚═╝ ╚═════╝");

    let blank = pad("");
    let agent_line = pad(&format!("  ▶  Agent    :  {}", agent_label));
    let url_line = pad(&format!("  ◈  Web UI   :  {}", ui_url));
    let ver_line = pad(&format!("  ·  mc {}", version));

    let top = format!("{}╔{}╗{}", ORANGE, "═".repeat(W), RESET);
    let bot = format!("{}╚{}╝{}", ORANGE, "═".repeat(W), RESET);

    let row = |content: &str, color: &str| {
        eprintln!(
            "{}║{}{}{}{}║{}",
            ORANGE, RESET, color, content, ORANGE, RESET
        );
    };

    eprintln!();
    eprintln!("{}", top);
    row(&logo_1, ORANGE_BOLD);
    row(&logo_2, ORANGE_BOLD);
    row(&logo_3, ORANGE_BOLD);
    row(&logo_4, ORANGE_BOLD);
    row(&logo_5, ORANGE_BOLD);
    row(&logo_6, ORANGE_BOLD);
    row(&blank, "");
    row(&agent_line, CYAN);
    row(&url_line, CYAN);
    row(&ver_line, GRAY);
    eprintln!("{}", bot);
    eprintln!();
}
