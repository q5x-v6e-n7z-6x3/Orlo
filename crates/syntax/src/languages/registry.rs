//! Languages defined as data: one folder per language, read from Orlo's bundled assets and the
//! user's `Languages/` folder. A user folder replaces the bundled one with the same id whole.
//! A folder that fails to load is reported and left out; the others load as if it were not
//! there.

use crate::manifest::{self, Manifest};
use orlo_config::settings::editorconfig::Glob;
use regex_automata::meta::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(any(test, feature = "test-support")))]
use std::sync::OnceLock;
use std::sync::{Arc, RwLock};

/// How much of a file's start `files.content` patterns see.
pub const CONTENT_BYTES: usize = 4096;
/// A folder holding more than this is more likely a mistake (a checkout's root) than a language.
const FOLDER_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// Shipped with Orlo.
    Bundled,
    /// From the user's `Languages/` folder.
    User,
}

/// The files embedded for the bundled languages: `(id, [(relative path, bytes)])`.
type Embedded = &'static [(&'static str, &'static [u8])];

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/languages.rs"));
}

/// Where a language's files are read from.
#[derive(Debug)]
enum Files {
    /// The folder's canonical path; each read checks again that a file stays inside it.
    Disk(PathBuf),
    Embedded(Embedded),
}

#[derive(Debug)]
pub struct LanguageDef {
    pub manifest: Manifest,
    pub origin: Origin,
    /// The folder, for messages; a bundled language's is `(bundled)/<id>`.
    pub folder: PathBuf,
    files: Files,
    first_line: Option<Regex>,
    content: Option<Regex>,
    globs: Vec<Glob>,
}

impl LanguageDef {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// Read and check the language in `folder`: its manifest, and every file the manifest
    /// names (inside the folder, a regular file, within its size limit).
    pub fn load(folder: &Path, origin: Origin) -> Result<Self, String> {
        let root = std::fs::canonicalize(folder).map_err(|e| error(folder, &e))?;
        count_entries(&root).map_err(|e| error(folder, &e))?;
        Self::checked(folder, origin, Files::Disk(root))
    }

    /// A language compiled into Orlo, from its embedded files.
    fn embedded(id: &str, files: Embedded) -> Result<Self, String> {
        let folder = Path::new("(bundled)").join(id);
        if files.len() > FOLDER_ENTRIES {
            return Err(error(&folder, &format!("has more than {FOLDER_ENTRIES} files and folders")));
        }
        Self::checked(&folder, Origin::Bundled, Files::Embedded(files))
    }

    /// Read the manifest, check every file it names (inside the folder, a regular file, within
    /// its size limit) and compile its detection patterns.
    fn checked(folder: &Path, origin: Origin, files: Files) -> Result<Self, String> {
        let path = folder.join(manifest::FILE);
        let text = read_from(&files, manifest::FILE, manifest::MANIFEST_BYTES).map_err(|e| error(&path, &e))?;
        let text = String::from_utf8(text).map_err(|e| error(&path, &e))?;
        let manifest = Manifest::parse(&text).map_err(|e| error(&path, &e))?;
        if folder.file_name().and_then(|n| n.to_str()) != Some(manifest.id.as_str()) {
            return Err(error(&path, &format!("id {:?} must match the folder's name", manifest.id)));
        }
        for (file, limit) in manifest.files() {
            match &files {
                Files::Disk(root) => check_inside(root, Path::new(file), limit).map(drop),
                Files::Embedded(_) => read_from(&files, file, limit).map(drop),
            }
            .map_err(|e| error(&folder.join(file), &e))?;
        }
        let compile = |p: &Option<String>| p.as_deref().map(manifest::pattern_regex).transpose();
        let first_line = compile(&manifest.files.first_line).map_err(|e| error(&path, &e))?;
        let content = compile(&manifest.files.content).map_err(|e| error(&path, &e))?;
        let globs = manifest
            .files
            .globs
            .iter()
            .map(|p| Glob::new(p))
            .collect::<Result<_, _>>()
            .map_err(|e: String| error(&path, &e))?;
        Ok(Self { manifest, origin, folder: folder.to_path_buf(), files, first_line, content, globs })
    }

    /// A file the manifest names, read now: on disk it must still lead nowhere outside the
    /// folder and still be within `limit` (a link may have changed since the folder loaded).
    /// Errors name the file.
    pub fn read(&self, relative: &str, limit: u64) -> Result<Vec<u8>, String> {
        manifest::relative(relative)
            .and_then(|()| read_from(&self.files, relative, limit))
            .map_err(|e| error(&self.folder.join(relative), &e))
    }
}

