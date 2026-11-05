use super::*;

impl EditorState {
    /// Malformed Unicode is decoded lossily and the document is flagged read-only so `save`
    /// cannot overwrite the original with mangled text.
    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::from_loaded(path, Loaded::read(path)?))
    }

    /// The document of a file read (perhaps on another thread) from `path`, with a view.
    pub fn from_loaded(path: &Path, l: Loaded) -> Self {
        Self::from_document(Document::from_loaded(path, l))
    }
}

impl Document {
    /// The document of a file read (perhaps on another thread) from `path`.
    pub fn from_loaded(path: &Path, l: Loaded) -> Self {
        let mut s = Self::new();
        s.rope = l.rope;
        s.endings = l.endings;
        s.line_ending = l.line_ending;
        s.bom = l.bom;
        s.encoding = l.encoding;
        s.disk_encoding = l.encoding;
        s.path = Some(path.to_path_buf());
        s.disk = l.disk;
        s.read_only = l.read_only;
        // clean: the file holds this text, written this way
        s.saved = Some(s.save_point());
        s
    }
}
