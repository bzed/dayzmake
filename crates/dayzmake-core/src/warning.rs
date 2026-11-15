//! Warnings raised by the library. A function that can warn takes a `WarningSink`; the library
//! never prints, and nothing is shared between calls.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::error::printable;

/// Per-name quota of kept warnings in [`Warnings::new`].
const DEFAULT_QUOTA: u32 = 10;

/// What a [`Warning`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WarningKind {
    /// A real warning: it has a name, can be muted and is subject to the per-name quota.
    #[default]
    Warning,
    /// A plain message for the user (progress, tool output): never muted, never counted against
    /// a quota, shown as the bare message.
    Note,
}

/// One warning: a message with an optional name (for muting), file and line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Warning {
    /// Warning or note.
    pub kind: WarningKind,
    /// The warning type, used for muting and the per-name quota.
    pub name: Option<&'static str>,
    /// What went wrong.
    pub message: String,
    /// The file the warning is about.
    pub file: Option<String>,
    /// The line the warning is about.
    pub line: Option<u32>,
}

impl Warning {
    /// A warning without location.
    #[must_use]
    pub fn new(name: Option<&'static str>, message: impl Into<String>) -> Warning {
        Warning {
            kind: WarningKind::Warning,
            name,
            message: message.into(),
            file: None,
            line: None,
        }
    }

    /// A note: a plain message that is not a warning (see [`WarningKind::Note`]).
    #[must_use]
    pub fn note(message: impl Into<String>) -> Warning {
        Warning {
            kind: WarningKind::Note,
            name: None,
            message: message.into(),
            file: None,
            line: None,
        }
    }

    /// Sets the file.
    #[must_use]
    pub fn in_file(mut self, file: impl Into<String>) -> Warning {
        self.file = Some(file.into());
        self
    }

    /// Sets the line.
    #[must_use]
    pub fn at_line(mut self, line: u32) -> Warning {
        self.line = Some(line);
        self
    }

    /// The location prefix (the file name is escaped with [`printable`]): `In file f:l: `, `In file f: `, `In line l: ` or nothing.
    #[must_use]
    pub fn location(&self) -> String {
        match (self.file.as_deref().map(printable), self.line) {
            (Some(file), Some(line)) => format!("In file {file}:{line}: "),
            (Some(file), None) => format!("In file {file}: "),
            (None, Some(line)) => format!("In line {line}: "),
            (None, None) => String::new(),
        }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == WarningKind::Note {
            return f.write_str(&self.message);
        }
        write!(f, "{}warning: {}", self.location(), self.message)?;
        if let Some(name) = self.name {
            write!(f, " [{name}]")?;
        }
        Ok(())
    }
}

/// Receives the warnings a library call raises.
pub trait WarningSink {
    /// Takes one warning.
    fn warn(&mut self, warning: Warning);

    /// True if warnings with this name would be dropped (muted or over quota); callers use it to
    /// skip work that only builds a warning.
    fn is_suppressed(&self, _name: Option<&str>) -> bool {
        false
    }
}

/// A sink that keeps warnings, with muted names and a per-name quota.
#[derive(Debug, Clone)]
pub struct Warnings {
    muted: BTreeSet<String>,
    quota: u32,
    raised: BTreeMap<String, u32>,
    kept: Vec<Warning>,
}

impl Default for Warnings {
    fn default() -> Self {
        Warnings::new()
    }
}

impl Warnings {
    /// Keeps at most 10 warnings per name; unnamed warnings have no quota.
    #[must_use]
    pub fn new() -> Warnings {
        Warnings {
            muted: BTreeSet::new(),
            quota: DEFAULT_QUOTA,
            raised: BTreeMap::new(),
            kept: Vec::new(),
        }
    }

    /// Drops all warnings with this name.
    pub fn mute(&mut self, name: impl Into<String>) {
        self.muted.insert(name.into());
    }

    /// Lifts the quota.
    pub fn unlimited(&mut self) {
        self.quota = u32::MAX;
    }

    /// Counts the warning and keeps it unless muted or over quota (notes are always kept); returns the kept warning.
    pub fn record(&mut self, warning: Warning) -> Option<&Warning> {
        if warning.kind == WarningKind::Note {
            self.kept.push(warning);
            return self.kept.last();
        }
        if let Some(name) = warning.name {
            if self.muted.contains(name) {
                return None;
            }
            let raised = self.raised.entry(name.to_string()).or_insert(0);
            *raised = raised.saturating_add(1);
            if *raised > self.quota {
                return None;
            }
        }
        self.kept.push(warning);
        self.kept.last()
    }

    /// The kept warnings in the order they were raised.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.kept
    }

    /// The kept warnings, consuming the sink.
    #[must_use]
    pub fn into_warnings(self) -> Vec<Warning> {
        self.kept
    }

    /// `(name, count)` for every name that went over its quota, sorted by name; muted names are
    /// not listed.
    #[must_use]
    pub fn suppressed(&self) -> Vec<(String, u32)> {
        self.raised
            .iter()
            .filter(|(name, raised)| !self.muted.contains(*name) && **raised > self.quota)
            .map(|(name, raised)| (name.clone(), *raised - self.quota))
            .collect()
    }
}

impl WarningSink for Warnings {
    fn warn(&mut self, warning: Warning) {
        self.record(warning);
    }

    fn is_suppressed(&self, name: Option<&str>) -> bool {
        let Some(name) = name else { return false };
        self.muted.contains(name) || self.raised.get(name).is_some_and(|r| *r >= self.quota)
    }
}
