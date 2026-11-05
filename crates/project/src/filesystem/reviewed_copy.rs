//! Copying one regular file across a folder comparison, as the bytes the comparison showed;
//! filesystem work runs off the UI thread.

use super::open_at;
use crate::comparison::{self, DiskSnapshot};
use std::ffi::{CString, OsStr};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
type WriteHook = fn(&Path) -> io::Result<()>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static AFTER_PARENT: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
    static BEFORE_PUBLISH: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
    static AFTER_WRITE: std::cell::Cell<Option<WriteHook>> = const { std::cell::Cell::new(None) };
}

fn check(stop: &AtomicBool) -> io::Result<()> {
    if stop.load(Ordering::Relaxed) {
        Err(io::Error::other("Copy cancelled"))
    } else {
        Ok(())
    }
}

fn id(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn name(path: &OsStr) -> io::Result<CString> {
    super::c_path(Path::new(path))
}

/// The (device, inode) of `child` in `parent`, never through a final link.
fn id_at(parent: &File, child: &OsStr) -> io::Result<(u64, u64)> {
    let child = name(child)?;
    let mut metadata = std::mem::MaybeUninit::uninit();
    // SAFETY: fstatat initializes stat on success and never follows the final link.
    if unsafe { libc::fstatat(parent.as_raw_fd(), child.as_ptr(), metadata.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) }
        != 0
    {
        return Err(io::Error::last_os_error());
    }
    let metadata = unsafe { metadata.assume_init() };
    Ok((metadata.st_dev as u64, metadata.st_ino))
}

fn absent(parent: &File, child: &OsStr) -> io::Result<()> {
    match id_at(parent, child) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Err(super::exists(Path::new(child))),
    }
}

/// The comparison snapshot of the file `relative` names under `parents`.
fn read(
    parents: &Parents,
    relative: &Path,
    path: &Path,
    hint: Option<crate::encoding::Encoding>,
    stop: &AtomicBool,
) -> io::Result<DiskSnapshot> {
    let file = parents.open_file(relative)?;
    let metadata = file.metadata()?;
    comparison::read_opened_with_hint(path, &metadata, file, hint, stop).map_err(|e| io::Error::other(e.to_string()))
}

/// Pinned physical ancestry, including the selected root and existing descendant parents.
struct Parents {
    root: PathBuf,
    root_index: usize,
    paths: Vec<PathBuf>,
    handles: Vec<File>,
}

impl Parents {
    fn capture(root: &Path, relative: &Path, allow_missing: bool) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(root)?;
        if !metadata.is_dir() {
            return Err(io::Error::other("Choose a real directory root"));
        }
        let physical = std::fs::canonicalize(root)?;
        let mut result = Self {
            root: root.to_path_buf(),
            root_index: 0,
            paths: vec![PathBuf::from("/")],
            handles: vec![OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open("/")?],
        };
        for component in physical.components() {
            if let Component::Normal(child) = component {
                result.push(child)?;
            }
        }
        result.root_index = result.handles.len() - 1;
        if id(&result.last().metadata()?) != id(&metadata) {
            return Err(io::Error::other("A selected root changed"));
        }
        for component in relative.parent().unwrap_or(Path::new("")).components() {
            let Component::Normal(child) = component else {
                return Err(io::Error::other("Copy needs a relative file path"));
            };
            match result.push(child) {
                Ok(()) => (),
                Err(e) if allow_missing && e.kind() == io::ErrorKind::NotFound => break,
                Err(e) => return Err(e),
            }
        }
        result.validate()?;
        Ok(result)
    }

    fn root_id(&self) -> io::Result<(u64, u64)> {
        Ok(id(&self.handles[self.root_index].metadata()?))
    }
    fn physical_root(&self) -> &Path {
        &self.paths[self.root_index]
    }
    fn last(&self) -> &File {
        self.handles.last().unwrap()
    }
    fn path(&self) -> &Path {
        self.paths.last().unwrap()
    }
    fn push(&mut self, child: &OsStr) -> io::Result<()> {
        let file = open_at(self.last(), child, libc::O_RDONLY | libc::O_DIRECTORY)?;
        self.paths.push(self.path().join(child));
        self.handles.push(file);
        Ok(())
    }
    /// The file `relative` names, opened through the last pinned parent.
    fn open_file(&self, relative: &Path) -> io::Result<File> {
        open_at(self.last(), relative.file_name().unwrap(), libc::O_RDONLY)
    }
    fn clone_handles(&self) -> io::Result<Self> {
        Ok(Self {
            root: self.root.clone(),
            root_index: self.root_index,
            paths: self.paths.clone(),
            handles: self.handles.iter().map(File::try_clone).collect::<io::Result<_>>()?,
        })
    }
    fn validate(&self) -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(&self.root)?;
        if !metadata.is_dir() || id(&metadata) != self.root_id()? {
            return Err(io::Error::other("A selected root changed"));
        }
        for i in 1..self.handles.len() {
            let current =
                open_at(&self.handles[i - 1], self.paths[i].file_name().unwrap(), libc::O_RDONLY | libc::O_DIRECTORY)?;
            if id(&current.metadata()?) != id(&self.handles[i].metadata()?) {
                return Err(io::Error::other(format!("Parent changed: {}", self.paths[i].display())));
            }
        }
        Ok(())
    }
}

