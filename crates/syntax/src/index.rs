//! Parser symbol results and incremental documentation gathering, independent of UI indexing.

/// The shared file-count bound used by indexing and related-file discovery.
pub const MAX_FILES: usize = 100_000;
const DOC_LINES: usize = 30;
const SIGNATURE_CHARS: usize = 200;

/// A definition the index found.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedSymbol {
    pub name: String,
    /// What follows `entity.name.` in its scope: "function", "struct", "impl" ...
    pub kind: String,
    /// 0-based; `col` counts chars and points at the name.
    pub line: usize,
    pub col: usize,
    /// Its line, trimmed, at most 200 chars.
    pub signature: String,
    /// The comment lines just above it, or for Python the docstring under it, markers removed.
    pub doc: Option<String>,
}

/// What a line is to the doc comments around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    Blank,
    /// All of its text is in a comment scope.
    Comment,
    /// It starts with an attribute or decorator.
    Attribute,
    /// It starts in a docstring (Python).
    Docstring,
    /// It goes on with a definition's header (Python's `def f(` … `):`).
    Header,
    Code,
}

/// Doc comments, gathered line by line as a symbol parse goes: the contiguous comment
/// lines directly above a definition (attributes between them skipped), or for Python the
/// docstring under it, which wins.
#[derive(Clone, Default, PartialEq)]
pub struct Docs {
    /// The comment lines since the last other line, markers removed: the first `DOC_LINES`.
    above: Vec<String>,
    /// Definitions (indexes into the parse's) whose docstring may come next.
    waiting: Vec<usize>,
    /// Their comment remains part of checkpoint state until a docstring replaces it.
    pending_doc: Option<String>,
    /// The docstring being read for them.
    reading: Option<Vec<String>>,
}

impl Docs {
    pub fn pending(&self) -> &[usize] {
        &self.waiting
    }

    /// A checkpoint keeps pending definitions separately, starting at index zero.
    pub fn checkpoint(&self) -> Self {
        Self { waiting: (0..self.waiting.len()).collect(), ..self.clone() }
    }

    /// Line `text` (without its break) was parsed: it is `line`, `found[new..]` are its
    /// definitions, and `open` says a docstring goes on past it.
    pub fn line(
        &mut self,
        line: Line,
        text: &str,
        docstrings: bool,
        open: bool,
        found: &mut [IndexedSymbol],
        new: usize,
    ) {
        if let Some(lines) = &mut self.reading {
            if lines.len() < DOC_LINES {
                lines.push(text.trim().to_string());
            }
            if !open {
                let doc = docstring(lines);
                for &i in &self.waiting {
                    found[i].doc = doc.clone();
                }
                self.waiting.clear();
                self.reading = None;
            }
            return;
        }
        if new < found.len() {
            let doc = comment_doc(&self.above);
            for s in &mut found[new..] {
                s.doc = doc.clone();
            }
            self.above.clear();
            if docstrings {
                self.pending_doc = doc;
                self.waiting = (new..found.len()).collect();
            }
            return;
        }
        match line {
            Line::Blank => self.above.clear(),
            Line::Comment if self.above.len() < DOC_LINES => self.above.push(strip_comment(text)),
            Line::Comment | Line::Attribute => {}
            Line::Docstring if !self.waiting.is_empty() => {
                self.above.clear();
                self.pending_doc = None;
                self.reading = Some(Vec::new());
                self.line(line, text, docstrings, open, found, new);
            }
            Line::Header if !self.waiting.is_empty() => {}
            _ => {
                self.above.clear();
                self.waiting.clear();
                self.pending_doc = None;
            }
        }
    }
}

/// `lines` without the empty ones at either end, as one text; `None` if nothing is left.
fn comment_doc(lines: &[String]) -> Option<String> {
    let start = lines.iter().position(|l| !l.is_empty())?;
    let end = lines.iter().rposition(|l| !l.is_empty())?;
    Some(lines[start..=end].join("\n"))
}

/// A docstring's text: its lines without the quotes that open and close it.
fn docstring(lines: &[String]) -> Option<String> {
    let mut lines = lines.to_vec();
    let first = lines.first_mut()?;
    if first.starts_with(['r', 'R', 'u', 'U']) {
        first.remove(0);
    }
    let quote = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|q| first.starts_with(q));
    if let Some(q) = quote {
        *first = first[q.len()..].to_string();
        // the closing quote, unless the doc was cut before it
        let last = lines.last_mut()?;
        if let Some(rest) = last.strip_suffix(q) {
            *last = rest.trim_end().to_string();
        }
    }
    comment_doc(&lines)
}

/// A definition's `signature`: its line, trimmed, at most 200 chars.
pub fn signature(text: &str) -> String {
    text.trim().chars().take(SIGNATURE_CHARS).collect()
}

/// A comment line without its markers (`///`, `//!`, `//`, `/**`, `/*`, `*/`, a leading `*`,
/// `#`, `--`, `;`) and the space after them.
fn strip_comment(line: &str) -> String {
    const MARKERS: [&str; 9] = ["///", "//!", "//", "/**", "/*", "*", "#", "--", ";"];
    let t = line.trim();
    let t = t.strip_suffix("*/").unwrap_or(t).trim_end();
    let t = match MARKERS.iter().find(|m| t.starts_with(**m)) {
        // the marker, and more of its last char: `////`, `##`, `;;`, `---`
        Some(m) => t[m.len()..].trim_start_matches(m.chars().last().unwrap_or(' ')),
        None => t,
    };
    t.strip_prefix(' ').unwrap_or(t).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_markers_come_off() {
        for (line, want) in [
            ("/// Doc", "Doc"),
            ("//! Inner", "Inner"),
            ("// plain", "plain"),
            ("/** Block", "Block"),
            (" * middle", "middle"),
            (" */", ""),
            ("/* one */", "one"),
            ("# hash", "hash"),
            ("## two", "two"),
            ("-- lua", "lua"),
            (";; lisp", "lisp"),
            ("///   indented", "  indented"),
            ("/// # Heading", "# Heading"),
        ] {
            assert_eq!(strip_comment(line), want, "{line}");
        }
    }
}
