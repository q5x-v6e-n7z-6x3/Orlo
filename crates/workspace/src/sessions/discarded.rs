//! Bounded rescue of explicitly discarded untitled text, independent of hot-exit sessions.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 50;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_STORE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_AGE_SECONDS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub created: u64,
    pub bytes: u64,
}

#[derive(Default)]
struct Shared {
    storage: Mutex<u64>,
    generation: AtomicU64,
    revision: AtomicU64,
}
type Guard = Arc<Shared>;

/// Worker-issued admission stamp. Checking it never locks storage or touches the filesystem.
/// Any serialized capture, deletion, Clear or retention removal invalidates older results.
pub struct Publication {
    shared: Guard,
    revision: u64,
    pub generation: u64,
    expires: u64,
}
impl Publication {
    pub fn is_current(&self) -> bool {
        self.is_current_at(now())
    }

    fn is_current_at(&self, now: u64) -> bool {
        now < self.expires
            && self.shared.generation.load(Ordering::Acquire) == self.generation
            && self.shared.revision.load(Ordering::Acquire) == self.revision
    }
}

pub struct Catalog {
    pub publication: Publication,
    pub entries: Vec<(Entry, io::Result<String>)>,
}

pub struct Loaded {
    pub publication: Publication,
    /// A safe existing record can still contain an invalid payload, which remains deletable.
    pub text: io::Result<String>,
}
static GENERATIONS: LazyLock<Mutex<HashMap<PathBuf, Guard>>> = LazyLock::new(Default::default);