pub struct ReviewedCopy {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub snapshot: DiskSnapshot,
    pub missing: Vec<PathBuf>,
    source_parents: Parents,
    destination_parents: Parents,
    relative: PathBuf,
}

impl ReviewedCopy {
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(roots: [PathBuf; 2], relative: PathBuf, source: usize, stop: &AtomicBool) -> Result<Self, String> {
        Self::capture_with_hint(roots, relative, source, None, stop)
    }

    pub fn capture_with_hint(
        roots: [PathBuf; 2],
        relative: PathBuf,
        source: usize,
        hint: Option<crate::encoding::Encoding>,
        stop: &AtomicBool,
    ) -> Result<Self, String> {
        let capture = || -> io::Result<Self> {
            check(stop)?;
            if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
                return Err(io::Error::other("Copy needs a relative file path"));
            }
            let source_parents = Parents::capture(&roots[source], &relative, false)?;
            let destination_parents = Parents::capture(&roots[1 - source], &relative, true)?;
            if source_parents.root_id()? == destination_parents.root_id()? {
                return Err(io::Error::other("Choose two different underlying folders"));
            }
            let path = roots[source].join(&relative);
            let snapshot = read(&source_parents, &relative, &path, hint, stop)?;
            let destination = roots[1 - source].join(&relative);
            let physical_parent = destination_parents.physical_root().join(relative.parent().unwrap());
            let mut missing = Vec::new();
            let rest = physical_parent.strip_prefix(destination_parents.path()).map_err(io::Error::other)?;
            let mut next = destination_parents.path().to_path_buf();
            for part in rest.components() {
                next.push(part);
                missing.push(next.clone());
            }
            let review =
                Self { source: path, destination, snapshot, missing, source_parents, destination_parents, relative };
            review.validate(stop)?;
            Ok(review)
        };
        capture().map_err(|e| e.to_string())
    }

    pub fn reservations(&self) -> Vec<PathBuf> {
        let parent = &self.destination_parents;
        let mut paths = vec![
            self.source.clone(),
            self.source_parents.physical_root().join(&self.relative),
            self.destination.clone(),
            parent.physical_root().join(&self.relative),
        ];
        if let Some(first) = self.missing.first() {
            paths.push(first.clone());
            let relative = first.strip_prefix(parent.physical_root()).unwrap();
            paths.push(parent.root.join(relative));
        }
        paths.sort();
        paths.dedup();
        paths
    }

    /// Whether `metadata` still has the reviewed source's device and full stamp.
    fn reviewed(&self, metadata: &Metadata) -> bool {
        comparison::identity(metadata) == (self.snapshot.device, self.snapshot.stamp)
    }

    /// `inspect` between stop checks and validations of the source's pinned parents.
    fn pinned_source(&self, stop: &AtomicBool, inspect: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        check(stop)?;
        self.source_parents.validate()?;
        inspect()?;
        self.source_parents.validate()?;
        check(stop)
    }

    fn source_intact(&self, stop: &AtomicBool) -> io::Result<()> {
        self.pinned_source(stop, || {
            let hint = Some(self.snapshot.text.format.encoding);
            let current = read(&self.source_parents, &self.relative, &self.source, hint, stop)?;
            if current.device != self.snapshot.device
                || current.stamp != self.snapshot.stamp
                || current.bytes != self.snapshot.bytes
            {
                return Err(io::Error::other("The reviewed source changed on disk"));
            }
            Ok(())
        })
    }

    fn copy_metadata_to(&self, destination: &File, stop: &AtomicBool) -> io::Result<()> {
        self.source_metadata_intact(stop)?;
        let source = self.source_parents.open_file(&self.relative)?;
        if !self.reviewed(&source.metadata()?) {
            return Err(io::Error::other("The reviewed source changed before metadata copying"));
        }
        // SAFETY: both descriptors are pinned regular files; metadata includes ACLs and security xattrs.
        if unsafe {
            libc::fcopyfile(source.as_raw_fd(), destination.as_raw_fd(), std::ptr::null_mut(), libc::COPYFILE_METADATA)
        } != 0
        {
            return Err(io::Error::other(format!("Cannot preserve source metadata: {}", io::Error::last_os_error())));
        }
        if !self.reviewed(&source.metadata()?) {
            return Err(io::Error::other("The reviewed source changed while copying metadata"));
        }
        self.source_metadata_intact(stop)
    }

    fn source_metadata_intact(&self, stop: &AtomicBool) -> io::Result<()> {
        self.pinned_source(stop, || {
            let metadata = self.source_parents.open_file(&self.relative)?.metadata()?;
            if !metadata.is_file() || !self.reviewed(&metadata) {
                return Err(io::Error::other("The reviewed source changed after byte validation"));
            }
            Ok(())
        })
    }

    fn validate(&self, stop: &AtomicBool) -> io::Result<()> {
        self.source_intact(stop)?;
        self.destination_parents.validate()?;
        let next = self.missing.first().and_then(|p| p.file_name()).unwrap_or(self.relative.file_name().unwrap());
        absent(self.destination_parents.last(), next)
    }

    pub fn stage(self: &Arc<Self>, stop: &AtomicBool) -> Result<StagedCopy, String> {
        let parents = self.destination_parents.clone_handles().map_err(|e| e.to_string())?;
        let mut staged = StagedCopy { review: self.clone(), parents, stage: None, copy: None, copy_stamp: None };
        let result = (|| -> io::Result<()> {
            self.validate(stop)?;
            for path in &self.missing {
                check(stop)?;
                mkdir_at(staged.parents.last(), path.file_name().unwrap(), 0o755)?;
                staged.parents.push(path.file_name().unwrap()).map_err(|e| {
                    io::Error::other(format!(
                        "Created directory left at {} (could not pin it safely): {e}",
                        path.display()
                    ))
                })?;
                #[cfg(any(test, feature = "test-support"))]
                if let Some(hook) = AFTER_PARENT.take() {
                    hook(path);
                }
            }
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let stage_name = loop {
                check(stop)?;
                let value = format!(".orlo-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
                match mkdir_at(staged.parents.last(), OsStr::new(&value), 0o700) {
                    Ok(()) => break value,
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(e),
                }
            };
            let stage = open_at(staged.parents.last(), OsStr::new(&stage_name), libc::O_RDONLY | libc::O_DIRECTORY)
                .map_err(|e| {
                    io::Error::other(format!(
                        "Staging directory left at {} (could not pin it safely): {e}",
                        staged.parents.path().join(&stage_name).display()
                    ))
                })?;
            staged.stage = Some((stage_name, stage));
            staged.copy = Some(open_at(
                &staged.stage.as_ref().unwrap().1,
                OsStr::new("copy"),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            )?);
            let copy = staged.copy.as_mut().unwrap();
            for chunk in self.snapshot.bytes.chunks(64 << 10) {
                check(stop)?;
                copy.write_all(chunk)?;
            }
            self.copy_metadata_to(copy, stop)?;
            copy.sync_all()?;
            #[cfg(any(test, feature = "test-support"))]
            if let Some(hook) = AFTER_WRITE.take() {
                hook(&self.source)?;
            }
            staged.copy_stamp = Some(staged.validate_bytes(stop)?);
            Ok(())
        })();
        match result {
            Ok(()) => Ok(staged),
            Err(error) => Err(staged.cleanup(Some(error.to_string())).unwrap_err()),
        }
    }
}

