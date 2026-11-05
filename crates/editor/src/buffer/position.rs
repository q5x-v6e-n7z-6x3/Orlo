//! Typed positions in a document's text. Each unit has its own type, so a char offset cannot be
//! passed where a byte or UTF-16 offset is wanted. They carry no version: conversions go through
//! `Coords`, which is the text of one version, so a position is only ever converted against the
//! text it was taken from.

use super::*;

/// A char index into the rope: the unit selections, changes and bookmarks use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharOffset(pub usize);

/// A UTF-8 byte offset, as the file is encoded in memory (parsers, regexes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(pub usize);

/// A UTF-16 code unit offset, as native text input counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utf16Offset(pub usize);

/// A 0-based line and a column in chars from that line's start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub line: usize,
    pub column: usize,
}

impl Point {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// The text of one version, for converting positions taken from it. Offsets past the end clamp
/// to it, and a column past its line's end to that end, as the editor clamps carets.
#[derive(Clone, Copy)]
pub struct Coords<'a> {
    rope: &'a Rope,
    version: u64,
}

impl<'a> Coords<'a> {
    pub fn new(rope: &'a Rope, version: u64) -> Self {
        Self { rope, version }
    }

    /// The version of the text these conversions are for.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The text itself, for reading chars and lines of this version.
    pub fn rope(&self) -> &'a Rope {
        self.rope
    }

    /// Whether this is the text of `version`: a position from another needs mapping first.
    pub fn is(&self, version: u64) -> bool {
        self.version == version
    }

    pub fn len(&self) -> CharOffset {
        CharOffset(self.rope.len_chars())
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    fn clamp(&self, o: CharOffset) -> usize {
        o.0.min(self.rope.len_chars())
    }

    pub fn char_to_byte(&self, o: CharOffset) -> ByteOffset {
        ByteOffset(self.rope.char_to_byte(self.clamp(o)))
    }

    /// The char that byte `b` is in (its start, when `b` is inside a multi-byte char).
    pub fn byte_to_char(&self, b: ByteOffset) -> CharOffset {
        CharOffset(self.rope.byte_to_char(b.0.min(self.rope.len_bytes())))
    }

    pub fn char_to_utf16(&self, o: CharOffset) -> Utf16Offset {
        Utf16Offset(self.rope.char_to_utf16_cu(self.clamp(o)))
    }

    /// The char that code unit `u` is in (the char, when `u` falls between a surrogate pair).
    pub fn utf16_to_char(&self, u: Utf16Offset) -> CharOffset {
        CharOffset(self.rope.utf16_cu_to_char(u.0.min(self.rope.len_utf16_cu())))
    }

    pub fn char_to_point(&self, o: CharOffset) -> Point {
        let o = self.clamp(o);
        let line = self.rope.char_to_line(o);
        Point::new(line, o - self.rope.line_to_char(line))
    }

    /// The point's char; a line past the last is the end of the text, and a column past its
    /// line's end (before its `\n` or `\r\n`) is that end, as `EditorState::offset` clamps.
    pub fn point_to_char(&self, p: Point) -> CharOffset {
        if p.line >= self.rope.len_lines() {
            return self.len();
        }
        let line = self.rope.line(p.line);
        let mut n = line.len_chars();
        if n > 0 && line.char(n - 1) == '\n' {
            n -= 1;
            if n > 0 && line.char(n - 1) == '\r' {
                n -= 1;
            }
        }
        CharOffset(self.rope.line_to_char(p.line) + p.column.min(n))
    }
}
