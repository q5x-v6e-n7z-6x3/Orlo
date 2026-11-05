//! Project folders, exclude patterns, and shared Sublime project files.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use orlo_config::settings::{strip_json, WordWrap};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub use crate::paths::{absolute, relative};

/// One folder of a window's side bar, in the order the user put them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Root {
    pub path: PathBuf,
    /// The side bar's label instead of the folder's name (Sublime's `name`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Sublime's patterns: folders and files hidden from the side bar, Goto Anything and the
    /// searches of this root.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folder_exclude_patterns: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub file_exclude_patterns: Vec<String>,
    /// How the project file spelt the path (`.`, `../lib`), written back while it still names
    /// the same folder.
    #[serde(skip)]
    pub spelt: Option<String>,
    /// Not a folder now (deleted, renamed, a drive not mounted): listed dimmed, kept, and back
    /// when it is.
    #[serde(skip)]
    pub missing: bool,
}

impl Root {
    pub fn new(path: PathBuf) -> Self {
        Self { path, ..Default::default() }
    }

    /// The side bar's label: the project's name for it, else the folder's.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            (self.path.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }

    pub fn excludes(&self) -> Excludes {
        Excludes::new(&self.path, &self.folder_exclude_patterns, &self.file_exclude_patterns)
    }
}

/// A root's exclude patterns, read as `.gitignore` lines from the root: `node_modules` hides
/// such a folder at any depth, `build/out` only that one; folder patterns never hide a file.
#[derive(Clone)]
pub struct Excludes {
    folders: Option<Gitignore>,
    files: Option<Gitignore>,
}

impl Excludes {
    pub fn new(root: &Path, folders: &[String], files: &[String]) -> Self {
        let build = |pats: &[String], dirs: bool| {
            if pats.is_empty() {
                return None;
            }
            let mut b = GitignoreBuilder::new(root);
            for p in pats {
                let line = match dirs && !p.ends_with('/') {
                    true => format!("{p}/"),
                    false => p.clone(),
                };
                // a pattern that does not parse hides nothing (validation reports it)
                let _ = b.add_line(None, &line);
            }
            b.build().ok()
        };
        Self { folders: build(folders, true), files: build(files, false) }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn none() -> Self {
        Self { folders: None, files: None }
    }

    /// Whether `path` (under the root) is hidden.
    pub fn hides(&self, path: &Path, is_dir: bool) -> bool {
        let m = match is_dir {
            true => &self.folders,
            false => &self.files,
        };
        m.as_ref().is_some_and(|g| g.matched(path, is_dir).is_ignore())
    }
}

/// Settings a project may set for its window (Sublime's `settings` object). Others are named
/// in a warning and do nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectSettings {
    /// Validated partial preferences, including project-local scoped rules.
    pub preferences: Map<String, Value>,
    pub tab_size: Option<usize>,
    pub translate_tabs_to_spaces: Option<bool>,
    pub word_wrap: Option<WordWrap>,
    pub wrap_width: Option<usize>,
    pub rulers: Option<Vec<usize>>,
}

/// A `.sublime-project` file as read: its folders made absolute, its supported settings, and
/// the whole object so a rewrite keeps keys Orlo does not use (`build_systems`, say).
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub path: PathBuf,
    pub roots: Vec<Root>,
    pub settings: ProjectSettings,
    raw: Map<String, Value>,
    /// The file could not be read when the session named it: it is never written over.
    pub unread: bool,
}

/// The extensions of the two files.
pub const PROJECT_EXT: &str = "sublime-project";
pub const WORKSPACE_EXT: &str = "orlo-workspace";

impl Project {
    /// A new project at `path` holding `roots`.
    pub fn new(path: PathBuf, roots: Vec<Root>) -> Self {
        Self { path, roots, settings: ProjectSettings::default(), raw: Map::new(), unread: false }
    }

    /// The project's name: the file's, without `.sublime-project`.
    pub fn name(&self) -> String {
        name_of(&self.path)
    }

    /// The workspace file a window of this project uses unless one was chosen.
    pub fn default_workspace(&self) -> PathBuf {
        self.path.with_extension(WORKSPACE_EXT)
    }