#[must_use = "finish this copy on a worker to publish or clean up"]
pub struct StagedCopy {
    review: Arc<ReviewedCopy>,
    parents: Parents,
    stage: Option<(String, File)>,
    copy: Option<File>,
    copy_stamp: Option<(u64, crate::watch::Stamp)>,
}

impl StagedCopy {
    fn copy_metadata(&self) -> io::Result<Metadata> {
        self.parents.validate()?;
        absent(self.parents.last(), self.review.relative.file_name().unwrap())?;
        let (stage_name, stage) = self.stage.as_ref().unwrap();
        let opened = open_at(self.parents.last(), OsStr::new(stage_name), libc::O_RDONLY | libc::O_DIRECTORY)?;
        if id(&opened.metadata()?) != id(&stage.metadata()?) {
            return Err(io::Error::other("Private staging directory changed"));
        }
        let opened = self.copy.as_ref().unwrap().metadata()?;
        if id_at(stage, OsStr::new("copy"))? != id(&opened) {
            return Err(io::Error::other("Staged file changed"));
        }
        Ok(opened)
    }

    fn validate_bytes(&self, stop: &AtomicBool) -> io::Result<(u64, crate::watch::Stamp)> {
        self.review.source_intact(stop)?;
        let identity = comparison::identity(&self.copy_metadata()?);
        let file = self.copy.as_ref().unwrap();
        use std::os::unix::fs::FileExt;
        let mut offset = 0;
        let mut chunk = [0; 64 << 10];
        loop {
            check(stop)?;
            let count = file.read_at(&mut chunk, offset as u64)?;
            if count == 0 {
                break;
            }
            if self.review.snapshot.bytes.get(offset..offset + count) != Some(&chunk[..count]) {
                return Err(io::Error::other("Staged bytes changed"));
            }
            offset += count;
        }
        if offset != self.review.snapshot.bytes.len() || comparison::identity(&file.metadata()?) != identity {
            return Err(io::Error::other("Staged bytes changed"));
        }
        check(stop)?;
        Ok(identity)
    }

