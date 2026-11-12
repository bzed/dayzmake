#![macro_use]

use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::io::Error;
use std::path::Path;
use std::sync::Mutex;

use colored::Colorize;
use peg::error::ParseError;
use peg::str::LineCol;

use crate::preprocess::PreprocessInfo;

struct WarningState {
    max: u32,
    muted: HashSet<String>,
    raised: HashMap<String, u32>,
}

static WARNING_STATE: std::sync::LazyLock<Mutex<WarningState>> = std::sync::LazyLock::new(|| {
    Mutex::new(WarningState {
        max: 10,
        muted: HashSet::new(),
        raised: HashMap::new(),
    })
});

#[macro_export]
/// Creates an `std::io::Error` with `ErrorKind::Other` from a formatted message.
macro_rules! error {
    ($($arg:tt)*) => (
        std::io::Error::new(std::io::ErrorKind::Other, format!($($arg)*))
    )
}

/// Extensions on `Result<T, std::io::Error>` for error context and reporting.
pub trait ErrorExt<T> {
    /// Prepends `msg` to the error message, keeping the original text.
    ///
    /// # Errors
    ///
    /// Returns the original error with `msg` prepended to its message.
    fn prepend_error<M: AsRef<[u8]> + Display>(self, msg: M) -> Result<T, Error>;
    /// Prints the error to stderr; exits with status 1 if `exit` is set.
    fn print_error(self, exit: bool) -> ();
}
impl<T> ErrorExt<T> for Result<T, Error> {
    fn prepend_error<M: AsRef<[u8]> + Display>(self, msg: M) -> Result<T, Error> {
        match self {
            Ok(t) => Ok(t),
            Err(e) => Err(error!("{}\n{}", msg, e)),
        }
    }

    fn print_error(self, exit: bool) {
        if let Err(error) = self {
            #[expect(clippy::print_stderr, reason = "CLI diagnostics")]
            {
                eprintln!("{}: {}", "error".red().bold(), error);
            }

            if exit {
                print_warning_summary();
                std::process::exit(1);
            }
        }
    }
}

/// Formats parse errors from the preprocessor with file/line context.
pub trait PreprocessParseErrorExt<T> {
    /// Formats a preprocess parse error, showing the offending input line.
    ///
    /// # Errors
    ///
    /// Returns the formatted error, or the original `Ok` value.
    fn format_error(self, origin: Option<&Path>, input: &str) -> Result<T, Error>;
}
impl<T> PreprocessParseErrorExt<T> for Result<T, ParseError<LineCol>> {
    fn format_error(self, origin: Option<&Path>, input: &str) -> Result<T, Error> {
        match self {
            Ok(t) => Ok(t),
            Err(pe) => {
                let line_origin = u32::try_from(pe.location.line - 1)
                    .map_err(|_| error!("invalid line number {}", pe.location.line))?;
                let file_origin = match origin {
                    Some(path) => format!("{}:", path.display()),
                    None => String::new(),
                };

                let line = input.lines().nth(pe.location.line - 1).unwrap_or("");

                Err(format_parse_error(
                    line,
                    &file_origin,
                    line_origin,
                    pe.location.column,
                    &pe.expected,
                ))
            }
        }
    }
}

/// Formats parse errors from the config parser with file/line context.
pub trait ConfigParseErrorExt<T> {
    /// Formats a config parse error, showing the offending input line.
    ///
    /// # Errors
    ///
    /// Returns the formatted error, or an error if the error location is out of range.
    fn format_error(self, info: &PreprocessInfo, input: &str) -> Result<T, Error>;
}
impl<T> ConfigParseErrorExt<T> for Result<T, ParseError<LineCol>> {
    fn format_error(self, info: &PreprocessInfo, input: &str) -> Result<T, Error> {
        match self {
            Ok(t) => Ok(t),
            Err(pe) => {
                // line_origins is never empty: preprocess() always seeds it with the
                // top-level file, and its length is an upper bound for the parser's
                // 1-based line numbers.
                let idx = min(pe.location.line, info.line_origins.len()) - 1;
                let Some((line_origin, file)) = info.line_origins.get(idx) else {
                    return Err(error!(
                        "Parse error location out of range: line {}",
                        pe.location.line
                    ));
                };
                let line_origin = *line_origin;
                let file_origin = match file {
                    Some(path) => format!("{}:", path.display()),
                    None => String::new(),
                };

                let line = input.lines().nth(pe.location.line - 1).unwrap_or("");

                Err(format_parse_error(
                    line,
                    &file_origin,
                    line_origin,
                    pe.location.column,
                    &pe.expected,
                ))
            }
        }
    }
}

