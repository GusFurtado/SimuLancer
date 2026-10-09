use std::fmt::Write;

use serde_saphyr::Location;

/// A place in the model file. Line and column are 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u64,
    pub column: u64,
    /// Length of the highlighted text, in characters.
    pub len: u64,
}

impl Position {
    pub(crate) fn from_location(location: &Location) -> Option<Position> {
        if *location == Location::UNKNOWN {
            return None;
        }
        Some(Position {
            line: location.line(),
            column: location.column(),
            len: location.span().len(),
        })
    }
}

/// A validation error, pointing at the part of the file that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub position: Option<Position>,
    pub message: String,
}

impl Diagnostic {
    /// Formats the error with the offending line of `source` underlined.
    pub fn render(&self, path: &str, source: &str) -> String {
        self.render_as("error", path, source)
    }

    /// Like [`Diagnostic::render`], with another label (e.g. "warning").
    pub fn render_as(&self, label: &str, path: &str, source: &str) -> String {
        let mut out = format!("{label}: {}\n", self.message);
        let Some(pos) = self.position else {
            let _ = writeln!(out, "  --> {path}");
            return out;
        };
        let _ = writeln!(out, "  --> {path}:{}:{}", pos.line, pos.column);
        let line_text = usize::try_from(pos.line)
            .ok()
            .and_then(|line| source.lines().nth(line.checked_sub(1)?));
        if let Some(text) = line_text {
            let number = pos.line.to_string();
            let gutter = " ".repeat(number.len());
            let indent = " ".repeat(pos.column.saturating_sub(1) as usize);
            let marks = "^".repeat(pos.len.max(1) as usize);
            let _ = writeln!(out, "{gutter} |");
            let _ = writeln!(out, "{number} | {text}");
            let _ = writeln!(out, "{gutter} | {indent}{marks}");
        }
        out
    }
}

/// Why a model file couldn't be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// Not valid YAML, or the structure doesn't match the format (unknown key,
    /// wrong type, missing field). Parsing stops at the first one.
    Syntax(Diagnostic),
    /// Valid structure, but the model doesn't make sense (unknown names,
    /// bad units, broken connections). All problems found, in file order.
    Invalid(Vec<Diagnostic>),
}

impl LoadError {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            LoadError::Syntax(diagnostic) => std::slice::from_ref(diagnostic),
            LoadError::Invalid(diagnostics) => diagnostics,
        }
    }

    pub fn render(&self, path: &str, source: &str) -> String {
        self.diagnostics()
            .iter()
            .map(|d| d.render(path, source))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
