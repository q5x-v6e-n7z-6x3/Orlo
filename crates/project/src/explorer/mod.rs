//! Filesystem inventories for the project explorer.

use crate::watch;
use orlo_config::settings::ExplorerSort;
use orlo_utils::text::natural_cmp;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

const EXCLUDED: [&str; 4] = [".git", ".svn", ".hg", ".DS_Store"];

/// Sublime's default exclusions, and a save's temp file a crash left (`name.orlo-PID-N.tmp`).
fn listed(name: &OsStr) -> bool {
    let n = name.to_string_lossy();
    !EXCLUDED.contains(&n.as_ref()) && crate::io::temp_pid(&n).is_none()
}

/// Off the UI thread: `dir` listed again when its names differ from `shown`.
/// Names only, so a file replaced by a folder of that name waits for the next expand
pub fn reread(dir: &Path, shown: Option<&[OsString]>) -> Option<Vec<(PathBuf, bool)>> {
    // a folder that is gone reads as empty, so it is not listed on every poll
    let names: Vec<OsString> = watch::names(dir).unwrap_or_default().into_iter().filter(|n| listed(n)).collect();
    (shown != Some(names.as_slice())).then(|| entries(dir))
}

/// Directories first, then files, case-insensitive; Sublime's default exclusions skipped.
pub fn entries(dir: &Path) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| listed(&e.file_name()))
                .map(|e| {
                    let p = e.path();
                    // only a link needs a stat: is it to a folder
                    let is_dir = match e.file_type() {
                        Ok(t) if !t.is_symlink() => t.is_dir(),
                        _ => p.is_dir(),
                    };
                    (p, is_dir)
                })
                .collect()
        })
        .unwrap_or_default();
    sort_entries(&mut v, ExplorerSort::Lexical);
    v
}

/// Order a cached listing without reading the filesystem. Fold each name once per update.
pub fn sort_entries(entries: &mut Vec<(PathBuf, bool)>, order: ExplorerSort) {
    let mut keyed: Vec<_> = std::mem::take(entries)
        .into_iter()
        .map(|entry| {
            let key = entry.0.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            (entry, key)
        })
        .collect();
    keyed.sort_by(|((a, ad), ak), ((b, bd), bk)| {
        bd.cmp(ad)
            .then_with(|| match order {
                ExplorerSort::Lexical => ak.cmp(bk),
                ExplorerSort::Natural => natural_cmp(ak.as_bytes(), bk.as_bytes()),
            })
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });
    entries.extend(keyed.into_iter().map(|(entry, _)| entry));
}

/// Sublime dims git-ignored entries.
/// Only plain names from the root .gitignore, no globs or nested files.
pub fn ignored_names(root: &Path) -> HashSet<String> {
    let path = root.join(".gitignore");
    (orlo_utils::paths::readable_ignore_file(&path).then(|| std::fs::read_to_string(&path).ok()))
        .flatten()
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// A path's filesystem identity, captured and checked only on a worker. Directory modification
/// times change as children change; device/inode identity survives those ordinary changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub is_dir: bool,
    identity: (u64, u64),
}

impl Target {
    pub fn read(path: PathBuf) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&path)?;
        Ok(Self { path, is_dir: metadata.is_dir(), identity: (metadata.dev(), metadata.ino()) })
    }

    pub fn directory(path: PathBuf) -> std::io::Result<Self> {
        let target = Self::read(path)?;
        if !target.is_dir {
            return Err(std::io::Error::other("location is no longer a folder"));
        }
        Ok(target)
    }

    pub fn validate(&self) -> std::io::Result<()> {
        if Self::read(self.path.clone())? != *self {
            return Err(std::io::Error::other("location was moved or replaced; browse it again"));
        }
        Ok(())
    }

    /// A guarded ordinary load: validate the actual opened handle, not only a path stat.
    pub fn read_file(&self, encoding: Option<crate::encoding::Encoding>) -> std::io::Result<crate::io::Loaded> {
        self.validate()?;
        let loaded = crate::io::Loaded::read_with_identity(&self.path, encoding, self.identity)?;
        self.validate()?;
        Ok(loaded)
    }

    pub fn children(&self) -> std::io::Result<Vec<Self>> {
        self.validate()?;
        // Preserve the explorer's existing exclusion and ordering policy, but report an
        // unreadable directory rather than presenting a misleading empty menu.
        std::fs::read_dir(&self.path)?;
        let children = entries(&self.path).into_iter().filter_map(|(p, _)| Self::read(p).ok()).collect();
        self.validate()?;
        Ok(children)
    }
}

/// A window's folder locations, independent of editor jumps and project roots.
#[derive(Clone, Default)]
pub struct LocationHistory {
    locations: Vec<Target>,
    at: usize,
}

impl LocationHistory {
    pub fn current(&self) -> Option<&Target> {
        self.locations.get(self.at)
    }

