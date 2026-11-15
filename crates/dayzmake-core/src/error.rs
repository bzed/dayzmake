#![macro_use]

use std::cmp::min;
use std::fmt::Display;
use std::io::Error;
use std::path::Path;

use peg::error::ParseError;
use peg::str::LineCol;

use crate::preprocess::PreprocessInfo;

#[macro_export]
/// Creates an `std::io::Error` with `ErrorKind::Other` from a formatted message.
macro_rules! error {
    ($($arg:tt)*) => (
        std::io::Error::new(std::io::ErrorKind::Other, format!($($arg)*))
    )
}

/// Extensions on `Result<T, std::io::Error>` for error context.
pub trait ErrorExt<T> {
    /// Prepends `msg` to the error message, keeping the original text.
    ///
    /// # Errors
    ///
    /// Returns the original error with `msg` prepended to its message.
    fn prepend_error<M: AsRef<[u8]> + Display>(self, msg: M) -> Result<T, Error>;
}
impl<T> ErrorExt<T> for Result<T, Error> {
    fn prepend_error<M: AsRef<[u8]> + Display>(self, msg: M) -> Result<T, Error> {
        match self {
            Ok(t) => Ok(t),
            Err(e) => Err(error!("{}\n{}", msg, e)),
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
        "^",
        line.chars()
            .map(|x| x.to_string())
            .nth(column_number - 1)
            .unwrap_or_else(|| "\\n".to_string()),
        expected
    )
}

/// Makes text from an untrusted file safe to print: control characters (escape sequences, line
/// breaks, bell) are written as `\n`, `\u{1b}` and so on, so they cannot drive the terminal.
#[must_use]
pub fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<char>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