fn format_parse_error(
    line: &str,
    file: &str,
    line_number: u32,
    column_number: usize,
    expected: &impl Display,
) -> Error {
    let trimmed = line.trim_start();

    error!(
        "In line {}{}:\n\n  {}\n  {}{}\n\nUnexpected token \"{}\", expected: {}",
        file,
        line_number,
        trimmed,
        " ".to_string()
            .repeat(column_number - 1 - (line.len() - trimmed.len())),
        "^".red().bold(),
        line.chars()
            .map(|x| x.to_string())
            .nth(column_number - 1)
            .unwrap_or_else(|| "\\n".to_string()),
        expected
    )
}

fn print_warning_message<M: AsRef<[u8]> + Display>(
    msg: M,
    name: Option<&'static str>,
    location: (&Option<M>, &Option<u32>),
) {
    let loc_str = match (location.0, location.1) {
        (Some(file), Some(line)) => format!("In file {file}:{line}: "),
        (Some(file), None) => format!("In file {file}: "),
        (None, Some(line)) => format!("In line {line}: "),
        (None, None) => String::new(),
    };

    let name_str = match name {
        Some(name) => format!(" [{name}]"),
        None => String::new(),
    };

    #[expect(clippy::print_stderr, reason = "CLI diagnostics")]
    {
        eprintln!(
            "{}{}: {}{}",
            loc_str,
            "warning".yellow().bold(),
            msg,
            name_str
        );
    }
}

/// Raises a warning, unless it is muted or its per-name quota is exhausted.
#[expect(
    clippy::needless_pass_by_value,
    reason = "callers build the location tuple inline; by-value keeps the call sites simple"
)]
pub fn warning<M: AsRef<[u8]> + Display>(
    msg: M,
    name: Option<&'static str>,
    location: (Option<M>, Option<u32>),
) {
    let mut state = warning_state();

    if let Some(name_str) = name {
        if state.muted.contains(name_str) {
            return;
        }

        let max_warnings = state.max;
        let raised_count = state.raised.entry(name_str.to_string()).or_insert(0);
        if *raised_count >= max_warnings {
            return;
        }
        *raised_count += 1;
    }

    // Drop the lock before printing to avoid deadlocks if printing logic ever changes to call back into this module.
    drop(state);
    print_warning_message(msg, name, (&location.0, &location.1));
}

/// Returns true if warnings with this name are muted or over their quota.
#[must_use]
pub fn warning_suppressed(name: Option<&'static str>) -> bool {
    let Some(name) = name else { return false };

    let state = warning_state();

    if state.muted.contains(name) {
        return true;
    }

    if let Some(raised) = state.raised.get(name) {
        raised >= &state.max
    } else {
        false
    }
}

/// Prints one summary line per warning type that hit its quota.
pub fn print_warning_summary() {
    let state = warning_state();
    let mut summary_warnings = Vec::new();

    for (name, raised) in &state.raised {
        if state.muted.contains(name) {
            continue;
        }

        if *raised > state.max {
            let excess = *raised - state.max;
            let msg = if excess > 1 {
                format!(
                    "{excess} warnings of type \"{name}\" were suppressed to prevent spam. Use \"-w {name}\" to disable these warnings entirely."
                )
            } else {
                format!(
                    "{excess} warning of type \"{name}\" was suppressed to prevent spam. Use \"-w {name}\" to disable these warnings entirely."
                )
            };
            summary_warnings.push(msg);
        }
    }

    drop(state);

    for msg in summary_warnings {
        print_warning_message(msg, None, (&None, &None));
    }
}

/// Mutes the given warning names; with `verbose`, lifts the per-name quota.
pub fn init_warnings(muted: impl IntoIterator<Item = String>, verbose: bool) {
    let mut state = warning_state();
    state.muted = muted.into_iter().collect();
    if verbose {
        state.max = u32::MAX;
    }
}

/// Locks the warning state; a poisoned lock is unwrapped instead of panicking,
/// because the state is still usable after a panic while holding it.
fn warning_state() -> std::sync::MutexGuard<'static, WarningState> {
    WARNING_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