    pub fn finish(self, publish: bool, stop: &AtomicBool) -> Result<(), String> {
        let result = (|| -> io::Result<()> {
            if !publish {
                return Err(io::Error::other("Copy cancelled or its review expired"));
            }
            // Exact bytes were validated before the UI commit gate; unchanged full stamps keep publication short.
            self.review.source_metadata_intact(stop)?;
            if Some(comparison::identity(&self.copy_metadata()?)) != self.copy_stamp {
                return Err(io::Error::other("Staged file changed after byte validation"));
            }
            #[cfg(any(test, feature = "test-support"))]
            if let Some(hook) = BEFORE_PUBLISH.take() {
                hook(&self.review.destination);
            }
            self.parents.validate()?;
            let from = name(OsStr::new("copy"))?;
            let to = name(self.review.relative.file_name().unwrap())?;
            check(stop)?;
            // SAFETY: both live pinned directories and names; RENAME_EXCL cannot replace a raced destination.
            if unsafe {
                libc::renameatx_np(
                    self.stage.as_ref().unwrap().1.as_raw_fd(),
                    from.as_ptr(),
                    self.parents.last().as_raw_fd(),
                    to.as_ptr(),
                    libc::RENAME_EXCL,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        let published = result.is_ok();
        self.cleanup(result.err().map(|e| e.to_string())).map_err(|error| {
            if published {
                format!("Copied file, but cleanup failed: {error}")
            } else {
                error
            }
        })
    }

    fn cleanup(self, error: Option<String>) -> Result<(), String> {
        let mut errors: Vec<String> = error.into_iter().collect();
        if let Some((stage_name, stage)) = &self.stage {
            if let Some(file) = &self.copy {
                if let Err(e) = remove_owned(stage, OsStr::new("copy"), file, false) {
                    errors.push(format!(
                        "Staged file left at {}/copy: {e}",
                        self.parents.path().join(stage_name).display()
                    ));
                }
            }
            if let Err(e) = remove_owned(self.parents.last(), OsStr::new(stage_name), stage, true) {
                errors
                    .push(format!("Staging directory left at {}: {e}", self.parents.path().join(stage_name).display()));
            }
        }
        if !errors.is_empty() {
            for i in (self.review.destination_parents.handles.len()..self.parents.handles.len()).rev() {
                if let Err(e) = remove_owned(
                    &self.parents.handles[i - 1],
                    self.parents.paths[i].file_name().unwrap(),
                    &self.parents.handles[i],
                    true,
                ) {
                    errors.push(format!("Directory left at {}: {e}", self.parents.paths[i].display()));
                }
            }
            Err(errors.join("; "))
        } else {
            Ok(())
        }
    }
}

fn mkdir_at(parent: &File, child: &OsStr, mode: libc::mode_t) -> io::Result<()> {
    let child = name(child)?;
    // SAFETY: live pinned parent and NUL-terminated child.
    if unsafe { libc::mkdirat(parent.as_raw_fd(), child.as_ptr(), mode) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn remove_owned(parent: &File, child: &OsStr, owned: &File, directory: bool) -> io::Result<()> {
    let current = match id_at(parent, child) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    if current != id(&owned.metadata()?) {
        return Err(io::Error::other("the path is no longer owned by this copy"));
    }
    let child = name(child)?;
    // SAFETY: pinned parent and child; directories are removed only if empty, never recursively.
    if unsafe { libc::unlinkat(parent.as_raw_fd(), child.as_ptr(), if directory { libc::AT_REMOVEDIR } else { 0 }) }
        != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;

    fn reviewed(roots: &[PathBuf; 2], stop: &AtomicBool) -> Arc<ReviewedCopy> {
        Arc::new(ReviewedCopy::capture(roots.clone(), PathBuf::from("a/b/file"), 0, stop).unwrap())
    }

    fn fixture(name: &str) -> [PathBuf; 2] {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "orlo-reviewed-copy-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&base);
        let roots = [base.join("left"), base.join("right")];
        for root in &roots {
            std::fs::create_dir_all(root).unwrap();
        }
        std::fs::create_dir_all(roots[0].join("a/b")).unwrap();
        std::fs::write(roots[0].join("a/b/file"), "reviewed\r\n").unwrap();
        roots.map(|root| root.canonicalize().unwrap())
    }

    #[test]
    fn reviewed_copy_nested_success_and_cancel() {
        for publish in [false, true] {
            let roots = fixture(if publish { "success" } else { "cancel" });
            let stop = AtomicBool::new(false);
            let review = reviewed(&roots, &stop);
            assert_eq!(review.missing, [roots[1].join("a"), roots[1].join("a/b")]);
            let staged = review.stage(&stop).unwrap();
            let result = staged.finish(publish, &stop);
            if publish {
                result.unwrap();
                assert_eq!(std::fs::read(roots[1].join("a/b/file")).unwrap(), b"reviewed\r\n");
            } else {
                assert!(result.is_err());
                assert!(!roots[1].join("a").exists());
            }
            assert_eq!(std::fs::read(roots[0].join("a/b/file")).unwrap(), b"reviewed\r\n");
        }
    }
    #[test]
    fn reviewed_copy_rejects_source_destination_and_cancellation_races() {
        for race in [
            "source",
            "file",
            "symlink",
            "directory",
            "cancel",
            "stage-bytes",
            "source-restored-mtime",
            "stage-restored-mtime",
        ] {
            let roots = fixture(race);
            let stop = AtomicBool::new(false);
            let review = reviewed(&roots, &stop);
            let staged = review.stage(&stop).unwrap();
            let target = roots[1].join("a/b/file");
            match race {
                "source" => std::fs::write(&review.source, "changed!").unwrap(),
                "file" => std::fs::write(&target, "keep destination").unwrap(),
                "symlink" => std::os::unix::fs::symlink(&review.source, &target).unwrap(),
                "directory" => std::fs::create_dir(&target).unwrap(),
                "cancel" => stop.store(true, Ordering::Relaxed),
                "source-restored-mtime" | "stage-restored-mtime" => {
                    let file = if race == "source-restored-mtime" {
                        OpenOptions::new().write(true).open(&review.source).unwrap()
                    } else {
                        staged.copy.as_ref().unwrap().try_clone().unwrap()
                    };
                    let before = file.metadata().unwrap();
                    use std::os::unix::fs::FileExt;
                    file.write_at(b"changed!\r\n", 0).unwrap();
                    file.set_modified(before.modified().unwrap()).unwrap();
                    let after = file.metadata().unwrap();
                    assert_eq!(after.len(), before.len());
                    assert_eq!((after.mtime(), after.mtime_nsec()), (before.mtime(), before.mtime_nsec()));
                    assert_ne!((after.ctime(), after.ctime_nsec()), (before.ctime(), before.ctime_nsec()));
                }
                _ => {
                    use std::os::unix::fs::FileExt;
                    staged.copy.as_ref().unwrap().write_at(b"tampered!", 0).unwrap();
                }
            }
            assert!(staged.finish(true, &stop).is_err(), "{race}");
            match race {
                "file" => assert_eq!(std::fs::read(&target).unwrap(), b"keep destination"),
                "symlink" => assert!(std::fs::symlink_metadata(&target).unwrap().is_symlink()),
                "directory" => assert!(target.is_dir()),
                _ => assert!(!roots[1].join("a").exists(), "owned empty parents cleaned: {race}"),
            }
        }
    }

    #[test]
    fn reviewed_copy_never_follows_replaced_parents_or_roots() {
        for phase in ["before-stage", "before-publish", "source-parent", "root"] {
            let roots = fixture(phase);
            std::fs::create_dir_all(roots[1].join("a/b")).unwrap();
            let outside = roots[1].parent().unwrap().join("outside");
            std::fs::create_dir_all(outside.join("b")).unwrap();
            std::fs::write(outside.join("keep"), "keep").unwrap();
            let stop = AtomicBool::new(false);
            let review = reviewed(&roots, &stop);
            let staged = (phase != "before-stage").then(|| review.stage(&stop).unwrap());
            let replaced = match phase {
                "source-parent" => roots[0].join("a"),
                "root" => roots[1].clone(),
                _ => roots[1].join("a"),
            };
            let moved = replaced.with_extension("moved");
            std::fs::rename(&replaced, &moved).unwrap();
            std::os::unix::fs::symlink(&outside, &replaced).unwrap();
            match staged {
                Some(staged) => assert!(staged.finish(true, &stop).is_err()),
                None => assert!(review.stage(&stop).is_err()),
            }
            assert!(!outside.join("b/file").exists());
            assert!(!outside.join("a/b/file").exists());
            assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"keep");
            if phase == "source-parent" {
                assert_eq!(std::fs::read(moved.join("b/file")).unwrap(), b"reviewed\r\n");
            } else {
                assert!(!moved.join("b/file").exists());
            }
        }
    }

    #[test]
    fn reviewed_copy_reports_nonempty_cleanup_and_denied_creation() {
        use std::os::unix::fs::PermissionsExt;
        let roots = fixture("cleanup");
        let stop = AtomicBool::new(false);
        let review = reviewed(&roots, &stop);
        let staged = review.stage(&stop).unwrap();
        std::fs::write(roots[1].join("a/keep"), "preexisting data").unwrap();
        let error = staged.finish(false, &stop).unwrap_err();
        assert!(error.contains("Directory left at"), "{error}");
        assert_eq!(std::fs::read(roots[1].join("a/keep")).unwrap(), b"preexisting data");
        assert!(!roots[1].join("a/b").exists());
        let roots = fixture("denied");
        let review = reviewed(&roots, &stop);
        std::fs::set_permissions(&roots[1], std::fs::Permissions::from_mode(0o555)).unwrap();
        let denied = review.stage(&stop).is_err();
        std::fs::set_permissions(&roots[1], std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(denied, "native denied mkdir under read-only parent");
        assert!(!roots[1].join("a").exists());
    }

    #[test]
    fn reviewed_copy_unicode_sources_keep_exact_bytes_with_and_without_bom() {
        use crate::encoding::{encode, Encoding};
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            for bom in [false, true] {
                let roots = fixture("unicode");
                let bytes = encode("猫🦀\r\n", encoding, bom).unwrap();
                std::fs::write(roots[0].join("a/b/file"), &bytes).unwrap();
                let stop = AtomicBool::new(false);
                let review = Arc::new(
                    ReviewedCopy::capture_with_hint(
                        roots.clone(),
                        PathBuf::from("a/b/file"),
                        0,
                        (!bom).then_some(encoding),
                        &stop,
                    )
                    .unwrap(),
                );
                assert_eq!(review.snapshot.text.rope.to_string(), "猫🦀\n");
                assert_eq!(review.snapshot.text.format.encoding, encoding);
                review.stage(&stop).unwrap().finish(true, &stop).unwrap();
                assert_eq!(std::fs::read(&review.destination).unwrap(), bytes);
                std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
            }
        }
    }

    #[test]
    fn reviewed_copy_requires_clean_supported_source_and_actual_absence() {
        for kind in [
            "binary",
            "lossy",
            "utf16-odd",
            "utf16-surrogate",
            "symlink",
            "fifo",
            "existing",
            "parent-file",
            "parent-link",
            "traversal",
        ] {
            let roots = fixture(kind);
            let source = roots[0].join("a/b/file");
            match kind {
                "binary" => std::fs::write(&source, b"a\0b").unwrap(),
                "lossy" => std::fs::write(&source, [0xff]).unwrap(),
                "utf16-odd" => std::fs::write(&source, [0xff, 0xfe, 0]).unwrap(),
                "utf16-surrogate" => std::fs::write(&source, [0xfe, 0xff, 0xd8, 0]).unwrap(),
                "symlink" => {
                    std::fs::remove_file(&source).unwrap();
                    std::os::unix::fs::symlink("other", &source).unwrap();
                }
                "fifo" => {
                    std::fs::remove_file(&source).unwrap();
                    let p = super::super::c_path(&source).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
                }
                "existing" => {
                    std::fs::create_dir_all(roots[1].join("a/b")).unwrap();
                    std::fs::write(roots[1].join("a/b/file"), "keep").unwrap();
                }
                "parent-file" => std::fs::write(roots[1].join("a"), "keep").unwrap(),
                "parent-link" => std::os::unix::fs::symlink(roots[0].join("a"), roots[1].join("a")).unwrap(),
                _ => (),
            }
            let relative =
                if kind == "traversal" { PathBuf::from("../left/a/b/file") } else { PathBuf::from("a/b/file") };
            assert!(ReviewedCopy::capture(roots, relative, 0, &AtomicBool::new(false)).is_err(), "{kind}");
        }
    }

    #[test]
    fn reviewed_copy_exclusive_publication_wins_last_moment_file_and_link_races() {
        for race in ["file", "symlink"] {
            let roots = fixture(&format!("exclusive-{race}"));
            let stop = AtomicBool::new(false);
            let review = reviewed(&roots, &stop);
            let staged = review.stage(&stop).unwrap();
            BEFORE_PUBLISH.set(Some(if race == "file" {
                |path| {
                    std::fs::write(path, "keep raced file").unwrap();
                }
            } else {
                |path| {
                    std::os::unix::fs::symlink("keep-link", path).unwrap();
                }
            }));
            let error = staged.finish(true, &stop).unwrap_err();
            assert!(error.contains("exists"), "exclusive rename rejects raced destination: {error}");
            if race == "file" {
                assert_eq!(std::fs::read(&review.destination).unwrap(), b"keep raced file");
            } else {
                assert_eq!(std::fs::read_link(&review.destination).unwrap(), Path::new("keep-link"));
            }
        }
    }

    #[test]
    fn reviewed_copy_mid_stage_mutation_and_created_parent_swap_are_safe() {
        let roots = fixture("mid-stage-source");
        let stop = AtomicBool::new(false);
        let review = reviewed(&roots, &stop);
        AFTER_WRITE.set(Some(|source| {
            std::fs::write(source, "mutated while staging").unwrap();
            Ok(())
        }));
        assert!(review.stage(&stop).is_err());
        assert!(!roots[1].join("a").exists());
        let roots = fixture("mid-stage-parent");
        let outside = roots[1].parent().unwrap().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let review = reviewed(&roots, &stop);
        AFTER_PARENT.set(Some(|path| {
            std::fs::rename(path, path.with_extension("moved")).unwrap();
            std::os::unix::fs::symlink(path.parent().unwrap().parent().unwrap().join("outside"), path).unwrap();
        }));
        let error = match review.stage(&stop) {
            Ok(_) => panic!("parent replacement must reject"),
            Err(error) => error,
        };
        assert!(error.contains("Directory left at"), "changed ownership is reported: {error}");
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        assert!(std::fs::symlink_metadata(roots[1].join("a")).unwrap().is_symlink());
        assert!(std::fs::read_dir(roots[1].join("a.moved")).unwrap().next().is_none());
    }
    #[test]
    fn reviewed_copy_write_failure_cleans_only_owned_data_and_preserves_mode() {
        let roots = fixture("write-failure");
        std::fs::write(roots[1].join("keep"), "keep preexisting").unwrap();
        let stop = AtomicBool::new(false);
        let source = roots[0].join("a/b/file");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o640)).unwrap();
        let review = reviewed(&roots, &stop);
        AFTER_WRITE.set(Some(|_| Err(io::Error::other("injected copy write failure"))));
        let error = match review.stage(&stop) {
            Err(error) => error,
            Ok(_) => panic!("injected failure must reject"),
        };
        assert!(error.contains("injected copy write failure"), "{error}");
        assert!(!roots[1].join("a").exists());
        assert_eq!(std::fs::read(roots[1].join("keep")).unwrap(), b"keep preexisting");
        review.stage(&stop).unwrap().finish(true, &stop).unwrap();
        assert_eq!(std::fs::metadata(&review.destination).unwrap().mode() & 0o777, 0o640);
        assert_eq!(std::fs::read(&source).unwrap(), b"reviewed\r\n");
    }
    #[test]
    fn reviewed_copy_reports_successful_publication_with_cleanup_leftovers() {
        let roots = fixture("published-cleanup");
        let stop = AtomicBool::new(false);
        let review = reviewed(&roots, &stop);
        let staged = review.stage(&stop).unwrap();
        let private = staged.parents.path().join(&staged.stage.as_ref().unwrap().0);
        std::fs::write(private.join("keep"), "external data").unwrap();
        let error = staged.finish(true, &stop).unwrap_err();
        assert!(error.starts_with("Copied file, but cleanup failed:"), "{error}");
        assert!(error.contains(&private.display().to_string()), "{error}");
        assert_eq!(std::fs::read(&review.destination).unwrap(), b"reviewed\r\n");
        assert_eq!(std::fs::read(private.join("keep")).unwrap(), b"external data");
    }
    #[test]
    fn reviewed_copy_preserves_native_acl_and_quarantine() {
        use std::process::Command;
        fn external_acl(path: &Path) -> Option<Vec<u8>> {
            unsafe extern "C" {
                fn acl_get_file(path: *const libc::c_char, kind: libc::c_int) -> *mut libc::c_void;
                fn acl_size(acl: *mut libc::c_void) -> libc::ssize_t;
                fn acl_copy_ext(
                    buffer: *mut libc::c_void,
                    acl: *mut libc::c_void,
                    size: libc::ssize_t,
                ) -> libc::ssize_t;
                fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
            }
            assert!(path.is_file());
            let path = name(path.as_os_str()).unwrap();
            // SAFETY: live NUL-terminated path; ACL_TYPE_EXTENDED is 0x100 in sys/acl.h.
            let acl = unsafe { acl_get_file(path.as_ptr(), 0x100) };
            if acl.is_null() {
                assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ENOENT));
                return None;
            }
            // SAFETY: the native ACL is live; its external representation is written into its sized buffer.
            let bytes = unsafe {
                let size = acl_size(acl);
                assert!(size >= 0);
                let mut bytes = vec![0u8; size as usize];
                let written = acl_copy_ext(bytes.as_mut_ptr().cast(), acl, size);
                assert_eq!(acl_free(acl), 0);
                assert!(written >= 0);
                bytes.truncate(written as usize);
                bytes
            };
            Some(bytes)
        }
        fn run(program: &str, args: &[&str], path: &Path) {
            assert!(Command::new(program).args(args).arg(path).status().unwrap().success());
        }
        let roots = fixture("security-metadata");
        let source = roots[0].join("a/b/file");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        run("/bin/chmod", &["+a", "user:nobody deny read"], &source);
        let source_acl = external_acl(&source).expect("restrictive source ACL");
        run(
            "/usr/bin/xattr",
            &["-w", "com.apple.quarantine", "0083;6ad72e9c;Orlo;00000000-0000-0000-0000-000000000000"],
            &source,
        );
        let stop = AtomicBool::new(false);
        let review = reviewed(&roots, &stop);
        review.stage(&stop).unwrap().finish(true, &stop).unwrap();
        assert_eq!(external_acl(&review.destination), Some(source_acl.clone()));
        let quarantine = Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&review.destination)
            .output()
            .unwrap();
        assert!(quarantine.status.success(), "quarantine error: {}", String::from_utf8_lossy(&quarantine.stderr));
        assert!(!String::from_utf8(quarantine.stdout).unwrap().trim().is_empty());
        assert_eq!(std::fs::read(&review.destination).unwrap(), b"reviewed\r\n");
        assert_eq!(std::fs::metadata(&review.destination).unwrap().mode() & 0o777, 0o755);
        run("/bin/chmod", &["-N"], &review.destination);
        assert_eq!(external_acl(&review.destination), None);
        assert_ne!(external_acl(&review.destination), Some(source_acl.clone()));
        run("/bin/chmod", &["+a", "user:nobody deny write"], &review.destination);
        assert_ne!(external_acl(&review.destination).expect("changed restrictive ACL"), source_acl);
    }
}
