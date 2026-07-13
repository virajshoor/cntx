use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use termimad::MadSkin;

use crate::models::RefreshReport;

/// Terminal color theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "light" | "light-mode" => Self::Light,
            _ => Self::Dark,
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

static CURRENT_THEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0); // 0=dark, 1=light

/// Set the active theme for markdown rendering.
pub fn set_theme(theme: Theme) {
    CURRENT_THEME.store(
        match theme {
            Theme::Dark => 0,
            Theme::Light => 1,
        },
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Get the current theme.
pub fn current_theme() -> Theme {
    match CURRENT_THEME.load(std::sync::atomic::Ordering::Relaxed) {
        1 => Theme::Light,
        _ => Theme::Dark,
    }
}

pub fn spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(90));
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
    spinner.enable_steady_tick(Duration::from_millis(260));
    spinner
}

/// A "thinking" indicator that shows an animated sequence like Claude Code's
/// thinking output. Spawns a background thread that prints dots until the
/// `done` flag is set. Call `thinking_start()` to get the flag, then
/// `thinking_stop(flag)` when the first token arrives.
pub fn thinking_start() -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();
    thread::spawn(move || {
        let frames = ["thinking", "thinking.", "thinking..", "thinking..."];
        let mut i = 0;
        while !done_clone.load(Ordering::Relaxed) {
            let frame = frames[i % frames.len()];
            // Use carriage return to overwrite the same line
            print!("\r{} {}", "~".cyan(), frame.dimmed());
            std::io::stdout().flush().ok();
            thread::sleep(Duration::from_millis(200));
            i += 1;
        }
        // Clear the thinking line
        print!("\r{}\r", " ".repeat(20));
        std::io::stdout().flush().ok();
    });
    done
}

/// Stop the thinking indicator. Call this when the first token arrives.
pub fn thinking_stop(done: Arc<AtomicBool>) {
    done.store(true, Ordering::Relaxed);
    // Small delay to let the thread clear the line
    thread::sleep(Duration::from_millis(50));
}

/// Show a live preview of the last N characters of the stream as it arrives.
/// Call `preview_start()` to get the shared buffer, then `preview_update(buf, text)`
/// to push new text, and `preview_stop()` to clear the line.
pub fn preview_start() -> Arc<std::sync::Mutex<String>> {
    Arc::new(std::sync::Mutex::new(String::new()))
}

/// Update the live preview line with the latest streamed text.
/// Shows the last ~80 characters of `text` on a single line, overwriting
/// the previous preview. Call this from the streaming callback.
pub fn preview_update(buf: &Arc<std::sync::Mutex<String>>, text: &str) {
    if let Ok(mut inner) = buf.lock() {
        inner.push_str(text);
        let display: String = inner
            .chars()
            .rev()
            .take(80)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // Collapse newlines for single-line display
        let display = display.replace('\n', " ").replace('\r', "");
        print!("\r{}", " ".repeat(100));
        print!("\r{}", display.dimmed());
        std::io::stdout().flush().ok();
    }
}

/// Clear the live preview line.
pub fn preview_stop() {
    print!("\r{}\r", " ".repeat(100));
    std::io::stdout().flush().ok();
}

/// Render markdown text to the terminal. Used for assistant responses so that
/// `**bold**`, headers, lists, and code blocks render instead of showing raw
/// markdown punctuation. Adapts colors to the current theme (dark/light).
pub fn print_markdown(text: &str) {
    let mut skin = MadSkin::default();
    let theme = current_theme();
    match theme {
        Theme::Dark => {
            skin.headers[0].compound_style.set_fg(ansi(14)); // cyan
            skin.bold.set_fg(ansi(14));
            skin.inline_code.set_fg(ansi(11)); // yellow
        }
        Theme::Light => {
            skin.headers[0].compound_style.set_fg(ansi(4)); // blue
            skin.bold.set_fg(ansi(4));
            skin.inline_code.set_fg(ansi(2)); // green
        }
    }
    skin.italic
        .add_attr(termimad::crossterm::style::Attribute::Italic);
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