    /// Read `path`. Errors stop the read (the window keeps what it had); warnings name what is
    /// ignored.
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(path, &text)
    }

    pub fn parse(path: &Path, text: &str) -> Result<(Self, Vec<String>), String> {
        let json = strip_json(text.strip_prefix('\u{feff}').unwrap_or(text));
        let raw = match json.trim() {
            "" => Map::new(),
            j => match serde_json::from_str::<Value>(j) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return Err("A project file must be one JSON object, { … }.".into()),
                Err(e) => return Err(format!("{}: {e}", path.display())),
            },
        };
        let dir = path.parent().unwrap_or(Path::new("/"));
        let mut roots: Vec<Root> = Vec::new();
        match raw.get("folders") {
            None | Some(Value::Null) => {}
            Some(Value::Array(folders)) => {
                for (i, f) in folders.iter().enumerate() {
                    let root = folder(f, dir).map_err(|e| format!("folders[{i}]: {e}"))?;
                    // the same folder twice is one root
                    if !roots.iter().any(|r| same_path(&r.path, &root.path)) {
                        roots.push(root);
                    }
                }
            }
            Some(_) => return Err("\"folders\" must be a list of { \"path\": … }".into()),
        }
        let mut warnings = Vec::new();
        let settings = match raw.get("settings") {
            None | Some(Value::Null) => ProjectSettings::default(),
            Some(Value::Object(s)) => settings(s, &mut warnings),
            Some(_) => return Err("\"settings\" must be an object".into()),
        };
        let p = Self { path: path.to_path_buf(), roots, settings, raw, unread: false };
        Ok((p, warnings))
    }

    /// The file's text: its object as read with `folders` made from `roots` (relative to the
    /// project's folder where they are inside it), tab-indented as Sublime writes it.
    /// Comments in a hand-written file are lost when Orlo rewrites it (adding or
    /// removing a folder); a JSON-preserving editor would keep them.
    pub fn to_text(&self) -> String {
        let dir = self.path.parent().unwrap_or(Path::new("/"));
        let folders: Vec<Value> = (self.roots.iter())
            .map(|r| {
                let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
                if let Value::Object(m) = &mut v {
                    m.insert("path".into(), Value::String(spell(r, dir)));
                }
                v
            })
            .collect();
        let mut raw = self.raw.clone();
        raw.insert("folders".into(), Value::Array(folders));
        let mut out = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
        let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
        let _ = Value::Object(raw).serialize(&mut ser);
        String::from_utf8(out).unwrap_or_default() + "\n"
    }

    /// Written to a temporary file and renamed into place.
    pub fn save(&self) -> std::io::Result<()> {
        crate::io::write_atomic(&self.path, &self.to_text())
    }
}

/// `path`'s name without its extension.
pub fn name_of(path: &Path) -> String {
    (path.file_stem()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The folders of `roots` that are there now.
pub fn present(roots: &[Root]) -> Vec<PathBuf> {
    (roots.iter()).filter(|r| !r.missing).map(|r| r.path.clone()).collect()
}

/// Two paths name one file or folder when they resolve to the same place.
pub fn same_path(a: &Path, b: &Path) -> bool {
    a == b || crate::io::resolve(a) == crate::io::resolve(b)
}

/// How to write `r`'s path: as the file spelt it while that still names it, else relative.
fn spell(r: &Root, dir: &Path) -> String {
    match &r.spelt {
        Some(s) if absolute(s, dir) == r.path => s.clone(),
        _ => relative(&r.path, dir),
    }
}

fn strings(m: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => (a.iter())
            .map(|s| match s {
                Value::String(s) => Ok(s.clone()),
                _ => Err(format!("\"{key}\" must be a list of strings")),
            })
            .collect(),
        Some(_) => Err(format!("\"{key}\" must be a list of strings")),
    }
}

fn folder(f: &Value, dir: &Path) -> Result<Root, String> {
    let Value::Object(m) = f else {
        return Err("a folder must be { \"path\": … }".into());
    };
    let Some(Value::String(spelt)) = m.get("path") else {
        return Err("a folder needs a \"path\" string".into());
    };
    let name = match m.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) => Some(n.clone()),
        Some(_) => return Err("\"name\" must be a string".into()),
    };
    let path = absolute(spelt, dir);
    let root = Root {
        folder_exclude_patterns: strings(m, "folder_exclude_patterns")?,
        file_exclude_patterns: strings(m, "file_exclude_patterns")?,
        name,
        spelt: Some(spelt.clone()),
        missing: !path.is_dir(),
        path,
    };
    for p in (root.folder_exclude_patterns.iter()).chain(&root.file_exclude_patterns) {
        let mut b = GitignoreBuilder::new(&root.path);
        if let Err(e) = b.add_line(None, p) {
            return Err(format!("the pattern \"{p}\" does not parse: {e}"));
        }
    }
    Ok(root)
}

/// What warns of misleading text (Trojan Source): a folder may not turn off the warnings that
/// would catch what it brings.
const PROTECTED: [&str; 3] = ["unicode_warnings", "unicode_warning_characters", "unicode_warning_ranges"];

