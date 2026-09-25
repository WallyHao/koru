//! Restrained ANSI styling that is disabled for redirected output.
//!
//! Every user-facing prompt is English; color is an optional emphasis that
//! must never leak into a pipe, a file, or a `NO_COLOR` session. Callers pick
//! the stream whose terminal status decides whether codes are emitted.
use std::io::IsTerminal;

/// A style bound to one output stream's color capability.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    enabled: bool,
}

impl Style {
    /// Styling that never emits escape codes; used by fixtures and redirected output.
    pub const fn plain() -> Self {
        Self { enabled: false }
    }

    /// Styling for standard output when it is an interactive terminal.
    pub fn stdout() -> Self {
        Self::detect(std::io::stdout().is_terminal())
    }

    /// Styling for standard error when it is an interactive terminal.
    pub fn stderr() -> Self {
        Self::detect(std::io::stderr().is_terminal())
    }

    fn detect(on_terminal: bool) -> Self {
        // NO_COLOR (any value, including empty) and TERM=dumb force plain text.
        let disabled = std::env::var_os("NO_COLOR").is_some()
            || std::env::var("TERM").is_ok_and(|term| term == "dumb");
        Self {
            enabled: on_terminal && !disabled,
        }
    }

    /// Whether this style emits escape codes.
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// Bold emphasis for headings and prompts.
    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    /// Dim text for secondary diagnostics such as the selected model line.
    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    /// Red for failures and denied actions.
    pub fn red(self, text: &str) -> String {
        self.paint("31", text)
    }

    /// Green for successful checks.
    pub fn green(self, text: &str) -> String {
        self.paint("32", text)
    }

    /// Yellow for warnings and truncation notices.
    pub fn yellow(self, text: &str) -> String {
        self.paint("33", text)
    }

    /// Cyan for interactive prompts and structured keys.
    pub fn cyan(self, text: &str) -> String {
        self.paint("36", text)
    }
}
