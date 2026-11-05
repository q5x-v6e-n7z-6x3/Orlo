//! Saving in three steps, as loading reads a file off the UI thread and then takes it in: the
//! request is captured on the UI thread (a snapshot of the text and how it is written), written
//! on another (encode, file history, `write_safely`, the new stamp), and finished on the UI
//! thread, which records exactly what reached disk.

use super::*;
use orlo_project::encoding::Encoding;

pub(crate) static NEXT_STATE: AtomicU64 = AtomicU64::new(1);
static NEXT_SAVE: AtomicU64 = AtomicU64::new(1);

/// A new text state's id, never reused: undo steps keep theirs, so undo can come back to it.
pub(crate) fn next_state() -> u64 {
    NEXT_STATE.fetch_add(1, Ordering::Relaxed)
}

/// What is on disk as far as the document knows: a text state (an undo step's id) and how it
/// was written. The document is clean only while it is at this point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedPoint {
    pub state: u64,
    pub endings: u64,
    pub line_ending: &'static str,
    pub encoding: Encoding,
    pub bom: bool,
}

/// A save captured on the UI thread, for another to write: everything the write needs and
/// nothing of the document itself.
#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub document: u64,
    pub id: u64,
    pub rope: Rope,
    pub endings: Option<LineEndings>,
    /// The text's length, so what is written can be checked against what was asked.
    pub chars: CharOffset,
    /// Where it goes, a symlink resolved.
    pub path: PathBuf,
    /// The destination's encoding before this write, for its file history.
    pub previous: Encoding,
    pub history: bool,
    /// The text state and how it is written: what the file holds once this lands.
    pub point: SavedPoint,
}

/// What reached disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    pub disk: Option<Stamp>,
    pub bytes: ByteOffset,
}

/// A write that failed, and whether it touched the file: a write in place cut short leaves it
/// at `disk`, which is no longer what was last read.
#[derive(Debug)]
pub struct SaveFailure {
    pub error: io::Error,
    pub disk: Option<Option<Stamp>>,
}

/// The bytes a save writes for `text`: its line ending applied, the encoding and its BOM.
pub fn encoded(text: String, line_ending: &str, encoding: Encoding, bom: bool) -> io::Result<Vec<u8>> {
    orlo_project::encoding::encode_owned(file_text(text, line_ending, false), encoding, bom)
}

impl SaveRequest {
    pub fn encoding(&self) -> Encoding {
        self.point.encoding
    }

    /// Encode, keep the file's old version when file history is on, write it safely and take
    /// the file's new stamp. Runs off the UI thread; touches no document.
    pub fn write(&self) -> Result<Written, SaveFailure> {
        self.write_checked(None)
    }

    /// Automatic writes refuse known changes since the last complete read/save.
    pub fn write_if_unchanged(&self, expected: Stamp) -> Result<Written, SaveFailure> {
        self.write_checked(Some(expected))
    }

    fn write_checked(&self, expected: Option<Stamp>) -> Result<Written, SaveFailure> {
        let check = || {
            if expected.is_some() && std::fs::metadata(&self.path).is_ok_and(|m| m.permissions().readonly()) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "The file is read-only"));
            }
            if expected.is_some_and(|expected| stamp(&self.path) != Some(expected)) {
                return Err(io::Error::other("The file changed on disk; reload or save explicitly"));
            }
            Ok(())
        };
        self.write_with_check(check)
    }

    fn write_with_check(&self, check: impl Fn() -> io::Result<()>) -> Result<Written, SaveFailure> {
        let p = &self.point;
        let failed = |error| SaveFailure { error, disk: None };
        if p.endings != self.endings.as_ref().map_or(0, LineEndings::identity) {
            return Err(failed(io::Error::new(
                io::ErrorKind::InvalidData,
                "Save boundary metadata does not match its capture",
            )));
        }
        let text = match &self.endings {
            Some(map) => map.text(&self.rope, 0..self.rope.len_chars()).map_err(failed)?,
            None => file_text(self.rope.to_string(), p.line_ending, false),
        };
        let bytes = orlo_project::encoding::encode_owned(text, p.encoding, p.bom).map_err(failed)?;
        check().map_err(failed)?;
        capture_history(&self.path, self.previous, self.history).map_err(failed)?;
        let write = |f: &mut std::fs::File| std::io::Write::write_all(f, &bytes);
        let disk = orlo_project::io::write_safely_checked(&self.path, &write, &check)
            .map_err(|(error, touched)| SaveFailure { error, disk: touched.then(|| stamp(&self.path)) })?;
        Ok(Written { disk, bytes: ByteOffset(bytes.len()) })
    }
}

/// An immutable copy, with the overwrite decision pinned before asynchronous waiting.
#[derive(Clone, Debug)]
pub struct CopyRequest {
    pub save: SaveRequest,
    destination: PathBuf,
    source: Option<PathBuf>,
    source_file: Option<(u64, u64)>,
    expected: Option<Stamp>,
    destination_file: Option<(u64, u64)>,
}

fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

impl CopyRequest {
    /// Source protection and accepted overwrite state are rechecked by the safe writer.
    pub fn write(&self) -> Result<Written, SaveFailure> {
        self.save.write_with_check(|| {
            let path = &self.save.path;
            if resolve(&self.destination) != *path
                || resolve(path) != *path
                || stamp(path) != self.expected
                || file_identity(path) != self.destination_file
            {
                return Err(io::Error::other("Copy destination changed; choose the destination again"));
            }
            if self.source.as_ref().is_some_and(|source| {
                resolve(source) == resolve(path)
                    || file_identity(path)
                        .is_some_and(|id| Some(id) == self.source_file || Some(id) == file_identity(source))
            }) {
                return Err(io::Error::other("A copy cannot overwrite its source"));
            }
            if std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Copy destination is read-only"));
            }
            Ok(())
        })
    }
}

