use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use termimad::MadSkin;

use crate::models::RefreshReport;

pub fn spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(std::time::Duration::from_millis(90));
    spinner
}

/// A working indicator with an animated dot spinner so the terminal does not
/// look frozen while the model generates.
pub fn working() -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} working")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&[".", "..", "...", ""]),
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(260));
    spinner
}

/// Render markdown text to the terminal. Used for assistant responses so that
/// `**bold**`, headers, lists, and code blocks render instead of showing raw
/// markdown punctuation.
pub fn print_markdown(text: &str) {
    let mut skin = MadSkin::default();
    skin.headers[0].compound_style.set_fg(ansi(14));
    skin.bold.set_fg(ansi(14));
    skin.italic
        .add_attr(termimad::crossterm::style::Attribute::Italic);
    skin.inline_code.set_fg(ansi(11));
    skin.print_text(text);
}

fn ansi(color: u8) -> termimad::crossterm::style::Color {
    termimad::crossterm::style::Color::AnsiValue(color)
}

pub fn print_refresh_report(report: &RefreshReport) {
    for endpoint in &report.endpoints {
        println!(
            "{} {} models available for {}",
            "[ok]".green(),
            endpoint.total_available,
            endpoint.endpoint.bold()
        );
        if !endpoint.added.is_empty() {
            println!("  added: {}", endpoint.added.join(", "));
        }
        if !endpoint.deprecated.is_empty() {
            println!("  deprecated: {}", endpoint.deprecated.join(", "));
        }
    }
}

pub fn muted(value: impl AsRef<str>) -> String {
    value.as_ref().dimmed().to_string()
}
