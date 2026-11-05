//! Named saved searches (a query, its replacement, its toggles and its Where), kept in
//! `~/Library/Application Support/Orlo/Saved Searches.json`, written atomically. A saved search
//! holds what was typed into the panels, never text from a document. Files Orlo reads (its
//! own and imports) are checked whole before anything is used: a bad entry names itself.

use crate::buffer::FindOpts;
use crate::search::pattern::Replacer;
use crate::search::results::SearchFlags;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file format's version; a newer one is refused rather than half read.
pub const VERSION: u32 = 1;

/// Longest name, query, replacement or Where Orlo saves or imports, in chars.
const MAX_FIELD: usize = 64 << 10;
/// Most searches one file may hold.
const MAX_SEARCHES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub replacement: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    /// Find in Files' scope; empty for a search of the document.
    #[serde(default, rename = "where")]
    pub where_: String,
    #[serde(default)]
    pub flags: SearchFlags,
}

impl SavedSearch {
    pub fn opts(&self) -> FindOpts {
        FindOpts { regex: self.regex, case_sensitive: self.case_sensitive, whole_word: self.whole_word, wrap: true }
    }

    /// Why it cannot be used: an empty or long field, a pattern or replacement that does not
    /// compile.
    pub fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("its name is empty".into());
        }
        if self.query.is_empty() {
            return Some("its query is empty".into());
        }
        let long =
            [&self.name, &self.query, &self.replacement, &self.where_].iter().any(|f| f.chars().count() > MAX_FIELD);
        if long {
            return Some(format!("a field is over {} characters", MAX_FIELD));
        }
        let preserve = self.flags.preserve_case;
        Replacer::new(&self.query, &self.replacement, &self.opts(), preserve).err()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    searches: Vec<SavedSearch>,
}

/// `Application Support/Orlo/Saved Searches.json`
pub fn path() -> PathBuf {
    orlo_utils::paths::support_dir().join("Saved Searches.json")
}

/// The searches in `text`, every one checked; all the problems, each naming its search.
pub fn parse(text: &str) -> Result<Vec<SavedSearch>, Vec<String>> {
    let store: Store = serde_json::from_str(text).map_err(|e| vec![format!("Not a saved searches file: {e}")])?;
    if store.version != VERSION {
        return Err(vec![format!("Version {} is not one this Orlo reads (it reads {VERSION})", store.version)]);
    }
    if store.searches.len() > MAX_SEARCHES {
        return Err(vec![format!("More than {MAX_SEARCHES} searches")]);
    }
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, s) in store.searches.iter().enumerate() {
        let who = format!("Search {} (\"{}\")", i + 1, s.name);
        if let Some(why) = s.problem() {
            errors.push(format!("{who}: {why}"));
        } else if !seen.insert(s.name.trim().to_lowercase()) {
            errors.push(format!("{who}: another search has this name"));
        }
    }
    match errors.is_empty() {
        true => Ok(store.searches),
        false => Err(errors),
    }
}