/// Capture tickets and Clear share a per-directory generation and publication lock.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    generation: Guard,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        let generation = GENERATIONS.lock().unwrap_or_else(|e| e.into_inner()).entry(root.clone()).or_default().clone();
        Self { root, generation }
    }

    pub fn current() -> Self {
        Self::new(super::support_dir().join("DiscardedUntitled"))
    }

    pub fn generation(&self) -> u64 {
        self.generation.generation.load(Ordering::Acquire)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_storage_lock_for_test(&self, held: impl FnOnce()) {
        let _guard = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        held();
    }

    /// Whole texts only. Oversize input refuses the batch before any file is published.
    pub fn capture(&self, generation: u64, texts: Vec<String>) -> io::Result<()> {
        self.capture_at(generation, texts, now())
    }

    fn capture_at(&self, generation: u64, texts: Vec<String>, now: u64) -> io::Result<()> {
        let current = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        if *current != generation {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Discard history was cleared; capture cancelled"));
        }
        if texts.iter().any(|text| text.len() > MAX_TEXT_BYTES) {
            return Err(invalid("Rescue keeps drafts up to 1 MiB; this draft is larger"));
        }
        let texts: Vec<_> = texts.into_iter().filter(|text| !text.is_empty()).collect();
        if texts.is_empty() {
            return Ok(());
        }
        self.directory(true)?;
        // Refuse damaged/unsafe existing storage before publishing any of this batch.
        self.entries()?;
        self.invalidate_publications();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut published = Vec::new();
        let result = (|| {
            for text in texts.into_iter().rev().take(MAX_ENTRIES).rev() {
                let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
                let id = format!("{now:020}-{nanos}-{}-{serial}.txt", std::process::id());
                let temporary = self.root.join(format!("{id}.tmp"));
                let mut file = orlo_project::io::create_private(&temporary)?;
                let result = (|| {
                    file.write_all(text.as_bytes())?;
                    file.sync_all()?;
                    std::fs::rename(&temporary, self.root.join(&id))
                })();
                if result.is_err() {
                    let _ = std::fs::remove_file(&temporary);
                }
                result?;
                published.push(id);
            }
            self.prune(now)?;
            File::open(&self.root)?.sync_all()
        })();
        if result.is_err() {
            for id in published {
                let _ = std::fs::remove_file(self.root.join(id));
            }
        }
        result
    }

    /// Listing and retention inspect metadata only; summaries and selected reads are separate.
    pub fn list(&self) -> io::Result<(u64, Vec<Entry>)> {
        self.list_at(now())
    }

    fn list_at(&self, now: u64) -> io::Result<(u64, Vec<Entry>)> {
        let generation = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        Ok((*generation, self.prune(now)?))
    }

    /// Metadata, permissions, pruning and payload reads all complete under the worker's lock.
    pub fn load(&self, generation: u64, id: &str) -> io::Result<Loaded> {
        let current = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        let entry = self.validate_locked(*current, generation, id)?;
        Ok(Loaded {
            publication: self.publication(*current, entry.created.saturating_add(MAX_AGE_SECONDS)),
            text: self.read_text(id),
        })
    }

    pub fn restore(&self, generation: u64, id: &str) -> io::Result<String> {
        self.load(generation, id)?.text
    }

    fn read_text(&self, id: &str) -> io::Result<String> {
        let mut text = String::new();
        self.open(id)?.take(MAX_TEXT_BYTES as u64 + 1).read_to_string(&mut text)?;
        if text.len() > MAX_TEXT_BYTES {
            return Err(invalid("Discarded draft exceeds the 1 MiB limit"));
        }
        Ok(text)
    }

    /// One serialized worker catalog; no derived text survives in persistent storage.
    pub fn catalog(&self) -> io::Result<Catalog> {
        let generation = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        let entries = self.prune(now())?;
        let expires = entries.iter().map(|e| e.created.saturating_add(MAX_AGE_SECONDS)).min().unwrap_or(u64::MAX);
        let entries = entries
            .into_iter()
            .map(|entry| {
                let summary = self.read_summary(&entry.id);
                (entry, summary)
            })
            .collect();
        Ok(Catalog { publication: self.publication(*generation, expires), entries })
    }

    fn publication(&self, generation: u64, expires: u64) -> Publication {
        Publication {
            shared: self.generation.clone(),
            generation,
            revision: self.generation.revision.load(Ordering::Acquire),
            expires,
        }
    }

    fn invalidate_publications(&self) {
        self.generation.revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Explicit storage validation for backend callers; UI admission uses Publication instead.
    pub fn validate(&self, generation: u64, id: &str) -> io::Result<()> {
        let current = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        self.validate_locked(*current, generation, id).map(|_| ())
    }

    fn validate_locked(&self, current: u64, generation: u64, id: &str) -> io::Result<Entry> {
        if current != generation {
            return Err(invalid("Discard history changed; open the rescue picker again"));
        }
        self.prune(now())?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "This discarded draft is missing or expired"))
    }

    /// A bounded search excerpt, loaded once when opening the picker, never while filtering.
    pub fn summary(&self, generation: u64, id: &str) -> io::Result<String> {
        let current = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        self.validate_locked(*current, generation, id)?;
        self.read_summary(id)
    }

    fn read_summary(&self, id: &str) -> io::Result<String> {
        let file = self.open(id)?;
        let length = file.metadata()?.len();
        if length > MAX_TEXT_BYTES as u64 {
            return Err(invalid("Discarded draft exceeds the 1 MiB limit"));
        }
        let mut bytes = Vec::new();
        file.take(1024).read_to_end(&mut bytes)?;
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) if error.error_len().is_none() && length > 1024 => {
                std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap()
            }
            Err(_) => return Err(invalid("Discarded draft contains malformed UTF-8")),
        };
        Ok(text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect())
    }

    /// Delete exactly one validated rescue record; queued captures retain their generation.
    pub fn delete(&self, generation: u64, id: &str) -> io::Result<()> {
        let current = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        self.validate_locked(*current, generation, id)?;
        self.invalidate_publications();
        std::fs::remove_file(self.root.join(id))?;
        File::open(&self.root)?.sync_all()
    }

    /// Invalidate queued captures, then unlink only rescue records (including crash remnants).
    pub fn clear(&self) -> io::Result<()> {
        let mut generation = self.generation.storage.lock().unwrap_or_else(|e| e.into_inner());
        self.invalidate_publications();
        *generation = generation.wrapping_add(1);
        self.generation.generation.store(*generation, Ordering::Release);
        if !self.directory(false)? {
            return Ok(());
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if created(&name).is_some() || name.strip_suffix(".tmp").and_then(created).is_some() {
                std::fs::remove_file(entry.path())?;
            }
        }
        File::open(&self.root)?.sync_all()
    }

    fn directory(&self, create: bool) -> io::Result<bool> {
        if create {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.root)?;
        }
        match orlo_project::io::make_private_directory(&self.root) {
            Ok(()) => Ok(true),
            Err(e) if !create && e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn open(&self, id: &str) -> io::Result<File> {
        let file =
            OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(self.root.join(id))?;
        let meta = file.metadata()?;
        // SAFETY: geteuid takes no arguments and cannot fail.
        if !meta.is_file() || meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0
        {
            return Err(invalid("Discarded draft must be a private regular file"));
        }
        Ok(file)
    }

    fn entries(&self) -> io::Result<Vec<Entry>> {
        if !self.directory(false)? {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if let Some(created) = created(&id) {
                entries.push(Entry { bytes: self.open(&id)?.metadata()?.len(), id, created });
            } else if id.strip_suffix(".tmp").and_then(created).is_some() {
                // No writer can be active under this lock; these are interrupted staging files.
                std::fs::remove_file(entry.path())?;
            }
        }
        entries.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(entries)
    }

    fn prune(&self, now: u64) -> io::Result<Vec<Entry>> {
        let mut kept = Vec::new();
        let mut damaged = Vec::new();
        let mut bytes = 0;
        let remove = |entry: Entry| {
            self.invalidate_publications();
            std::fs::remove_file(self.root.join(entry.id))
        };
        for entry in self.entries()? {
            if now.saturating_sub(entry.created) >= MAX_AGE_SECONDS {
                remove(entry)?;
            } else if entry.bytes > MAX_TEXT_BYTES as u64 {
                // Externally damaged artifacts never spend usable history's count/byte budget.
                damaged.push(entry);
            } else if kept.len() >= MAX_ENTRIES || bytes + entry.bytes > MAX_STORE_BYTES {
                remove(entry)?;
            } else {
                bytes += entry.bytes;
                kept.push(entry);
            }
        }
        let room = MAX_ENTRIES - kept.len();
        for (n, entry) in damaged.into_iter().enumerate() {
            if n < room {
                kept.push(entry);
            } else {
                remove(entry)?;
            }
        }
        kept.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(kept)
    }
}

