//! Lexical related-file discovery; called only on the background executor.

use orlo_editor::searching::search;
use orlo_project::project::Root;
use regex_automata::meta::Regex;
use ropey::Rope;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug)]
pub struct Related {
    pub path: PathBuf,
    pub reason: &'static str,
}

/// The entries of `folder`; none when it cannot be read.
fn entries(folder: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(folder).into_iter().flatten().flatten().map(|e| e.path())
}

pub fn counterparts(path: &Path) -> Vec<PathBuf> {
    let Some(folder) = path.parent() else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries(folder)
        .filter(|p| {
            p.is_file()
                && p.file_stem() == path.file_stem()
                && p.extension().is_some()
                && path.extension().is_some()
                && p.extension() != path.extension()
        })
        .collect();
    files.sort();
    files
}

pub fn switch_file(path: &Path) -> Option<PathBuf> {
    let files = counterparts(path);
    files.iter().find(|p| p.as_path() > path).or(files.first()).cloned()
}

fn test_base(stem: &str) -> Option<&str> {
    stem.strip_suffix("_test")
        .or_else(|| stem.strip_prefix("test_"))
        .or_else(|| stem.strip_suffix(".test"))
        .or_else(|| stem.strip_suffix(".spec"))
}

fn is_test_pair(a: &Path, b: &Path) -> bool {
    let (Some(a), Some(b)) = (a.file_stem().and_then(|s| s.to_str()), b.file_stem().and_then(|s| s.to_str())) else {
        return false;
    };
    test_base(a) == Some(b) || test_base(b) == Some(a)
}