/// The saved searches; none when the file is not there yet.
pub fn list(path: &Path) -> Result<Vec<SavedSearch>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text).map_err(|e| e.join("\n")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `searches` as the file holds them, sorted by name.
pub fn to_text(searches: &[SavedSearch]) -> String {
    let mut searches = searches.to_vec();
    searches.sort_by_key(|s| s.name.to_lowercase());
    let store = Store { version: VERSION, searches };
    serde_json::to_string_pretty(&store).unwrap_or_default() + "\n"
}

/// Written beside itself and renamed into place: a failure leaves the old file.
pub fn write(path: &Path, searches: &[SavedSearch]) -> Result<(), String> {
    let text = to_text(searches);
    parse(&text).map_err(|e| e.join("\n"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::buffer::write_atomic(path, &text).map_err(|e| e.to_string())
}

fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `name`, or with " 2", " 3" … after it, the first not in `list`.
pub fn unique_name(list: &[SavedSearch], name: &str) -> String {
    let name = name.trim();
    (std::iter::once(name.to_string()).chain((2..).map(|n| format!("{name} {n}"))))
        .find(|c| !list.iter().any(|s| same(&s.name, c)))
        .unwrap_or_default()
}

/// Add `s`, or with `overwrite` put it in place of the search of its name; a name already
/// taken without `overwrite` is refused.
pub fn upsert(list: &mut Vec<SavedSearch>, s: SavedSearch, overwrite: bool) -> Result<(), String> {
    if let Some(why) = s.problem() {
        return Err(why);
    }
    match list.iter().position(|x| same(&x.name, &s.name)) {
        Some(i) if overwrite => list[i] = s,
        Some(_) => return Err(format!("A saved search is already called \"{}\"", s.name.trim())),
        None => list.push(s),
    }
    Ok(())
}

pub fn rename(list: &mut [SavedSearch], from: &str, to: &str) -> Result<(), String> {
    let to = to.trim();
    if to.is_empty() {
        return Err("Enter a name".into());
    }
    if !same(from, to) && list.iter().any(|s| same(&s.name, to)) {
        return Err(format!("A saved search is already called \"{to}\""));
    }
    let s = (list.iter_mut().find(|s| same(&s.name, from)))
        .ok_or_else(|| format!("No saved search is called \"{from}\""))?;
    s.name = to.to_string();
    Ok(())
}

pub fn delete(list: &mut Vec<SavedSearch>, name: &str) -> bool {
    let n = list.len();
    list.retain(|s| !same(&s.name, name));
    list.len() < n
}

/// Import: each incoming search is added, one whose name is taken under its name with " 2",
/// " 3" … after it. The names it was added under, in order.
pub fn import(list: &mut Vec<SavedSearch>, incoming: Vec<SavedSearch>) -> Vec<String> {
    let mut names = Vec::new();
    for mut s in incoming {
        s.name = unique_name(list, &s.name);
        names.push(s.name.clone());
        list.push(s);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(name: &str, query: &str) -> SavedSearch {
        SavedSearch {
            name: name.into(),
            query: query.into(),
            replacement: String::new(),
            regex: false,
            case_sensitive: false,
            whole_word: false,
            where_: String::new(),
            flags: SearchFlags::default(),
        }
    }

    #[test]
    fn a_file_round_trips_and_is_written_atomically() {
        let dir = std::env::temp_dir().join("orlo-saved-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("Saved Searches.json");
        assert_eq!(list(&p).unwrap(), Vec::new(), "no file yet: none");
        let mut s = search("TODOs", r"TODO(?=:)");
        s.regex = true;
        s.where_ = "~/src, *.rs, -target/".into();
        s.replacement = r"\U$0".into();
        write(&p, &[s.clone(), search("b", "x")]).unwrap();
        let back = list(&p).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[1], s, "sorted by name; every field kept");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"where\": \"~/src, *.rs, -target/\""), "{text}");
        let leftovers: Vec<_> =
            std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name() != "Saved Searches.json").collect();
        assert!(leftovers.is_empty(), "no temp file left");
    }

    #[test]
    fn bad_files_and_bad_searches_are_refused_with_reasons() {
        let bad = [
            ("not json", "Not a saved searches file"),
            (r#"{"version": 2, "searches": []}"#, "Version 2"),
            (r#"{"version": 1, "searches": [{"name": "a", "query": "x", "colour": 1}]}"#, "unknown field"),
            (r#"{"version": 1, "searches": [{"name": "", "query": "x"}]}"#, "name is empty"),
            (r#"{"version": 1, "searches": [{"name": "a", "query": "("}]}"#, "unclosed"),
            (
                r#"{"version": 1, "searches": [{"name": "a", "query": "(x)", "regex": true, "replacement": "$2"}]}"#,
                "group 2",
            ),
            (
                r#"{"version": 1, "searches": [{"name": "a", "query": "x"}, {"name": "A ", "query": "y"}]}"#,
                "another search has this name",
            ),
        ];
        for (text, why) in bad {
            let mut text = text.to_string();
            if why == "unclosed" {
                text = text.replace(r#""query": "(""#, r#""query": "(", "regex": true"#);
            }
            let e = parse(&text).unwrap_err().join("\n");
            assert!(e.contains(why), "{text}: {e}");
        }
    }

    #[test]
    fn names_are_unique_predictably() {
        let mut l = vec![search("Find TODO", "TODO")];
        assert!(upsert(&mut l, search("find todo", "x"), false).is_err());
        upsert(&mut l, search("find todo", "y"), true).unwrap();
        assert_eq!((l.len(), l[0].query.as_str()), (1, "y"));
        let names = import(&mut l, vec![search("find todo", "a"), search("find todo", "b"), search("other", "c")]);
        assert_eq!(names, ["find todo 2", "find todo 3", "other"]);
        assert!(rename(&mut l, "other", "find todo 2").is_err());
        rename(&mut l, "other", "Renamed").unwrap();
        assert!(delete(&mut l, "renamed"));
        assert!(!delete(&mut l, "renamed"));
        assert_eq!(l.len(), 3);
    }

    #[test]
    fn writing_refuses_a_store_the_reader_cannot_load_and_keeps_the_old_file() {
        let dir = std::env::temp_dir().join(format!("orlo-saved-limits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Saved Searches.json");
        let original = vec![search("keep", "x")];
        write(&path, &original).unwrap();
        let old = std::fs::read(&path).unwrap();
        let mut too_many = Vec::new();
        for i in 0..=MAX_SEARCHES {
            upsert(&mut too_many, search(&format!("Search {i}"), "x"), false).unwrap();
        }
        let mut renamed = original.clone();
        rename(&mut renamed, "keep", &"n".repeat(MAX_FIELD + 1)).unwrap();
        for invalid in [too_many, renamed] {
            assert!(write(&path, &invalid).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), old);
            assert_eq!(list(&path).unwrap(), original);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