fn created(name: &str) -> Option<u64> {
    let mut parts = name.strip_suffix(".txt")?.split('-');
    let time = parts.next()?.parse().ok()?;
    for _ in 0..3 {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    parts.next().is_none().then_some(time)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Temp(Store);
    impl Temp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "orlo-discarded-{}-{}-{id}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            Self(Store::new(root))
        }
        fn capture(&self, text: &str) {
            self.0.capture(self.0.generation(), vec![text.into()]).unwrap();
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.root);
        }
    }

    #[test]
    fn empty_default_unicode_and_private_files_round_trip() {
        let t = Temp::new();
        assert_eq!(t.0.list().unwrap(), (0, Vec::new()));
        assert!(!t.0.root.exists(), "listing creates no store");
        t.capture("救助 🦀 e\u{301}\0\n");
        let (generation, entries) = t.0.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(t.0.restore(generation, &entries[0].id).unwrap(), "救助 🦀 e\u{301}\0\n");
        assert_eq!(std::fs::metadata(&t.0.root).unwrap().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(t.0.root.join(&entries[0].id)).unwrap().mode() & 0o777, 0o600);
        assert_eq!(t.0.list().unwrap().1, entries, "restoring does not consume history");
    }

    #[test]
    fn retention_bounds_whole_unicode_text_by_count_bytes_and_age() {
        let t = Temp::new();
        for i in 0..51 {
            t.capture(&format!("draft {i}"));
        }
        let (generation, entries) = t.0.list().unwrap();
        assert_eq!(entries.len(), 50);
        assert_eq!(t.0.restore(generation, &entries[0].id).unwrap(), "draft 50");
        assert!(!entries.iter().any(|entry| t.0.restore(generation, &entry.id).unwrap() == "draft 0"));
        t.0.clear().unwrap();
        let unicode = "🦀".repeat(262_144);
        let captured = now();
        for _ in 0..9 {
            t.0.capture_at(t.0.generation(), vec![unicode.clone()], captured).unwrap();
        }
        let (generation, entries) = t.0.list().unwrap();
        assert_eq!(entries.len(), 8);
        assert_eq!(entries.iter().map(|e| e.bytes).sum::<u64>(), 8_388_608);
        assert_eq!(t.0.restore(generation, &entries[0].id).unwrap(), unicode);
        let mut too_large = unicode;
        too_large.push('é');
        assert!(t.0.capture(generation, vec![too_large]).is_err());
        assert_eq!(t.0.list().unwrap().1, entries, "oversize refuses before changing retention");
        assert_eq!(t.0.list_at(entries[0].created + 2_591_999).unwrap().1.len(), 8);
        assert!(t.0.list_at(entries[0].created + 2_592_000).unwrap().1.is_empty());
        assert!(std::fs::read_dir(&t.0.root).unwrap().next().is_none());
    }

    #[test]
    fn clear_invalidates_a_delayed_capture_and_picker_but_keeps_other_files() {
        let t = Temp::new();
        t.capture("discarded");
        let (generation, entries) = t.0.list().unwrap();
        let other = t.0.root.join("not-a-rescue.txt");
        std::fs::write(&other, "unrelated").unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let writer = t.0.clone();
        let delayed = std::thread::spawn(move || {
            receive.recv().unwrap();
            writer.capture(generation, vec!["queued before clear".into()])
        });
        t.0.clear().unwrap();
        send.send(()).unwrap();
        assert_eq!(delayed.join().unwrap().unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(t.0.restore(generation, &entries[0].id).is_err());
        assert!(t.0.list().unwrap().1.is_empty());
        assert_eq!(std::fs::read_to_string(other).unwrap(), "unrelated");
        t.capture("new discard after clear");
        let (generation, entries) = t.0.list().unwrap();
        assert_eq!(t.0.restore(generation, &entries[0].id).unwrap(), "new discard after clear");
    }

    #[test]
    fn no_follow_or_special_files_and_failed_capture_leave_originals_alone() {
        let outside = Temp::new();
        outside.capture("outside");
        let external = outside.0.root.join(&outside.0.list().unwrap().1[0].id);
        let t = Temp::new();
        symlink(&outside.0.root, &t.0.root).unwrap();
        assert!(t.0.capture(0, vec!["must not escape".into()]).is_err());
        assert!(t.0.clear().is_err());
        std::fs::remove_file(&t.0.root).unwrap();
        t.capture("retained");
        let id = t.0.list().unwrap().1[0].id.clone();
        let leaf = t.0.root.join(&id);
        std::fs::remove_file(&leaf).unwrap();
        symlink(&external, &leaf).unwrap();
        assert!(t.0.restore(0, &id).is_err());
        assert!(t.0.capture(0, vec!["refused".into()]).is_err());
        t.0.clear().unwrap();
        assert_eq!(std::fs::read_to_string(&external).unwrap(), "outside");
        std::fs::hard_link(&external, &leaf).unwrap();
        assert!(t.0.list().is_err());
        t.0.clear().unwrap();
        std::fs::create_dir(&leaf).unwrap();
        assert!(t.0.list().is_err());
        assert!(t.0.clear().is_err(), "failed clear must report an error");
        assert_eq!(std::fs::read_to_string(&external).unwrap(), "outside");
    }

    #[test]
    fn missing_corrupt_and_oversize_records_never_restore_as_empty_success() {
        let t = Temp::new();
        t.capture("text");
        let id = t.0.list().unwrap().1[0].id.clone();
        let path = t.0.root.join(&id);
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(t.0.restore(0, &id).is_err());
        assert!(t.0.restore(0, "../../outside").is_err());
        OpenOptions::new().write(true).open(&path).unwrap().set_len(1_048_577).unwrap();
        assert_eq!(t.0.list().unwrap().1.len(), 1);
        assert!(t.0.restore(0, &id).is_err());
        std::fs::remove_file(path).unwrap();
        assert_eq!(t.0.restore(0, &id).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn publication_tokens_cover_capture_delete_clear_pruning_and_exact_expiry() {
        let t = Temp::new();
        t.capture("first");
        let (generation, entries) = t.0.list().unwrap();
        let id = &entries[0].id;
        let loaded = t.0.load(generation, id).unwrap();
        let catalog = t.0.catalog().unwrap();
        assert!(loaded.publication.is_current() && catalog.publication.is_current());
        assert!(loaded.publication.is_current_at(entries[0].created + MAX_AGE_SECONDS - 1));
        assert!(!loaded.publication.is_current_at(entries[0].created + MAX_AGE_SECONDS));
        t.capture("unrelated capture");
        assert!(!loaded.publication.is_current() && !catalog.publication.is_current());
        let loaded = t.0.load(generation, id).unwrap();
        t.0.delete(generation, id).unwrap();
        assert!(!loaded.publication.is_current());
        assert_eq!(t.0.generation(), generation, "Delete preserves unrelated capture tickets");
        let id = t.0.list().unwrap().1[0].id.clone();
        let loaded = t.0.load(generation, &id).unwrap();
        t.0.list_at(now() + MAX_AGE_SECONDS).unwrap();
        assert!(!loaded.publication.is_current(), "retention removal invalidates loaded text");
        t.capture("before Clear");
        let catalog = t.0.catalog().unwrap();
        t.0.clear().unwrap();
        assert!(!catalog.publication.is_current());
        assert_eq!(t.0.generation(), generation + 1);
    }

    #[test]
    fn oversized_artifacts_have_no_priority_over_fifty_usable_records() {
        let t = Temp::new();
        t.0.capture(0, (0..50).map(|n| format!("valid {n}")).collect()).unwrap();
        let before = t.0.list().unwrap().1;
        let bad = t.0.root.join(format!("{:020}-9999999999999999999-1-1.txt", now()));
        let file = orlo_project::io::create_private(&bad).unwrap();
        file.set_len(MAX_STORE_BYTES).unwrap();
        assert_eq!(t.0.list().unwrap().1, before);
        assert!(!bad.exists(), "bounded invalid overflow removes only the unusable artifact");
        for entry in before {
            assert!(t.0.restore(0, &entry.id).unwrap().starts_with("valid "));
        }
    }

    #[test]
    fn oversized_record_cannot_prune_valid_full_store_siblings() {
        let t = Temp::new();
        let text = "🦀".repeat(MAX_TEXT_BYTES / 4);
        t.0.capture(0, vec![text.clone(); 8]).unwrap();
        let (_, before) = t.0.list().unwrap();
        let bad = &before[0].id;
        OpenOptions::new().write(true).open(t.0.root.join(bad)).unwrap().set_len(MAX_STORE_BYTES).unwrap();
        let (_, listed) = t.0.list().unwrap();
        for entry in &before[1..] {
            assert!(t.0.root.join(&entry.id).is_file(), "damaged newest record must not unlink valid siblings");
            assert_eq!(t.0.restore(0, &entry.id).unwrap(), text);
        }
        assert!(listed.iter().any(|e| &e.id == bad), "invalid payload remains individually deletable");
        assert!(t.0.summary(0, bad).is_err());
        assert!(t.0.restore(0, bad).is_err());
        t.0.delete(0, bad).unwrap();
        assert!(!t.0.root.join(bad).exists());
        assert_eq!(t.0.list().unwrap().1.len(), 7);
        for entry in &before[1..] {
            assert_eq!(t.0.restore(0, &entry.id).unwrap(), text);
        }
    }

    #[test]
    fn fifty_escaped_unicode_summaries_are_bounded_without_persistent_index() {
        let t = Temp::new();
        let text = "quoted \" slash \\ controls\0\n\t 救助 🦀 e\u{301} ".repeat(100);
        t.0.capture(0, (0..50).map(|n| format!("{n}: {text}")).collect()).unwrap();
        let (generation, entries) = t.0.list().unwrap();
        assert_eq!(entries.len(), 50);
        let summaries: Vec<_> = entries.iter().map(|e| t.0.summary(generation, &e.id).unwrap()).collect();
        assert!(summaries.iter().all(|s| s.len() <= 1024 && !s.chars().any(char::is_control)));
        assert!(summaries.iter().all(|s| s.contains("救助 🦀") && s.contains('"') && s.contains('\\')));
        let escaped = serde_json::to_vec(&summaries).unwrap();
        assert_eq!(serde_json::from_slice::<Vec<String>>(&escaped).unwrap(), summaries);
        assert!(escaped.len() <= 50 * (6 * 1024 + 3) + 2);
        assert_eq!(std::fs::read_dir(&t.0.root).unwrap().count(), 50, "only original records, no index/schema");
        for entry in entries {
            assert!(t.0.restore(generation, &entry.id).unwrap().ends_with(&text));
        }
    }

    #[test]
    fn thirty_day_boundary_prunes_actual_record_and_pending_reads_cannot_return() {
        let t = Temp::new();
        let captured = now();
        t.0.capture_at(0, vec!["thirty days".into()], captured).unwrap();
        assert_eq!(t.0.list_at(captured + 2_591_999).unwrap().1.len(), 1);
        assert!(t.0.list_at(captured + 2_592_000).unwrap().1.is_empty());
        assert!(std::fs::read_dir(&t.0.root).unwrap().next().is_none());
        t.capture("queued preview");
        let (generation, entries) = t.0.list().unwrap();
        let id = entries[0].id.clone();
        let loaded = t.0.restore(generation, &id).unwrap();
        t.0.delete(generation, &id).unwrap();
        assert_eq!(loaded, "queued preview");
        assert!(t.0.validate(generation, &id).is_err(), "already-read text cannot publish after deletion");
        t.capture("queued before clear");
        let id = t.0.list().unwrap().1[0].id.clone();
        let _loaded = t.0.restore(generation, &id).unwrap();
        t.0.clear().unwrap();
        assert!(t.0.validate(generation, &id).is_err());
    }

    #[test]
    fn fresh_store_search_preview_delete_and_corrupt_siblings() {
        let t = Temp::new();
        t.capture("救助 🦀 e\u{301}\n");
        let fresh = Store::new(t.0.root.clone());
        let (generation, entries) = fresh.list().unwrap();
        let good = entries[0].id.clone();
        assert!(fresh.summary(generation, &good).unwrap().contains("救助 🦀"));
        t.capture("corrupt");
        let bad = fresh.list().unwrap().1[0].id.clone();
        std::fs::write(fresh.root.join(&bad), [0xff]).unwrap();
        assert!(fresh.summary(generation, &bad).is_err());
        assert!(fresh.restore(generation, &bad).is_err());
        assert_eq!(fresh.restore(generation, &good).unwrap(), "救助 🦀 e\u{301}\n");
        assert!(fresh.delete(generation, "../../outside").is_err());
        fresh.delete(generation, &bad).unwrap();
        assert!(fresh.restore(generation, &bad).is_err());
        assert!(fresh.validate(generation, &bad).is_err());
        assert_eq!(fresh.list().unwrap().1.len(), 1);
        fresh.clear().unwrap();
        assert!(fresh.summary(generation, &good).is_err());
        assert!(fresh.delete(generation, &good).is_err());
    }

    #[test]
    fn bounded_summary_preserves_utf8_boundary_and_does_not_cache_payload() {
        let t = Temp::new();
        let text = format!("{}🦀 tail", "a".repeat(1023));
        t.capture(&text);
        let (generation, entries) = t.0.list().unwrap();
        let id = &entries[0].id;
        assert_eq!(t.0.summary(generation, id).unwrap(), "a".repeat(1023));
        assert_eq!(t.0.restore(generation, id).unwrap(), text);
        t.0.delete(generation, id).unwrap();
        assert!(t.0.restore(generation, id).is_err());
        t.0.capture(generation, vec!["after delete".into()]).unwrap();
        assert_eq!(t.0.list().unwrap().1.len(), 1);
    }

    #[test]
    fn empty_payloads_do_not_create_rescue_records() {
        let t = Temp::new();
        t.capture("");
        assert!(t.0.list().unwrap().1.is_empty());
        assert!(!t.0.root.exists());
    }

    #[test]
    fn old_session_recovery_and_clear_are_independent() {
        let t = Temp::new();
        t.capture("discarded only");
        let session = t.0.root.join("Session.json");
        let old =
            r#"{"windows":[{"groups":[{"views":[{"path":"","unsaved":{"text":"old draft 界","dirty":true}}]}]}]}"#;
        std::fs::write(&session, old).unwrap();
        let (loaded, status, _) = super::super::load_with(&session);
        assert_eq!(status, super::super::Status::Clean);
        assert_eq!(loaded.windows[0].groups[0].views[0].unsaved.as_ref().unwrap().text, "old draft 界");
        t.0.clear().unwrap();
        assert_eq!(std::fs::read_to_string(session).unwrap(), old);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn inherited_acls_never_expose_rescue_text() {
        let t = Temp::new();
        std::fs::create_dir(&t.0.root).unwrap();
        let result = std::process::Command::new("chmod")
            .args(["+a", "everyone allow read,readattr,readextattr,readsecurity,file_inherit,directory_inherit"])
            .arg(&t.0.root)
            .status()
            .unwrap();
        assert!(result.success());
        t.capture("private draft");
        let entry = t.0.list().unwrap().1.remove(0);
        for path in [&t.0.root, &t.0.root.join(entry.id)] {
            let output = std::process::Command::new("ls").arg("-lde").arg(path).output().unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 1, "inherited ACL removed");
        }
        std::fs::set_permissions(&t.0.root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}