/// Test names search the project (with its excludes), or just siblings for a loose file.
/// The shared walk's file limit also bounds discovery here.
pub fn discover(path: &Path, roots: &[Root], text: &Rope) -> Vec<Related> {
    let source = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut found = BTreeMap::<PathBuf, (u8, &'static str)>::new();
    let mut add = |candidate: PathBuf, rank, reason| {
        if candidate.is_file() {
            let candidate = candidate.canonicalize().unwrap_or(candidate);
            if candidate != source {
                let entry = found.entry(candidate).or_insert((rank, reason));
                if rank < entry.0 {
                    *entry = (rank, reason);
                }
            }
        }
    };
    for p in counterparts(path) {
        add(p, 0, "counterpart");
    }
    let scope = search::Scope {
        roots: orlo_project::project::present(roots),
        files: Vec::new(),
        include: Vec::new(),
        exclude: Vec::new(),
        gitignore: true,
        filter_files: false,
        root_excludes: roots
            .iter()
            .map(|r| (r.path.clone(), r.folder_exclude_patterns.clone(), r.file_exclude_patterns.clone()))
            .collect(),
    };
    let mut remaining = crate::index::MAX_FILES;
    for root in &scope.roots {
        for p in search::walk(&scope, root).flatten().take(remaining) {
            remaining -= 1;
            if is_test_pair(path, &p) {
                add(p, 1, "test");
            }
        }
        if remaining == 0 {
            break;
        }
    }
    let folder = path.parent().unwrap_or(Path::new("."));
    let in_project = scope.roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        source.starts_with(root)
    });
    if !in_project {
        for p in entries(folder).take(crate::index::MAX_FILES) {
            if is_test_pair(path, &p) {
                add(p, 1, "test");
            }
        }
    }
    static QUOTED: OnceLock<Regex> = OnceLock::new();
    static MODULE: OnceLock<Regex> = OnceLock::new();
    static PYTHON: OnceLock<Regex> = OnceLock::new();
    let quoted = QUOTED.get_or_init(|| Regex::new(r#"'([^'\r\n]*)'|"([^"\r\n]*)""#).unwrap());
    let module = MODULE.get_or_init(|| Regex::new(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;").unwrap());
    let python = PYTHON.get_or_init(|| Regex::new(r"\bfrom\s+(\.+)([\w.]*)(?:\s+import\s+([^\r\n#]+))?").unwrap());
    let bases: Vec<_> = std::iter::once(folder.to_path_buf()).chain(roots.iter().map(|r| r.path.clone())).collect();
    let module_dir = match (path.extension().is_some_and(|ext| ext == "rs"), path.file_stem().and_then(|s| s.to_str()))
    {
        (true, Some(stem)) if !matches!(stem, "lib" | "main" | "mod") => folder.join(stem),
        _ => folder.to_path_buf(),
    };
    let mut seen = HashSet::new();
    // All line copies and lexical scans stay here, including a huge single line.
    for line in text.lines().take(10_000) {
        let line = line.to_string();
        for caps in quoted.captures_iter(&line) {
            let m = caps.get_group(1).or_else(|| caps.get_group(2)).unwrap();
            let value = &line[m.range()];
            let prefix = line[..caps.get_match().unwrap().start()].trim_end();
            let included = prefix.strip_suffix('(').is_some_and(|p| p.trim_end().ends_with("require"))
                || (prefix.ends_with("from") && prefix.split_whitespace().any(|w| w == "import"))
                || prefix.rsplit_once('#').is_some_and(|(_, p)| p.trim() == "include");
            if value.is_empty() || !seen.insert((value.to_owned(), included)) {
                continue;
            }
            let (rank, reason) = match included {
                true => (2, "included"),
                false => (3, "referenced"),
            };
            for base in &bases {
                let candidate = base.join(value);
                if candidate.is_file() {
                    add(candidate, rank, reason);
                } else if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    add(base.join(format!("{value}.{ext}")), rank, reason);
                }
            }
        }
        for caps in module.captures_iter(&line) {
            let name = &line[caps.get_group(1).unwrap().range()];
            add(module_dir.join(format!("{name}.rs")), 2, "included");
            add(module_dir.join(name).join("mod.rs"), 2, "included");
        }
        for caps in python.captures_iter(&line) {
            let mut base = folder.to_path_buf();
            for _ in 1..caps.get_group(1).unwrap().len() {
                base.pop();
            }
            let mut modules = Vec::new();
            let name = &line[caps.get_group(2).unwrap().range()];
            if !name.is_empty() {
                base = base.join(name.replace('.', "/"));
                modules.push(base.clone());
            }
            if let Some(import) = caps.get_group(3) {
                let names = line[import.range()].trim_matches(['(', ')', ' ']).split(',');
                modules.extend(
                    (names.filter_map(|name| name.split_whitespace().next()))
                        .filter(|name| name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                        .map(|name| base.join(name)),
                );
            }
            for module in modules {
                add(module.with_extension("py"), 2, "included");
                add(module.join("__init__.py"), 2, "included");
            }
        }
    }
    let mut rows: Vec<_> = found.into_iter().collect();
    rows.sort_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| a.0.cmp(&b.0)));
    rows.into_iter().map(|(path, (_, reason))| Related { path, reason }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orlo_project::project::Root;
    use ropey::Rope;
    use std::path::PathBuf;

    fn fixture(name: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orlo-related-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        dir
    }

    fn has(rows: &[Related], path: PathBuf, reason: &str) -> bool {
        rows.iter().any(|r| r.path == path && r.reason == reason)
    }

    #[test]
    fn counterparts_cycle_sorted_different_extensions_and_wrap() {
        let dir = fixture("cycle", &["foo.c", "foo.h", "foo.cpp", "foo.json", "foo", "food.h"]);
        let names: Vec<_> = counterparts(&dir.join("foo.h"))
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["foo.c", "foo.cpp", "foo.json"]);
        assert_eq!(switch_file(&dir.join("foo.h")), Some(dir.join("foo.json")));
        assert_eq!(switch_file(&dir.join("foo.json")), Some(dir.join("foo.c")));
        assert_eq!(switch_file(&dir.join("food.h")), None);
    }

    #[test]
    fn all_reasons_resolve_existing_files_and_deduplicate() {
        let dir = fixture(
            "reasons",
            &[
                "src/foo.js",
                "src/foo.ts",
                "tests/foo_test.js",
                "tests/test_foo.js",
                "tests/foo.test.js",
                "tests/foo.spec.js",
                "src/inc.h",
                "src/from.js",
                "src/required.js",
                "src/rustmod.rs",
                "src/nested/mod.rs",
                "src/py.py",
                "src/sub.py",
                "src/ref.js",
                "root.txt",
                "src/space name.txt",
            ],
        );
        let text = Rope::from_str(concat!(
            "#include \"inc.h\"\nimport x from './from';\nconst x = require('./required');\n",
            "mod rustmod;\nmod nested;\nfrom .py import thing\nfrom . import sub\n",
            "'ref'\n'root.txt'\n'space name.txt'\n'missing'\n'./from'\n",
        ));
        let rows = discover(&dir.join("src/foo.js"), &[Root::new(dir.clone())], &text);
        for (path, reason) in [
            ("src/foo.ts", "counterpart"),
            ("tests/foo_test.js", "test"),
            ("tests/test_foo.js", "test"),
            ("tests/foo.test.js", "test"),
            ("tests/foo.spec.js", "test"),
            ("src/inc.h", "included"),
            ("src/from.js", "included"),
            ("src/required.js", "included"),
            ("src/rustmod.rs", "included"),
            ("src/nested/mod.rs", "included"),
            ("src/py.py", "included"),
            ("src/sub.py", "included"),
            ("src/ref.js", "referenced"),
            ("root.txt", "referenced"),
            ("src/space name.txt", "referenced"),
        ] {
            assert!(has(&rows, dir.join(path), reason), "missing {path} ({reason}): {rows:?}");
        }
        assert_eq!(rows.len(), 15, "missing paths and duplicate reasons must not produce rows");
    }

    #[test]
    fn a_file_reached_through_a_link_does_not_list_itself() {
        let dir = fixture("self-link", &["src/foo.js", "src/.js", "src/bar.js"]);
        std::os::unix::fs::symlink(dir.join("src"), dir.join("link")).unwrap();
        let text = Rope::from_str("'foo.js'\n''\n'bar'\n'bar'\n");
        let rows = discover(&dir.join("link/foo.js"), &[], &text);
        let paths: Vec<_> = rows.iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, vec![dir.join("src/bar.js")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reverse_test_names_and_sibling_fallback() {
        let dir = fixture("reverse", &["foo.rs", "foo_test.rs", "test_foo.rs", "foo.test.rs", "foo.spec.rs"]);
        for name in ["foo_test.rs", "test_foo.rs", "foo.test.rs", "foo.spec.rs"] {
            let rows = discover(&dir.join(name), &[], &Rope::new());
            assert!(has(&rows, dir.join("foo.rs"), "test"), "{name}: {rows:?}");
        }
    }

    #[test]
    fn project_test_discovery_does_not_reintroduce_excluded_siblings() {
        let dir = fixture("excluded-tests", &["foo.rs", "foo_test.rs", "test_foo.rs", "foo.spec.rs"]);
        std::fs::create_dir(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".gitignore"), "test_foo.rs\n").unwrap();
        let mut root = Root::new(dir.clone());
        root.file_exclude_patterns = vec!["*_test.rs".into()];
        let rows = discover(&dir.join("foo.rs"), &[root], &Rope::new());
        let tests: Vec<_> = rows.iter().filter(|r| r.reason == "test").map(|r| r.path.clone()).collect();
        assert_eq!(tests, vec![dir.join("foo.spec.rs")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loose_file_keeps_sibling_tests_with_an_unrelated_project_open() {
        let dir = fixture("outside-project", &["project/main.rs", "loose/foo.rs", "loose/foo_test.rs"]);
        let rows = discover(&dir.join("loose/foo.rs"), &[Root::new(dir.join("project"))], &Rope::new());
        assert!(has(&rows, dir.join("loose/foo_test.rs"), "test"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scan_stops_after_ten_thousand_lines_and_handles_a_huge_line() {
        let dir = fixture("limit", &["main.rs", "before.rs", "after.rs", "huge.rs"]);
        let text = Rope::from_str(&format!("{}'before'\n'after'\n", "\n".repeat(9999)));
        let rows = discover(&dir.join("main.rs"), &[], &text);
        assert!(rows.iter().any(|r| r.path == dir.join("before.rs")));
        assert!(!rows.iter().any(|r| r.path == dir.join("after.rs")));
        let huge = Rope::from_str(&format!("{}'huge'", " ".repeat(4_000_000)));
        assert!(discover(&dir.join("main.rs"), &[], &huge).iter().any(|r| r.path == dir.join("huge.rs")));
    }

    #[test]
    fn python_parent_imports_resolve_modules_and_packages() {
        let dir = fixture(
            "python",
            &[
                "pkg/sub/main.py",
                "pkg/parent.py",
                "pkg/sub/package/__init__.py",
                "pkg/sub/sibling.py",
                "pkg/sub/second.py",
            ],
        );
        let rows = discover(
            &dir.join("pkg/sub/main.py"),
            &[],
            &Rope::from_str("from ..parent import x\nfrom .package import x\nfrom . import sibling, second as alias\n"),
        );
        for path in ["pkg/parent.py", "pkg/sub/package/__init__.py", "pkg/sub/sibling.py", "pkg/sub/second.py"] {
            assert!(has(&rows, dir.join(path), "included"), "{rows:?}");
        }
    }

    #[test]
    fn navigation_rust_modules_resolve_from_the_file_module_directory() {
        let dir = fixture(
            "rust-module-dirs",
            &[
                "src/lib.rs",
                "src/main.rs",
                "src/foo.rs",
                "src/child.rs",
                "src/foo/child.rs",
                "src/foo/nested/mod.rs",
                "src/foo/mod.rs",
            ],
        );
        let included = |file: &str, text: &str| {
            discover(&dir.join(file), &[Root::new(dir.clone())], &Rope::from_str(text))
                .into_iter()
                .filter(|r| r.reason == "included")
                .map(|r| r.path)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            included("src/foo.rs", "mod child;\nmod nested;"),
            vec![dir.join("src/foo/child.rs"), dir.join("src/foo/nested/mod.rs")]
        );
        for file in ["src/lib.rs", "src/main.rs"] {
            assert_eq!(included(file, "mod child;"), vec![dir.join("src/child.rs")]);
        }
        assert_eq!(included("src/foo/mod.rs", "mod child;"), vec![dir.join("src/foo/child.rs")]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