/// The supported keys of a project's `settings`; the others, and values of the wrong kind,
/// are named in `warnings`.
fn settings(s: &Map<String, Value>, warnings: &mut Vec<String>) -> ProjectSettings {
    let mut out = ProjectSettings::default();
    let defaults = orlo_config::settings::Settings::default();
    for (key, value) in s {
        let mut value = value.clone();
        if PROTECTED.contains(&key.as_str()) {
            warnings.push(format!("settings: {key:?} guards against misleading text; set it in global preferences"));
            continue;
        }
        // nor inside a per-file override
        if let Some(overrides) = (key == "overrides").then(|| value.as_array_mut()).flatten() {
            for o in overrides.iter_mut().filter_map(|o| o.get_mut("settings")?.as_object_mut()) {
                for k in PROTECTED.iter().filter(|k| o.remove(**k).is_some()) {
                    warnings.push(format!(
                        "settings: overrides: {k:?} guards against misleading text; set it in global preferences"
                    ));
                }
            }
        }
        let value = &value;
        if orlo_config::settings::APPLICATION_SETTINGS.contains(&key.as_str()) {
            warnings.push(format!("settings: {key:?} is application-wide; set it in global preferences"));
            continue;
        }
        if key != "overrides" && !orlo_config::settings::FILE_SETTINGS.contains(&key.as_str()) {
            warnings.push(format!("settings: Orlo ignores unsupported preference {key:?}"));
            continue;
        }
        let layer = Map::from_iter([(key.clone(), value.clone())]);
        match defaults.overlay(&layer) {
            Ok(parsed) => {
                match key.as_str() {
                    "tab_size" => out.tab_size = Some(parsed.tab_size),
                    "translate_tabs_to_spaces" => out.translate_tabs_to_spaces = Some(parsed.translate_tabs_to_spaces),
                    "word_wrap" => out.word_wrap = Some(parsed.word_wrap),
                    "wrap_width" => out.wrap_width = Some(parsed.wrap_width),
                    "rulers" => out.rulers = Some(parsed.rulers),
                    _ => {}
                }
                out.preferences.insert(key.clone(), value.clone());
            }
            Err(error) => warnings.push(format!("settings: {key:?} has a value Orlo cannot use: {error}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_cannot_turn_off_misleading_text_warnings() {
        let dir = temp("orlo-project-protected");
        let p = dir.join("p.sublime-project");
        let text = r#"{"settings":{"unicode_warnings":false,"tab_size":2,
            "overrides":[{"path":"*.rs","settings":{"unicode_warning_characters":{},"tab_size":3}}]}}"#;
        let (project, warnings) = Project::parse(&p, text).unwrap();
        let prefs = &project.settings.preferences;
        assert!(!prefs.contains_key("unicode_warnings") && prefs.contains_key("tab_size"));
        let overrides = serde_json::to_string(&prefs["overrides"]).unwrap();
        assert!(!overrides.contains("unicode_warning_characters") && overrides.contains("tab_size"), "{overrides}");
        assert_eq!(warnings.iter().filter(|w| w.contains("misleading text")).count(), 2, "{warnings:?}");
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_project_reads_relative_folders_names_patterns_and_settings() {
        let dir = temp("orlo-project-read");
        std::fs::create_dir_all(dir.join("app/src")).unwrap();
        let p = dir.join("app/My App.sublime-project");
        let text = r#"{
            // Sublime's dialect: comments and trailing commas
            "folders": [
                { "path": ".", "name": "App", "folder_exclude_patterns": ["target"], },
                { "path": "../lib" },
                { "path": "src" },
                { "path": "./" },
            ],
            "settings": { "tab_size": 2, "word_wrap": true, "wrap_width": 80, "font_size": 20, "color_scheme": "x" },
            "build_systems": [{ "name": "make" }],
        }"#;
        let (proj, warnings) = Project::parse(&p, text).unwrap();
        let paths: Vec<_> = proj.roots.iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, [dir.join("app"), dir.join("lib"), dir.join("app/src")]);
        assert_eq!(proj.roots[0].label(), "App");
        assert!(proj.roots[1].missing, "lib is not there");
        assert_eq!(proj.roots[0].folder_exclude_patterns, ["target"]);
        assert_eq!(proj.settings.tab_size, Some(2));
        assert_eq!(proj.settings.word_wrap, Some(WordWrap::On));
        assert_eq!(proj.settings.wrap_width, Some(80));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("color_scheme") && warnings[0].contains("application-wide"));
        assert_eq!(proj.settings.preferences["font_size"], serde_json::json!(20));
        assert!(!proj.settings.preferences.contains_key("color_scheme"));
        assert_eq!(proj.name(), "My App");
        assert_eq!(proj.default_workspace(), dir.join("app/My App.orlo-workspace"));
        // written back: the spelling kept, other keys kept, a new folder relative
        let mut proj = proj;
        proj.roots.push(Root::new(dir.join("app/docs")));
        proj.roots.push(Root::new(PathBuf::from("/elsewhere")));
        let back = proj.to_text();
        assert!(back.contains("\"build_systems\""), "{back}");
        assert!(back.contains("\"../lib\"") && back.contains("\"docs\""), "{back}");
        assert!(back.contains("\"/elsewhere\""), "{back}");
        let (again, _) = Project::parse(&p, &back).unwrap();
        assert_eq!(again.roots.len(), 5);
        assert_eq!(again.roots[3].path, dir.join("app/docs"));
    }

    #[test]
    fn a_broken_project_is_an_error_and_wrong_settings_are_warnings() {
        let p = Path::new("/p/x.sublime-project");
        assert!(Project::parse(p, "[1]").is_err());
        assert!(Project::parse(p, "{ \"folders\": 3 }").is_err());
        assert!(Project::parse(p, "{ \"folders\": [{ \"name\": \"a\" }] }").is_err());
        assert!(Project::parse(p, "{ \"folders\": [{ \"path\": \"a\", \"file_exclude_patterns\": \"x\" }] }").is_err());
        assert!(
            Project::parse(p, "{ \"folders\": [{ \"path\": \"a\", \"file_exclude_patterns\": [\"a{\"] }] }").is_err()
        );
        assert!(Project::parse(p, "{ nope").is_err());
        let (empty, w) = Project::parse(p, "").unwrap();
        assert!(empty.roots.is_empty() && w.is_empty());
        let (proj, w) =
            Project::parse(p, r#"{ "settings": { "tab_size": "big", "rulers": [80, [100, "x"]] } }"#).unwrap();
        assert_eq!(proj.settings.tab_size, None);
        assert_eq!(proj.settings.rulers, Some(vec![80, 100]));
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn file_autosave_project_delay_is_validated_and_inherits_on_invalid_layers() {
        let p = Path::new("/p/x.sublime-project");
        let global = orlo_config::settings::parse(r#"{"file_autosave_delay_ms":750}"#).unwrap().0;
        let delay =
            |s: &orlo_config::settings::Settings| serde_json::to_value(s).unwrap()["file_autosave_delay_ms"].clone();
        for value in ["0", "-1", "1.5", "4294967296", "true", "null", r#""250""#] {
            let (project, warnings) = Project::parse(
                p,
                &format!(r#"{{"settings":{{"file_autosave":true,"file_autosave_delay_ms":{value}}}}}"#),
            )
            .unwrap();
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].contains("file_autosave_delay_ms"));
            let resolved = global.overlay(&project.settings.preferences).unwrap();
            assert!(resolved.file_autosave);
            assert_eq!(delay(&resolved), 750);
            let (project, warnings) = Project::parse(p, &format!(r#"{{"settings":{{"file_autosave_delay_ms":250,"overrides":[{{"path":"*.rs","settings":{{"file_autosave_delay_ms":{value}}}}}]}}}}"#)).unwrap();
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].contains("overrides"));
            assert_eq!(
                delay(&global.overlay(&project.settings.preferences).unwrap().for_file(
                    Some(Path::new("x.rs")),
                    "Rust",
                    "source.rust"
                )),
                250
            );
        }
        let (project, warnings) =
            Project::parse(p, r#"{"settings":{"file_autosave":true,"file_autosave_delay_ms":250.0}}"#).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(delay(&global.overlay(&project.settings.preferences).unwrap()), 250);
        let (_, warnings) = Project::parse(p, &project.to_text()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn excludes_hide_folders_and_files_as_gitignore_lines() {
        let root = Path::new("/r");
        let x = Excludes::new(root, &["node_modules".into(), "build/out".into()], &["*.pyc".into()]);
        assert!(x.hides(Path::new("/r/a/node_modules"), true));
        assert!(!x.hides(Path::new("/r/a/node_modules"), false), "a file of that name stays");
        assert!(x.hides(Path::new("/r/build/out"), true));
        assert!(!x.hides(Path::new("/r/a/build/out"), true), "anchored");
        assert!(x.hides(Path::new("/r/a/b.pyc"), false));
        assert!(!x.hides(Path::new("/r/a/b.pyc"), true), "file patterns never hide folders");
        assert!(!Excludes::none().hides(Path::new("/r/x"), true));
    }
}