    pub fn visit(&mut self, target: Target) {
        debug_assert!(target.is_dir);
        if self.current().is_some_and(|current| current.path == target.path) {
            self.locations[self.at] = target;
            return;
        }
        self.locations.truncate(self.at + usize::from(!self.locations.is_empty()));
        self.locations.push(target);
        if self.locations.len() > 32 {
            self.locations.remove(0);
        }
        self.at = self.locations.len() - 1;
    }

    /// On a worker: skip missing/replaced entries, retaining the current location on failure.
    pub fn step(&mut self, forward: bool) -> Option<Target> {
        let current = self.current()?.path.clone();
        loop {
            let next = if forward {
                (self.at + 1 < self.locations.len()).then_some(self.at + 1)?
            } else {
                self.at.checked_sub(1)?
            };
            let target = &self.locations[next];
            if target.path != current && target.validate().is_ok() {
                self.at = next;
                return Some(target.clone());
            }
            self.locations.remove(next);
            if next < self.at {
                self.at -= 1;
            }
        }
    }

    /// Drop locations outside the current project without changing surviving entries.
    /// If the current root was removed, prefer the nearest previous surviving location.
    pub fn retain(&mut self, mut keep: impl FnMut(&Target) -> bool) {
        let mut index = 0;
        let mut through_current: usize = 0;
        self.locations.retain(|target| {
            let retained = keep(target);
            if index <= self.at && retained {
                through_current += 1;
            }
            index += 1;
            retained
        });
        self.at = through_current.saturating_sub(1);
    }