fn error(path: &Path, e: &dyn std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}

fn read_from(files: &Files, relative: &str, limit: u64) -> Result<Vec<u8>, String> {
    match files {
        Files::Disk(root) => std::fs::read(check_inside(root, Path::new(relative), limit)?).map_err(|e| e.to_string()),
        Files::Embedded(files) => {
            let (_, bytes) = files.iter().find(|(path, _)| *path == relative).ok_or("No such file")?;
            if bytes.len() as u64 > limit {
                return Err(format!("is larger than {} KiB", limit >> 10));
            }
            Ok(bytes.to_vec())
        }
    }
}

/// The languages compiled into Orlo; CI keeps them loading without errors.
fn bundled_languages() -> impl Iterator<Item = Result<LanguageDef, String>> {
    bundled::BUNDLED.iter().map(|(id, files)| LanguageDef::embedded(id, files))
}

/// The file at `relative` under the canonical `root`, if it resolves inside it (a link may not
/// lead out) to a regular file of at most `limit` bytes.
fn check_inside(root: &Path, relative: &Path, limit: u64) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err("leads outside the language's folder".into());
    }
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("is not a file".into());
    }
    if metadata.len() > limit {
        return Err(format!("is larger than {} KiB", limit >> 10));
    }
    Ok(path)
}

