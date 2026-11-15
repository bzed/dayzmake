//! The CLI's warning sink: prints every warning to stderr as it is raised, and the error line.

use std::io::Error;

use colored::Colorize;
use dayzmake_core::warning::{Warning, WarningKind, WarningSink, Warnings};

/// Prints each kept warning to stderr immediately; remembers the quotas for [`print_summary`].
///
/// [`print_summary`]: PrintWarnings::print_summary
pub struct PrintWarnings {
    inner: Warnings,
}

#[expect(clippy::print_stderr, reason = "CLI diagnostics")]
fn print_line(location: &str, message: &str, name: Option<&str>) {
    let name = name.map_or_else(String::new, |n| format!(" [{n}]"));
    eprintln!("{location}{}: {message}{name}", "warning".yellow().bold());
}

#[expect(clippy::print_stderr, reason = "CLI diagnostics")]
fn print_note(message: &str) {
    eprintln!("{message}");
}

impl PrintWarnings {
    /// Mutes the given warning names; with `verbose`, lifts the per-name quota.
    #[must_use]
    pub fn new(muted: Vec<String>, verbose: bool) -> Self {
        let mut inner = Warnings::new();
        for name in muted {
            inner.mute(name);
        }
        if verbose {
            inner.unlimited();
        }
        PrintWarnings { inner }
    }

    /// Prints one line per warning type that hit its quota.
    pub fn print_summary(&self) {
        for (name, excess) in self.inner.suppressed() {
            let msg = if excess > 1 {
                format!(
                    "{excess} warnings of type \"{name}\" were suppressed to prevent spam. Use \"-w {name}\" to disable these warnings entirely."
                )
            } else {
                format!(
                    "{excess} warning of type \"{name}\" was suppressed to prevent spam. Use \"-w {name}\" to disable these warnings entirely."
                )
            };
            print_line("", &msg, None);
        }
    }
}

impl WarningSink for PrintWarnings {
    fn warn(&mut self, warning: Warning) {
        if let Some(kept) = self.inner.record(warning) {
            if kept.kind == WarningKind::Note {
                print_note(&kept.message);
                return;
            }
            print_line(&kept.location(), &kept.message, kept.name);
        }
    }

    fn is_suppressed(&self, name: Option<&str>) -> bool {
        self.inner.is_suppressed(name)
    }
}

/// Prints the error of a `Result` for the user.
pub trait PrintError {
    /// Prints `error: <message>` to stderr in red; exits with status 1 if `exit`. Print the
    /// warning summary before calling it.
    fn print_error(self, exit: bool);
}

impl<T> PrintError for Result<T, Error> {
    fn print_error(self, exit: bool) {
        if let Err(error) = self {
            #[expect(clippy::print_stderr, reason = "CLI diagnostics")]
            {
                eprintln!("{}: {}", "error".red().bold(), error);
            }
            if exit {
                std::process::exit(1);
            }
        }
    }
}