    pub fn moved(&mut self, from: &Path, to: &Path) {
        for target in &mut self.locations {
            if let Ok(rest) = target.path.strip_prefix(from) {
                target.path = to.join(rest);
            }
        }
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn n28_history_bounds_branches_skips_replacements_and_follows_known_moves() {
        let d = temp("orlo-n28-history");
        let mut history = LocationHistory::default();
        for n in 0..35 {
            let p = d.join(n.to_string());
            std::fs::create_dir_all(&p).unwrap();
            let target = Target::directory(p).unwrap();
            history.visit(target.clone());
            history.visit(target);
        }
        assert_eq!(history.locations.len(), 32);
        assert_eq!(history.locations[0].path, d.join("3"));
        assert_eq!(history.step(false).unwrap().path, d.join("33"));
        std::fs::remove_dir(d.join("32")).unwrap();
        std::fs::rename(d.join("31"), d.join("old31")).unwrap();
        std::fs::create_dir(d.join("31")).unwrap();
        assert_eq!(history.step(false).unwrap().path, d.join("30"));
        assert_eq!(history.step(true).unwrap().path, d.join("33"));
        history.visit(Target::directory(d.join("7")).unwrap());
        assert!(history.step(true).is_none(), "a visit truncates the forward branch");
        std::fs::rename(d.join("7"), d.join("seven")).unwrap();
        history.moved(&d.join("7"), &d.join("seven"));
        history.current().unwrap().validate().unwrap();
        assert_eq!(history.current().unwrap().path, d.join("seven"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn n28_revisiting_a_replaced_current_path_refreshes_identity_without_an_adjacent_duplicate() {
        let d = temp("orlo-n28-current-replacement");
        let path = d.join("folder");
        std::fs::create_dir(&path).unwrap();
        let mut history = LocationHistory::default();
        history.visit(Target::directory(path.clone()).unwrap());
        std::fs::rename(&path, d.join("old-folder")).unwrap();
        std::fs::create_dir(&path).unwrap();
        history.visit(Target::directory(path).unwrap());
        history.current().unwrap().validate().unwrap();
        assert_eq!(history.locations.len(), 1, "the same path must not consume another location");
        assert!(history.step(false).is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn n28_skipping_a_deleted_intermediate_folder_does_not_stop_at_the_same_location() {
        let d = temp("orlo-n28-history-prune");
        let mut history = LocationHistory::default();
        for name in ["root", "a", "b", "a"] {
            let path = d.join(name);
            std::fs::create_dir_all(&path).unwrap();
            history.visit(Target::directory(path).unwrap());
        }
        std::fs::remove_dir(d.join("b")).unwrap();
        assert_eq!(history.step(false).unwrap().path, d.join("root"));
        assert_eq!(history.step(true).unwrap().path, d.join("a"));
        assert!(history.step(true).is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn n28_guarded_read_checks_the_opened_handle_after_a_path_stat() {
        fn replace_file(path: &Path) {
            std::fs::rename(path, path.with_extension("old")).unwrap();
            std::fs::write(path, "replacement").unwrap();
        }
        fn replace_directory(path: &Path) {
            std::fs::rename(path, path.with_extension("old")).unwrap();
            std::fs::create_dir(path).unwrap();
        }
        fn delete(path: &Path) {
            std::fs::remove_file(path).unwrap();
        }
        let d = temp("orlo-n28-open-handle");
        for (name, change) in
            [("file", replace_file as fn(&Path)), ("directory", replace_directory), ("delete", delete)]
        {
            let path = d.join(name);
            std::fs::write(&path, "original").unwrap();
            let target = Target::read(path).unwrap();
            crate::io::MID_READ.set(Some(change));
            assert!(target.read_file(None).is_err(), "{name}: a pre-open stat must not authorize another file handle");
        }
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn n28_breadcrumb_children_preserve_explorer_order_and_reject_stale_identity() {
        let d = temp("orlo-n28-children");
        std::fs::create_dir_all(d.join("folder")).unwrap();
        std::fs::write(d.join("child.txt"), "child").unwrap();
        std::fs::create_dir(d.join(".git")).unwrap();
        let target = Target::directory(d.clone()).unwrap();
        let children = target.children().unwrap();
        assert_eq!(
            children.iter().map(|t| (t.path.file_name().unwrap().to_str().unwrap(), t.is_dir)).collect::<Vec<_>>(),
            vec![("folder", true), ("child.txt", false)]
        );
        std::fs::rename(d.join("child.txt"), d.join("old.txt")).unwrap();
        std::fs::write(d.join("child.txt"), "replacement").unwrap();
        assert!(children[1].validate().is_err());
        std::fs::remove_dir_all(d).unwrap();
        assert!(target.children().is_err());
    }
}

#[cfg(test)]
mod sort_tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn n29_project_preferences_cannot_override_explorer_presentation() {
        let (project, warnings) = crate::project::Project::parse(
            Path::new("/tmp/n29.orlo-project"),
            r#"{"settings":{"explorer_sort":"natural","explorer_sticky_ancestors":true,"tab_size":2}}"#,
        )
        .unwrap();
        assert_eq!(project.settings.tab_size, Some(2));
        assert!(!project.settings.preferences.contains_key("explorer_sort"));
        assert!(!project.settings.preferences.contains_key("explorer_sticky_ancestors"));
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|w| w.contains("application-wide")));
    }

    #[test]
    fn n29_natural_order_has_unbounded_numeric_runs_and_deterministic_ties() {
        let huge = format!("part-{}", "9".repeat(400));
        let bigger = format!("part-1{}", "0".repeat(400));
        let names = ["part-10", "part-02", "part-2", "Part-2", "part-000", "part-0", "part-2z", "part-02a"];
        let mut entries: Vec<_> = names.into_iter().map(|n| (PathBuf::from(n), false)).collect();
        entries.extend([(bigger.clone().into(), false), (huge.clone().into(), false)]);
        entries.extend([("chapter-10".into(), true), ("chapter-2".into(), true)]);
        sort_entries(&mut entries, ExplorerSort::Natural);
        let actual: Vec<_> = entries.iter().map(|(p, _)| p.to_str().unwrap()).collect();
        assert_eq!(
            actual,
            [
                "chapter-2",
                "chapter-10",
                "part-0",
                "part-000",
                "Part-2",
                "part-2",
                "part-02",
                "part-02a",
                "part-2z",
                "part-10",
                &huge,
                &bigger
            ]
        );
        let expected = entries.clone();
        entries.reverse();
        sort_entries(&mut entries, ExplorerSort::Natural);
        assert_eq!(entries, expected, "independent of filesystem enumeration order");
        sort_entries(&mut entries, ExplorerSort::Lexical);
        assert_eq!(entries[0].0, Path::new("chapter-10"));
        assert_eq!(entries[1].0, Path::new("chapter-2"));
        for a in ["a0", "a00", "a0b", "a00a", "a2", "a10", "a２", "á2"] {
            for b in ["a0", "a00", "a0b", "a00a", "a2", "a10", "a２", "á2"] {
                assert_eq!(natural_cmp(a.as_bytes(), b.as_bytes()), natural_cmp(b.as_bytes(), a.as_bytes()).reverse());
            }
        }
        assert_eq!(natural_cmp(b"a2", "a２".as_bytes()), Ordering::Less, "only ASCII digits are numeric");
    }

    #[test]
    fn folders_and_links_to_folders_come_first_then_names_in_any_case() {
        let dir = std::env::temp_dir().join(format!("orlo-explorer-sort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("b")).unwrap();
        for f in ["C.txt", "a.txt", ".DS_Store"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        std::os::unix::fs::symlink(dir.join("b"), dir.join("A-link")).unwrap();
        std::os::unix::fs::symlink(dir.join("a.txt"), dir.join("z-link")).unwrap();
        let got: Vec<_> = entries(&dir)
            .into_iter()
            .map(|(p, d)| (p.file_name().unwrap().to_string_lossy().into_owned(), d))
            .collect();
        let want = [("A-link", true), ("b", true), ("a.txt", false), ("C.txt", false), ("z-link", false)];
        assert_eq!(got, want.map(|(n, d)| (n.to_string(), d)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