/// Refuse a folder with more than `FOLDER_ENTRIES` entries, counting subfolders' too; links are
/// counted, not followed.
fn count_entries(root: &Path) -> Result<(), String> {
    let (mut pending, mut seen) = (vec![root.to_path_buf()], 0);
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            seen += 1;
            if seen > FOLDER_ENTRIES {
                return Err(format!("has more than {FOLDER_ENTRIES} files and folders"));
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

/// The loaded languages and what detection looks them up by.
#[derive(Debug, Default)]
pub struct Registry {
    /// Sorted by id.
    languages: Vec<Arc<LanguageDef>>,
    extensions: Claims,
    names: Claims,
    /// Languages with a `files.first_line` pattern, the strongest first.
    first_lines: Vec<usize>,
    globs: Vec<usize>,
}

/// Extension or file name claims, each to its claimants, the strongest first.
#[derive(Debug, Default)]
struct Claims {
    /// As the manifest writes it.
    exact: HashMap<String, Vec<usize>>,
    /// Lowercase.
    folded: HashMap<String, Vec<usize>>,
}

impl Claims {
    fn add(&mut self, key: &str, language: usize) {
        for (claims, key) in [(&mut self.exact, key.to_string()), (&mut self.folded, key.to_ascii_lowercase())] {
            let claimants = claims.entry(key).or_default();
            if !claimants.contains(&language) {
                claimants.push(language);
            }
        }
    }

    /// An exact-case claim (`C` for C++) before one that matches only ignoring case (`c` for C).
    fn get(&self, key: &str) -> [Option<&Vec<usize>>; 2] {
        [self.exact.get(key), self.folded.get(&key.to_ascii_lowercase())]
    }
}

impl Registry {
    /// Load every folder; for each id the strongest folder that loads wins (a user folder over
    /// a bundled one). Returns every folder's errors.
    pub fn build(folders: impl IntoIterator<Item = (Origin, PathBuf)>) -> (Self, Vec<String>) {
        Self::from_loaded(folders.into_iter().map(|(origin, folder)| LanguageDef::load(&folder, origin)))
    }

    /// The bundled languages alone: what Orlo knows before any user folder is read.
    pub fn bundled() -> (Self, Vec<String>) {
        Self::from_loaded(bundled_languages())
    }

    fn from_loaded(loaded: impl IntoIterator<Item = Result<LanguageDef, String>>) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let mut by_id: HashMap<String, LanguageDef> = HashMap::new();
        for loaded in loaded {
            match loaded {
                Ok(language) => {
                    let replaced = by_id.get(language.id()).is_some_and(|old| old.origin > language.origin);
                    if !replaced {
                        by_id.insert(language.id().to_string(), language);
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        let mut languages: Vec<_> = by_id.into_values().map(Arc::new).collect();
        languages.sort_by(|a, b| a.id().cmp(b.id()));
        let mut registry = Self { languages, ..Self::default() };
        // strongest first: a user's language before a bundled one, then by id
        let mut order: Vec<usize> = (0..registry.languages.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(registry.languages[i].origin));
        for i in order {
            let files = &registry.languages[i].manifest.files;
            for (claims, keys) in [(&mut registry.extensions, &files.extensions), (&mut registry.names, &files.names)] {
                for key in keys {
                    claims.add(key, i);
                }
            }
            if !registry.languages[i].globs.is_empty() {
                registry.globs.push(i);
            }
            if registry.languages[i].first_line.is_some() {
                registry.first_lines.push(i);
            }
        }
        (registry, errors)
    }

    pub fn languages(&self) -> &[Arc<LanguageDef>] {
        &self.languages
    }

    pub fn get(&self, id: &str) -> Option<&Arc<LanguageDef>> {
        let i = self.languages.binary_search_by(|l| l.id().cmp(id)).ok()?;
        Some(&self.languages[i])
    }

    /// The language the syntax menu and `overrides` call `name`; hidden languages are left out.
    pub fn named(&self, name: &str) -> Option<&Arc<LanguageDef>> {
        self.languages.iter().find(|l| !l.manifest.hidden && l.name() == name)
    }

    /// The language of a file, from its name, then its extension (the longest first, so
    /// `d.ts` before `ts`; an exact-case claim before a folded one), then its globs and first
    /// line. `head` is the start of its text (empty for a new file). Where several languages
    /// claim a name or an extension, one whose `content` pattern matches wins over one with
    /// none, and one whose pattern fails is out.
    pub fn detect(&self, path: Option<&Path>, head: &str) -> Option<&Arc<LanguageDef>> {
        let head = &head[..head.floor_char_boundary(CONTENT_BYTES)];
        let name = path.and_then(Path::file_name).and_then(|n| n.to_str()).unwrap_or_default();
        // as with `Path::extension`, a leading dot (`.bashrc`) does not start an extension
        let extensions = (name.match_indices('.')).filter(|&(at, _)| at > 0).map(|(at, _)| &name[at + 1..]);
        let claims = self.names.get(name).into_iter().chain(extensions.flat_map(|e| self.extensions.get(e)));
        claims
            .flatten()
            .find_map(|claimants| self.settle(claimants.iter().copied(), head))
            .or_else(|| {
                let path = path.filter(|_| !self.globs.is_empty())?.to_string_lossy();
                self.settle(
                    self.globs
                        .iter()
                        .copied()
                        .filter(|&i| self.languages[i].globs.iter().any(|g| g.matches(path.trim_start_matches('/')))),
                    head,
                )
            })
            .or_else(|| {
                let line = head.split('\n').next().unwrap_or("");
                let line = line.strip_suffix('\r').unwrap_or(line);
                (self.first_lines.iter())
                    .map(|&i| &self.languages[i])
                    .find(|l| l.first_line.as_ref().is_some_and(|r| r.is_match(line)))
            })
    }

    fn settle(&self, claimants: impl IntoIterator<Item = usize>, head: &str) -> Option<&Arc<LanguageDef>> {
        let mut unpatterned = None;
        for i in claimants {
            let language = &self.languages[i];
            match &language.content {
                Some(pattern) if pattern.is_match(head) => return Some(language),
                Some(_) => {}
                None => unpatterned = unpatterned.or(Some(language)),
            }
        }
        unpatterned
    }
}

fn with_slot<R>(read: impl FnOnce(&RwLock<Arc<Registry>>) -> R) -> R {
    // GPUI's deterministic executor runs each app on its test thread. Isolate reloads
    // between concurrent tests, just as the test support directory is isolated.
    #[cfg(any(test, feature = "test-support"))]
    {
        thread_local! { static REGISTRY: RwLock<Arc<Registry>> = RwLock::new(Arc::new(Registry::bundled().0)); }
        REGISTRY.with(read)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        static REGISTRY: OnceLock<RwLock<Arc<Registry>>> = OnceLock::new();
        read(REGISTRY.get_or_init(|| RwLock::new(Arc::new(Registry::bundled().0))))
    }
}

/// The current generation; work that started on an older one keeps it until it finishes.
pub fn registry() -> Arc<Registry> {
    with_slot(|slot| slot.read().unwrap_or_else(|e| e.into_inner()).clone())
}

/// Read the language folders again (at launch and on Reload Customizations), replacing the
/// current generation; returns what failed, one message per folder.
pub fn reload(support: &Path) -> Vec<String> {
    let (folders, mut errors) = orlo_config::themes::language_folders(support);
    let user = folders.into_iter().map(|f| LanguageDef::load(&f, Origin::User));
    let (registry, failed) = Registry::from_loaded(bundled_languages().chain(user));
    errors.extend(failed);
    with_slot(|slot| *slot.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(registry));
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("orlo-registry-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        /// A language folder `<root>/<id>` with `manifest` and empty `files`.
        fn language(&self, root: &str, id: &str, manifest: &str, files: &[&str]) -> PathBuf {
            let folder = self.0.join(root).join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(manifest::FILE), manifest).unwrap();
            for file in files {
                let path = folder.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "").unwrap();
            }
            folder
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(id: &str, name: &str, files: &str) -> String {
        format!(r#"{{"schema":1,"id":"{id}","name":"{name}","scope":"source.{id}","files":{files}}}"#)
    }

    fn detected<'a>(registry: &'a Registry, path: &str, head: &str) -> Option<&'a str> {
        registry.detect(Some(Path::new(path)), head).map(|l| l.id())
    }

    /// A language claiming extension `x`, with `content` as its pattern when given.
    fn x_language(dir: &Dir, root: &str, id: &str, content: Option<&str>) -> PathBuf {
        let content = content.map_or(String::new(), |c| format!(r#","content":"{c}""#));
        let files = format!(r#"{{"extensions":["x"]{content}}}"#);
        dir.language(root, id, &manifest(id, id, &files), &[])
    }

    #[test]
    fn a_user_folder_replaces_the_bundled_one_whole() {
        let dir = Dir::new("override");
        let bundled =
            dir.language("bundled", "pony", &manifest("pony", "Pony", r#"{"extensions":["pony","pny"]}"#), &[]);
        let user = dir.language("user", "pony", &manifest("pony", "Pony (mine)", r#"{"extensions":["pony"]}"#), &[]);
        // the order folders come in does not matter
        for folders in [
            vec![(Origin::Bundled, bundled.clone()), (Origin::User, user.clone())],
            vec![(Origin::User, user.clone()), (Origin::Bundled, bundled.clone())],
        ] {
            let (registry, errors) = Registry::build(folders);
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(registry.languages().len(), 1);
            let pony = registry.get("pony").unwrap();
            assert_eq!((pony.name(), pony.origin), ("Pony (mine)", Origin::User));
            // nothing of the bundled folder is merged in
            assert_eq!(detected(&registry, "a.pny", ""), None);
            assert_eq!(registry.named("Pony (mine)").unwrap().id(), "pony");
            assert!(registry.named("Pony").is_none());
        }
    }

    #[test]
    fn a_broken_user_folder_leaves_the_bundled_language_working() {
        let dir = Dir::new("broken-override");
        let bundled = dir.language("bundled", "pony", &manifest("pony", "Pony", r#"{"extensions":["pony"]}"#), &[]);
        let user = dir.language("user", "pony", "{\"schema\":1,", &[]);
        let (registry, errors) = Registry::build([(Origin::Bundled, bundled), (Origin::User, user.clone())]);
        assert_eq!(registry.get("pony").unwrap().origin, Origin::Bundled);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with(&user.join("language.json").display().to_string()));
    }

    #[test]
    fn one_broken_folder_among_good_ones_is_reported_alone() {
        let dir = Dir::new("isolation");
        let good =
            |id: &str| dir.language("user", id, &manifest(id, id, &format!(r#"{{"extensions":["{id}"]}}"#)), &[]);
        let mut folders = vec![good("aaa"), good("zzz")];
        let wrong_id = dir.language("user", "folder", &manifest("other", "X", "{}"), &[]);
        let no_manifest = dir.0.join("user/empty");
        fs::create_dir_all(&no_manifest).unwrap();
        let missing_file = dir.language(
            "user",
            "nosyntax",
            &manifest("nosyntax", "N", "{}").replace(r#""files":{}"#, r#""files":{},"syntax":"N.sublime-syntax""#),
            &[],
        );
        let unknown_field = dir.language("user", "typo", &manifest("typo", "T", r#"{"extention":["t"]}"#), &[]);
        folders.extend([wrong_id.clone(), no_manifest.clone(), missing_file.clone(), unknown_field.clone()]);
        let (registry, errors) = Registry::build(folders.into_iter().map(|f| (Origin::User, f)));
        let ids: Vec<_> = registry.languages().iter().map(|l| l.id()).collect();
        assert_eq!(ids, ["aaa", "zzz"]);
        assert_eq!(errors.len(), 4, "{errors:?}");
        let error_for = |path: PathBuf| {
            let path = path.display().to_string();
            errors
                .iter()
                .find(|e| e.starts_with(&path))
                .unwrap_or_else(|| panic!("no error for {path}: {errors:?}"))
                .clone()
        };
        assert!(error_for(wrong_id.join("language.json")).contains("must match the folder"));
        assert!(error_for(no_manifest.join("language.json")).contains("No such file"));
        assert!(error_for(missing_file.join("N.sublime-syntax")).contains("No such file"));
        assert!(error_for(unknown_field.join("language.json")).contains("unknown field `extention`"));
    }

    #[test]
    fn named_files_must_stay_inside_the_folder_and_within_limits() {
        let dir = Dir::new("files");
        let with_syntax = |id: &str, syntax: &str| {
            manifest(id, id, "{}").replace(r#""files":{}"#, &format!(r#""files":{{}},"syntax":"{syntax}""#))
        };
        fs::write(dir.0.join("Outside.sublime-syntax"), "").unwrap();
        let escape = dir.language("user", "escape", &with_syntax("escape", "S.sublime-syntax"), &[]);
        std::os::unix::fs::symlink(dir.0.join("Outside.sublime-syntax"), escape.join("S.sublime-syntax")).unwrap();
        let folder = dir.language("user", "folder", &with_syntax("folder", "S.sublime-syntax"), &[]);
        fs::create_dir(folder.join("S.sublime-syntax")).unwrap();
        let big = dir.language("user", "big", &with_syntax("big", "S.sublime-syntax"), &[]);
        fs::write(big.join("S.sublime-syntax"), vec![b' '; manifest::SYNTAX_BYTES as usize + 1]).unwrap();
        let crowded = dir.language("user", "crowded", &manifest("crowded", "C", "{}"), &[]);
        for i in 0..FOLDER_ENTRIES {
            fs::write(crowded.join(format!("{i}.txt")), "").unwrap();
        }
        let huge = dir.language("user", "huge", "", &[]);
        fs::write(huge.join(manifest::FILE), vec![b' '; manifest::MANIFEST_BYTES as usize + 1]).unwrap();
        let linked_inside =
            dir.language("user", "inside", &with_syntax("inside", "S.sublime-syntax"), &["real/S.sublime-syntax"]);
        std::os::unix::fs::symlink(linked_inside.join("real/S.sublime-syntax"), linked_inside.join("S.sublime-syntax"))
            .unwrap();
        let (registry, errors) =
            Registry::build([escape, folder, big, crowded, huge, linked_inside].into_iter().map(|f| (Origin::User, f)));
        assert_eq!(registry.languages().iter().map(|l| l.id()).collect::<Vec<_>>(), ["inside"]);
        for expected in
            ["leads outside", "is not a file", "larger than 4096 KiB", "more than 256 files", "larger than 256 KiB"]
        {
            assert!(errors.iter().any(|e| e.contains(expected)), "{expected}: {errors:?}");
        }
    }

    #[test]
    fn detection_by_name_extension_first_line_and_content() {
        let dir = Dir::new("detect");
        let folders = [
            dir.language(
                "bundled",
                "rux",
                &manifest("rux", "Rux", r#"{"extensions":["rux"],"content":"\\A\\s*(?:[^<\\s]|\\z)"}"#),
                &[],
            ),
            dir.language(
                "user",
                "ruxlang",
                &manifest("ruxlang", "Rux UI", r#"{"extensions":["rux"],"content":"\\A\\s*<"}"#),
                &[],
            ),
            dir.language(
                "bundled",
                "make",
                &manifest("make", "Makefile", r#"{"names":["Makefile","GNUmakefile"],"extensions":["mk"]}"#),
                &[],
            ),
            dir.language(
                "bundled",
                "pony",
                &manifest("pony", "Pony", r#"{"extensions":["Pony"],"first_line":"^#!.*\\bpony\\b"}"#),
                &[],
            ),
        ];
        let origins = [Origin::Bundled, Origin::User, Origin::Bundled, Origin::Bundled];
        let (registry, errors) = Registry::build(origins.into_iter().zip(folders));
        assert!(errors.is_empty(), "{errors:?}");
        let rux = |head: &str| detected(&registry, "/p/main.rux", head);
        assert_eq!(rux("module main\nfunc f() {}\n"), Some("rux"));
        assert_eq!(rux("// a comment\n"), Some("rux"));
        assert_eq!(rux(""), Some("rux"), "a new file is Rux");
        assert_eq!(rux("\n\n<template>\n  <p/>\n"), Some("ruxlang"));
        assert_eq!(rux("<!-- note -->\n"), Some("ruxlang"));
        // only the first 4 KiB are read, cut at a char boundary
        let past_the_cap = " ".repeat(CONTENT_BYTES) + "<template>";
        assert_eq!(rux(&past_the_cap), Some("rux"));
        // a cut through a character steps back to its start rather than panic
        let split = " ".repeat(CONTENT_BYTES - 1) + "é<";
        assert_eq!(rux(&split), Some("rux"));

        assert_eq!(detected(&registry, "/p/Makefile", ""), Some("make"));
        assert_eq!(detected(&registry, "/p/makefile", ""), Some("make"));
        assert_eq!(detected(&registry, "/p/rules.MK", ""), Some("make"));
        assert_eq!(detected(&registry, "/p/a.PONY", ""), Some("pony"));
        assert_eq!(detected(&registry, "/p/run", "#!/usr/bin/env pony\r\nx"), Some("pony"));
        assert_eq!(detected(&registry, "/p/run", "#!/bin/sh\n# pony\n"), None);
        assert_eq!(detected(&registry, "/p/a.txt", "module x"), None);
        assert!(registry.detect(None, "#!/usr/bin/env pony").is_some());
        assert!(registry.detect(None, "").is_none());
    }

    #[test]
    fn a_matching_pattern_beats_no_pattern_and_a_failing_one_loses_to_it() {
        let dir = Dir::new("patterns");
        let lang = |root: &str, id: &str, content| x_language(&dir, root, id, content);
        // the user's language has no pattern; the bundled one's matches, and wins
        let (registry, _) = Registry::build([
            (Origin::User, lang("user", "plain", None)),
            (Origin::Bundled, lang("bundled", "shaped", Some("^shape"))),
        ]);
        assert_eq!(detected(&registry, "a.x", "shape"), Some("shaped"));
        // its pattern fails, so the language without one takes the file
        assert_eq!(detected(&registry, "a.x", "other"), Some("plain"));
    }

    #[test]
    fn the_longest_extension_is_tried_first() {
        let dir = Dir::new("suffixes");
        let lang = |id: &str, extension: &str| {
            dir.language("bundled", id, &manifest(id, id, &format!(r#"{{"extensions":["{extension}"]}}"#)), &[])
        };
        let (registry, errors) = Registry::build([
            (Origin::Bundled, lang("typescript", "ts")),
            (Origin::Bundled, lang("declarations", "d.ts")),
            (Origin::Bundled, lang("shell", "bashrc")),
        ]);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(detected(&registry, "lib.D.TS", ""), Some("declarations"));
        assert_eq!(detected(&registry, "a.b.d.ts", ""), Some("declarations"));
        assert_eq!(detected(&registry, "main.ts", ""), Some("typescript"));
        // a leading dot names a file, not an extension, as with `Path::extension`
        assert_eq!(detected(&registry, ".bashrc", ""), None);
        assert_eq!(detected(&registry, "x.bashrc", ""), Some("shell"));
    }

    #[test]
    fn an_exact_case_claim_beats_one_that_matches_only_ignoring_case() {
        let dir = Dir::new("case");
        let lang = |root: &str, id: &str, files: &str| dir.language(root, id, &manifest(id, id, files), &[]);
        let (registry, errors) = Registry::build([
            (Origin::Bundled, lang("bundled", "c", r#"{"extensions":["c","h"]}"#)),
            (Origin::Bundled, lang("bundled", "cpp", r#"{"extensions":["C","cc"]}"#)),
            (Origin::User, lang("user", "mine", r#"{"extensions":["c"]}"#)),
            (Origin::Bundled, lang("bundled", "make", r#"{"names":["makefile"]}"#)),
            (Origin::Bundled, lang("bundled", "other", r#"{"names":["Makefile"]}"#)),
            (Origin::Bundled, lang("bundled", "shaped", r#"{"extensions":["X"],"content":"^shape"}"#)),
            (Origin::Bundled, lang("bundled", "plain", r#"{"extensions":["x"]}"#)),
        ]);
        assert!(errors.is_empty(), "{errors:?}");
        for (path, head, want) in [
            ("main.c", "", "mine"),
            // the exact claim wins even over a user's folded one
            ("main.C", "", "cpp"),
            ("main.H", "", "c"),
            ("main.CC", "", "cpp"),
            ("Makefile", "", "other"),
            ("makefile", "", "make"),
            ("MAKEFILE", "", "make"),
            ("a.X", "shape", "shaped"),
            // an exact claimant whose pattern fails leaves the file to the folded ones
            ("a.X", "other", "plain"),
            ("a.x", "shape", "plain"),
        ] {
            assert_eq!(detected(&registry, path, head), Some(want), "{path}: {head}");
        }
    }

    #[test]
    fn several_claimants_without_patterns_go_to_the_users_then_by_id() {
        let dir = Dir::new("ties");
        let lang = |root: &str, id: &str, content| x_language(&dir, root, id, content);
        let (registry, _) = Registry::build([
            (Origin::Bundled, lang("bundled", "alpha", None)),
            (Origin::Bundled, lang("bundled", "beta", None)),
        ]);
        assert_eq!(detected(&registry, "a.x", ""), Some("alpha"));
        let (registry, _) = Registry::build([
            (Origin::Bundled, lang("bundled", "alpha", None)),
            (Origin::User, lang("user", "zeta", None)),
        ]);
        assert_eq!(detected(&registry, "a.x", ""), Some("zeta"));
        // a pattern that fails rules its language out, even with no other claimant
        let (registry, _) = Registry::build([(Origin::User, lang("user", "only", Some("^yes")))]);
        assert_eq!(detected(&registry, "a.x", "yes"), Some("only"));
        assert_eq!(detected(&registry, "a.x", "no"), None);
    }

    #[test]
    fn hidden_languages_are_detected_but_not_named() {
        let dir = Dir::new("hidden");
        let jsx = dir.language(
            "bundled",
            "jsx",
            &manifest("jsx", "JavaScript", r#"{"extensions":["jsx"]}"#).replace(r#""name""#, r#""hidden":true,"name""#),
            &[],
        );
        let js = dir.language(
            "bundled",
            "javascript",
            &manifest("javascript", "JavaScript", r#"{"extensions":["js"]}"#),
            &[],
        );
        let (registry, errors) = Registry::build([(Origin::Bundled, jsx), (Origin::Bundled, js)]);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(detected(&registry, "a.jsx", ""), Some("jsx"));
        assert_eq!(registry.named("JavaScript").unwrap().id(), "javascript");
    }

    #[test]
    fn arbitrary_bytes_in_a_manifest_are_an_error_not_a_panic() {
        let dir = Dir::new("noise");
        let folder = dir.language("user", "noise", "", &[]);
        // mostly JSON's own characters, so some inputs get deep into the parser
        const ALPHABET: &[u8] = b"{}[]\",:01 schemaidnetruefalsul\\\n\xff";
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for round in 0..300 {
            let len = (round * 7) % 600;
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    ALPHABET[(seed % ALPHABET.len() as u64) as usize]
                })
                .collect();
            fs::write(folder.join(manifest::FILE), &bytes).unwrap();
            let (registry, errors) = Registry::build([(Origin::User, folder.clone())]);
            assert!(registry.languages().is_empty());
            assert_eq!(errors.len(), 1);
        }
    }

    #[test]
    fn globs_are_validated_and_detect_paths_before_shebang_fallback() {
        let dir = Dir::new("globs");
        let files =
            r#"{"globs":["**/views/*.{tpl,view}","Build[12]","/p/absolute/*.fixture"],"first_line":"^#!.*\\bpony\\b"}"#;
        let pony = dir.language("user", "pony", &manifest("pony", "Pony", files), &[]);
        let exact = dir.language(
            "user",
            "exact",
            &manifest("exact", "Exact", r#"{"names":["Build1"],"extensions":["view"]}"#),
            &[],
        );
        let (registry, errors) = Registry::build([(Origin::User, pony), (Origin::User, exact)]);
        assert!(errors.is_empty(), "{errors:?}");
        for (path, head, want) in [
            ("/p/views/中.tpl", "", Some("pony")),
            ("/p/views/main.view", "", Some("exact")),
            ("/p/views/nested/no.tpl", "", None),
            ("/p/Views/no.tpl", "", None),
            ("/p/absolute/main.fixture", "", Some("pony")),
            ("/other/p/absolute/main.fixture", "", None),
            ("/p/Build2", "", Some("pony")),
            ("/p/Build1", "", Some("exact")),
            ("/p/build2", "", None),
            ("/p/run", "#!/usr/bin/env pony\r\n", Some("pony")),
            ("/p/run", "#!/bin/sh\n# pony\n", None),
        ] {
            assert_eq!(detected(&registry, path, head), want, "{path}: {head}");
        }
        let invalid = dir.language("user", "invalid", &manifest("invalid", "Invalid", r#"{"globs":["{3..1}"]}"#), &[]);
        let (_, errors) = Registry::build([(Origin::User, invalid.clone())]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("files.globs"), "{errors:?}");
        assert!(errors[0].contains(&invalid.display().to_string()), "{errors:?}");
    }

    #[test]
    fn glob_claimants_keep_existing_origin_and_content_precedence() {
        let dir = Dir::new("glob-priority");
        let fallback = dir.language("user", "alpha", &manifest("alpha", "Alpha", r#"{"globs":["*.fixture"]}"#), &[]);
        let shaped = dir.language(
            "bundled",
            "zeta",
            &manifest("zeta", "Zeta", r#"{"globs":["*.fixture"],"content":"^shape"}"#),
            &[],
        );
        let (registry, errors) = Registry::build([(Origin::Bundled, shaped), (Origin::User, fallback)]);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(detected(&registry, "/p/main.fixture", "shape"), Some("zeta"));
        assert_eq!(detected(&registry, "/p/main.fixture", "other"), Some("alpha"));
        assert_eq!(detected(&registry, "/p/main.FIXTURE", "shape"), None);
        assert_eq!(detected(&registry, "/p/main.fixture.extra", "shape"), None);
    }

    #[test]
    fn embedded_languages_load_and_read_like_folders() {
        static GOOD: [(&str, &[u8]); 2] = [
            (
                "language.json",
                br#"{"schema":1,"id":"emb","name":"Emb","scope":"source.emb","syntax":"Emb.sublime-syntax"}"#,
            ),
            ("Emb.sublime-syntax", b"%YAML 1.2"),
        ];
        static MISSING: [(&str, &[u8]); 1] = [(
            "language.json",
            br#"{"schema":1,"id":"gone","name":"Gone","scope":"source.gone","syntax":"Gone.sublime-syntax"}"#,
        )];
        let emb = LanguageDef::embedded("emb", &GOOD).unwrap();
        assert_eq!((emb.origin, emb.folder.as_path()), (Origin::Bundled, Path::new("(bundled)/emb")));
        assert_eq!(emb.read("Emb.sublime-syntax", 1 << 10).unwrap(), b"%YAML 1.2");
        assert!(emb.read("Emb.sublime-syntax", 4).unwrap_err().contains("larger than"));
        assert!(emb.read("../x", 1 << 10).unwrap_err().contains("inside the language's folder"));
        let gone = LanguageDef::embedded("gone", &MISSING).unwrap_err();
        assert!(gone.starts_with("(bundled)/gone/Gone.sublime-syntax"), "{gone}");
        assert!(LanguageDef::embedded("other", &GOOD).unwrap_err().contains("must match the folder"));
        // the bundled table itself loads without errors
        assert!(Registry::bundled().1.is_empty());
    }

    #[test]
    fn a_read_checks_the_file_again_after_the_folder_loaded() {
        let dir = Dir::new("reread");
        let with_syntax =
            manifest("late", "Late", "{}").replace(r#""files":{}"#, r#""files":{},"syntax":"S.sublime-syntax""#);
        let folder = dir.language("user", "late", &with_syntax, &["S.sublime-syntax"]);
        let language = LanguageDef::load(&folder, Origin::User).unwrap();
        assert_eq!(language.read("S.sublime-syntax", 16).unwrap(), b"");
        fs::write(dir.0.join("Outside.sublime-syntax"), "x").unwrap();
        fs::remove_file(folder.join("S.sublime-syntax")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("Outside.sublime-syntax"), folder.join("S.sublime-syntax")).unwrap();
        let error = language.read("S.sublime-syntax", 16).unwrap_err();
        assert!(error.contains("leads outside") && error.contains("S.sublime-syntax"), "{error}");
    }

    #[test]
    fn reload_reads_the_support_folders_languages_and_reports_failures() {
        let dir = Dir::new("reload");
        dir.language("Languages", "pony", &manifest("pony", "Pony", r#"{"extensions":["pony"]}"#), &[]);
        dir.language("Languages", "broken", "not json", &[]);
        dir.language("Languages", "rust", &manifest("rust", "My Rust", r#"{"extensions":["rsx"]}"#), &[]);
        let errors = reload(&dir.0);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("Languages/broken/language.json"));
        let current = registry();
        assert_eq!(detected(&current, "a.pony", ""), Some("pony"));
        assert_eq!(current.get("pony").unwrap().origin, Origin::User);
        // the user's folder replaces the bundled one whole, the other bundled ones stay
        assert_eq!((current.get("rust").unwrap().name(), detected(&current, "a.rs", "")), ("My Rust", None));
        assert_eq!(detected(&current, "a.go", ""), Some("go"));
        fs::remove_dir_all(dir.0.join("Languages")).unwrap();
        assert!(reload(&dir.0).is_empty());
        // only the bundled languages remain
        let bundled = Registry::bundled().0;
        let ids = |r: &Registry| r.languages().iter().map(|l| (l.id().to_string(), l.origin)).collect::<Vec<_>>();
        assert!(!bundled.languages().is_empty());
        assert!(bundled.languages().iter().all(|l| l.origin == Origin::Bundled));
        assert_eq!(ids(&registry()), ids(&bundled));
        assert_eq!(detected(&registry(), "a.rs", ""), Some("rust"));
        assert_eq!(registry().get("pony").map(|l| l.origin), Some(Origin::Bundled));
        // a job holding the older generation keeps it
        assert!(current.get("pony").is_some());
    }
}
