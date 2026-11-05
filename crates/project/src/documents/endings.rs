//! Physical file boundaries beside normalized LF source. Both ropes share their snapshots.
use orlo_utils::text::{diff, Change};
use ropey::Rope;
use std::io;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct LineEndings {
    kinds: Rope,
    id: u64,
}

impl PartialEq for LineEndings {
    fn eq(&self, other: &Self) -> bool {
        self.kinds == other.kinds
    }
}
impl Eq for LineEndings {}

fn kind(ending: &str) -> char {
    match ending {
        "\r\n" => 'w',
        "\r" => 'r',
        _ => 'l',
    }
}
fn ending(kind: char) -> &'static str {
    match kind {
        'w' => "\r\n",
        'r' => "\r",
        _ => "\n",
    }
}

impl LineEndings {
    fn new(kinds: Rope) -> Self {
        Self { kinds, id: NEXT.fetch_add(1, Ordering::Relaxed) }
    }

    pub fn identity(&self) -> u64 {
        self.id
    }
    pub fn compact(&self) -> String {
        self.kinds.to_string()
    }

    /// Present malformed metadata is an error; absence is handled by the legacy caller.
    pub fn restore(kinds: &str, source: &Rope) -> io::Result<Self> {
        if kinds.len() > super::io::MAX_SIZE as usize
            || kinds.len() != source.len_lines() - 1
            || !kinds.bytes().all(|b| matches!(b, b'l' | b'w' | b'r'))
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid line ending boundary map"));
        }
        Ok(Self::new(Rope::from_str(kinds)))
    }

    pub fn uniform(source: &Rope, preferred: &str) -> Self {
        Self::new(Rope::from_str(&kind(preferred).to_string().repeat(source.len_lines() - 1)))
    }

    pub fn counts(&self) -> [usize; 3] {
        let mut counts = [0; 3];
        for c in self.kinds.chars() {
            counts[match c {
                'w' => 1,
                'r' => 2,
                _ => 0,
            }] += 1;
        }
        counts
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.kinds.chars().map(ending)
    }

    pub fn at(&self, line: usize) -> Option<&'static str> {
        self.kinds.get_char(line).map(ending)
    }

    /// Actual minimal changes preserve every boundary outside their removed ranges.
    pub fn apply_changes(&mut self, before: &Rope, after: &Rope, changes: &[Change], preferred: &str) {
        if !changes.iter().any(|c| c.lines != (0, 0)) {
            return;
        }
        let mut source = before.clone();
        for c in changes {
            let range = c.start..c.start + c.removed;
            let inserted = after.slice(c.start..c.start + c.inserted).to_string();
            self.splice(&source, range.clone(), &inserted, preferred);
            source.remove(range);
            source.insert(c.start, &inserted);
        }
    }

    pub fn apply_diff(&mut self, before: &Rope, after: &Rope, preferred: &str) {
        self.apply_changes(before, after, &diff(before, after), preferred);
    }

    pub fn splice(&mut self, source: &Rope, range: Range<usize>, inserted: &str, preferred: &str) {
        let start = source.char_to_line(range.start);
        let end = source.char_to_line(range.end);
        let count = inserted.bytes().filter(|b| *b == b'\n').count();
        if start == end && count == 0 {
            return;
        }
        self.kinds.remove(start..end);
        self.kinds.insert(start, &kind(preferred).to_string().repeat(count));
        self.id = NEXT.fetch_add(1, Ordering::Relaxed);
    }

    /// Serialize LF source, retaining literal CR source characters from text operations.
    pub fn text(&self, source: &Rope, range: Range<usize>) -> io::Result<String> {
        if self.kinds.len_chars() != source.len_lines() - 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Line ending map does not match source"));
        }
        let mut kinds = self.kinds.chars_at(source.char_to_line(range.start));
        let mut out = String::new();
        for c in source.slice(range).chars() {
            if c == '\n' {
                out.push_str(ending(kinds.next().expect("validated boundary map")));
            } else {
                out.push(c);
            }
        }
        Ok(out)
    }
}

/// Physical serialization locations, including CR source content from arbitrary text edits.
pub fn visit_boundaries(
    source: &Rope,
    endings: Option<&LineEndings>,
    preferred: &'static str,
    mut visit: impl FnMut(&'static str, usize),
) {
    let mut kinds = endings.map(LineEndings::iter);
    let mut pending = None;
    for (at, source) in source.chars().enumerate() {
        let boundary = (source == '\n').then(|| kinds.as_mut().and_then(Iterator::next).unwrap_or(preferred));
        let mut literal = [0; 4];
        let text: &str = match boundary {
            Some(ending) => ending,
            None => source.encode_utf8(&mut literal),
        };
        for c in text.chars() {
            match c {
                '\r' => {
                    if let Some(at) = pending.replace(at) {
                        visit("\r", at);
                    }
                }
                '\n' => {
                    if let Some(at) = pending.take() {
                        visit("\r\n", at);
                    } else {
                        visit("\n", at);
                    }
                }
                _ => {
                    if let Some(at) = pending.take() {
                        visit("\r", at);
                    }
                }
            }
        }
    }
    if let Some(at) = pending {
        visit("\r", at);
    }
}

pub fn physical_counts(source: &Rope, endings: Option<&LineEndings>, preferred: &'static str) -> [usize; 3] {
    let mut counts = [0; 3];
    visit_boundaries(source, endings, preferred, |ending, _| {
        counts[match ending {
            "\r\n" => 1,
            "\r" => 2,
            _ => 0,
        }] += 1
    });
    counts
}

/// Disk-only normalization: all CR/CRLF/LF are boundaries. Arbitrary text keeps io::normalized.
pub fn decoded(text: &str) -> (Rope, &'static str, LineEndings) {
    let mut source = ropey::RopeBuilder::new();
    let mut kinds = ropey::RopeBuilder::new();
    let mut counts = [0usize; 3];
    let bytes = text.as_bytes();
    let (mut start, mut at) = (0, 0);
    while at < bytes.len() {
        let (width, k, index) = match bytes[at] {
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => (2, "w", 1),
            b'\r' => (1, "r", 2),
            b'\n' => (1, "l", 0),
            _ => {
                at += 1;
                continue;
            }
        };
        source.append(&text[start..at]);
        source.append("\n");
        kinds.append(k);
        counts[index] += 1;
        at += width;
        start = at;
    }
    source.append(&text[start..]);
    let [lf, crlf, cr] = counts;
    let preferred = if cr > lf && cr > crlf {
        "\r"
    } else if crlf > lf && crlf >= cr {
        "\r\n"
    } else {
        "\n"
    };
    (source.finish(), preferred, LineEndings::new(kinds.finish()))
}
