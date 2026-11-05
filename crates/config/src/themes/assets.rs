//! Syntax and appearance asset discovery shared by configuration and highlighting.

use std::path::{Path, PathBuf};

pub fn asset_matches(support: &Path, path: &Path, selected: &str) -> bool {
    path == Path::new(selected)
        || path.strip_prefix(support).is_ok_and(|p| p == Path::new(selected))
        || path.file_name().is_some_and(|n| n == selected)
}

/// Expanded package directories share the same formats as loose customizations.
/// Symlink directories are skipped so a broken package cannot create a traversal cycle; linked
/// files are followed.
pub fn customization_files(support: &Path) -> (Vec<PathBuf>, Vec<String>) {
    fn visit(path: &Path, files: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                errors.push(format!("{}: {e}", path.display()));
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    errors.push(e.to_string());
                    continue;
                }
            };
            match entry.file_type() {
                Ok(t) if t.is_dir() => visit(&entry.path(), files, errors),
                Ok(t) if t.is_file() => files.push(entry.path()),
                Ok(t) if t.is_symlink() && entry.path().is_file() => files.push(entry.path()),
                Ok(_) => {}
                Err(e) => errors.push(format!("{}: {e}", entry.path().display())),
            }
        }
    }
    let (mut files, mut errors) = (Vec::new(), Vec::new());
    for folder in ["Syntaxes", "Themes"] {
        visit(&support.join(folder), &mut files, &mut errors);
    }
    match crate::automation::packages_with_errors(&support.join("Packages")) {
        Ok((packages, package_errors)) => {
            errors.extend(package_errors);
            for package in packages.into_iter().filter(|p| p.enabled) {
                visit(&package.path, &mut files, &mut errors);
            }
        }
        Err(error) => errors.push(error),
    }
    files.sort();
    (files, errors)
}

/// The language folders under `Languages/`, sorted: one folder per language, named for its id.
/// A folder holding a `.disabled` file is left out, as a package is. Loose files and hidden
/// folders are skipped; a linked folder is followed, so a checkout can be linked in.
pub fn language_folders(support: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let root = support.join("Languages");
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![format!("{}: {e}", root.display())]),
    };
    let (mut folders, mut errors) = (Vec::new(), Vec::new());
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                errors.push(format!("{}: {e}", root.display()));
                continue;
            }
        };
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        match std::fs::metadata(&path) {
            Ok(m) if m.is_dir() && !path.join(".disabled").exists() => folders.push(path),
            Ok(_) => {}
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    folders.sort();
    (folders, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_folders_skip_files_hidden_and_disabled_folders() {
        let dir = std::env::temp_dir().join(format!("orlo-language-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(language_folders(&dir), (Vec::new(), Vec::new()));
        let languages = dir.join("Languages");
        for folder in ["pony", "nim", ".hidden", "off"] {
            std::fs::create_dir_all(languages.join(folder)).unwrap();
        }
        std::fs::write(languages.join("off/.disabled"), "").unwrap();
        std::fs::write(languages.join("notes.txt"), "").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(languages.join("pony"), languages.join("linked")).unwrap();
        let (folders, errors) = language_folders(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<_> = folders.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        #[cfg(unix)]
        assert_eq!(names, ["linked", "nim", "pony"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_broken_package_is_reported_and_linked_files_and_other_packages_still_load() {
        let dir = std::env::temp_dir().join(format!("orlo-customizations-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let packages = dir.join("Packages");
        for name in ["good", "broken", "renamed"] {
            std::fs::create_dir_all(packages.join(name)).unwrap();
        }
        std::fs::write(packages.join("good/package.json"), r#"{"name":"good"}"#).unwrap();
        std::fs::write(packages.join("good/Good.tmTheme"), "").unwrap();
        std::fs::write(packages.join("broken/package.json"), "not json").unwrap();
        std::fs::write(packages.join("broken/Broken.tmTheme"), "").unwrap();
        std::fs::write(packages.join("renamed/package.json"), r#"{"name":"other"}"#).unwrap();
        std::fs::create_dir_all(dir.join("Themes")).unwrap();
        std::fs::write(dir.join("Real.tmTheme"), "").unwrap();
        std::os::unix::fs::symlink(dir.join("Real.tmTheme"), dir.join("Themes/Linked.tmTheme")).unwrap();
        std::os::unix::fs::symlink(dir.join("Missing"), dir.join("Themes/Dangling.tmTheme")).unwrap();
        let (mut files, errors) = customization_files(&dir);
        files.retain(|f| f.extension().is_some_and(|e| e == "tmTheme"));
        assert_eq!(files, [packages.join("good/Good.tmTheme"), dir.join("Themes/Linked.tmTheme")]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("broken")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("folder name")), "{errors:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