impl Document {
    /// The point the document is at now: its text state and how it would be written.
    pub fn save_point(&self) -> SavedPoint {
        SavedPoint {
            state: self.state,
            endings: self.endings.as_ref().map_or(0, LineEndings::identity),
            line_ending: self.line_ending,
            encoding: self.encoding,
            bom: self.bom,
        }
    }

    /// The document as it is now is what the file holds, as far as closing and undo go: a clean
    /// document given a repository's line ending or byte order mark as it opens stays clean.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.save_point());
        self.refresh_dirty();
    }

    /// Dirty unless at the point the file holds (Find Results never is).
    pub(crate) fn refresh_dirty(&mut self) {
        self.dirty = !self.scratch && self.saved != Some(self.save_point());
    }

    /// Whether the document can be written to `path`: a lossily decoded one cannot overwrite
    /// its original, however the path is spelled.
    pub fn check_save(&self, path: &Path) -> io::Result<()> {
        if self.is_following() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Stop following before saving"));
        }
        if !self.read_only {
            return Ok(());
        }
        let Some(original) = &self.path else {
            return Ok(());
        };
        use std::os::unix::fs::MetadataExt;
        let metadata = |p: &Path| match std::fs::metadata(p) {
            Ok(m) => Ok(Some((m.dev(), m.ino()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        };
        if let Some(source) = metadata(original)? {
            if resolve(original) == resolve(path) || metadata(path)? == Some(source) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Cannot overwrite a read-only original; use Save As with a different file",
                ));
            }
        }
        Ok(())
    }

    /// Capture without ending a typing group or changing any source state.
    pub fn capture_copy(&self, path: PathBuf) -> io::Result<CopyRequest> {
        self.check_copy(&path)?;
        let destination = path.clone();
        let path = resolve(&path);
        Ok(CopyRequest {
            destination,
            expected: stamp(&path),
            destination_file: file_identity(&path),
            source: self.path.clone(),
            source_file: self.path.as_deref().and_then(file_identity),
            save: SaveRequest {
                document: self.id,
                id: NEXT_SAVE.fetch_add(1, Ordering::Relaxed),
                rope: self.rope.clone(),
                endings: self.endings.clone(),
                chars: CharOffset(self.rope.len_chars()),
                path,
                previous: Encoding::default(),
                history: self.history_enabled,
                point: self.save_point(),
            },
        })
    }

    pub fn check_copy(&self, path: &Path) -> io::Result<()> {
        if self.is_following() {
            return Err(io::Error::other("Stop following before saving a copy"));
        }
        if self.path.as_ref().is_some_and(|source| {
            resolve(source) == resolve(path) || file_identity(path).is_some_and(|id| Some(id) == file_identity(source))
        }) {
            return Err(io::Error::other("A copy cannot overwrite its source"));
        }
        Ok(())
    }

    /// The request to write the document to `path` as it is now; `previous` is the
    /// destination's encoding when another document has it open. Typing after this starts a new
    /// undo step, so undo can come back to the text written.
    pub fn capture_save(&mut self, path: PathBuf, previous: Option<Encoding>) -> io::Result<SaveRequest> {
        self.check_save(&path)?;
        let previous = previous.unwrap_or_else(|| match self.path.as_ref() == Some(&path) {
            true => self.disk_encoding,
            false => Default::default(),
        });
        self.typing.set(None);
        Ok(SaveRequest {
            document: self.id,
            id: NEXT_SAVE.fetch_add(1, Ordering::Relaxed),
            rope: self.rope.clone(),
            endings: self.endings.clone(),
            chars: CharOffset(self.rope.len_chars()),
            path,
            previous,
            history: self.history_enabled,
            point: self.save_point(),
        })
    }

    /// `req` reached disk as `written`: the document's file is that one now, and it is clean if
    /// it is still at the point written (an edit, an undo past it or a new line ending meanwhile
    /// leaves it unsaved).
    pub fn finish_save(&mut self, req: &SaveRequest, written: &Written) {
        (self.disk, self.deleted) = (written.disk, false);
        self.path = Some(req.path.clone());
        self.disk_encoding = req.point.encoding;
        // Find Results saved as a file: its edits ask to be saved from now on
        self.scratch = false;
        self.read_only = false;
        self.saved = Some(req.point);
        self.refresh_dirty();
    }

    /// `req` failed as `failure` says: a write in place cut the file short, so its stamp is
    /// taken (not a change to offer to reload), and no text of the document is what it holds
    /// now; untouched, a change from outside is still asked about.
    pub fn fail_save(&mut self, req: &SaveRequest, failure: &SaveFailure) {
        if let Some(disk) = failure.disk {
            if self.path.as_ref() == Some(&req.path) {
                self.disk = disk;
                self.forget_saved_point();
            }
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "buffer was loaded lossily; use Save As"));
        }
        let path =
            self.path.clone().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "untitled buffer has no path"))?;
        self.save_to(path)
    }

    pub fn save_to(&mut self, path: PathBuf) -> io::Result<()> {
        self.save_to_with_disk_encoding(path, None)
    }

    /// Capture, write and finish at once, on this thread.
    pub fn save_to_with_disk_encoding(&mut self, path: PathBuf, previous: Option<Encoding>) -> io::Result<()> {
        let req = self.capture_save(path, previous)?;
        match req.write() {
            Ok(written) => {
                self.finish_save(&req, &written);
                Ok(())
            }
            Err(failure) => {
                self.fail_save(&req, &failure);
                Err(failure.error)
            }
        }
    }
}
