//! EditorConfig's repository policies. Nearest files and later matching sections win;
//! `unset` removes an inherited policy. Invalid values are reported and never change text.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties {
    pub spaces: Option<bool>,
    pub indent_size: Option<usize>,
    pub indent_is_tab: bool,
    pub tab_width: Option<usize>,
    pub line_ending: Option<&'static str>,
    pub bom: Option<bool>,
    pub trim: Option<bool>,
    pub final_newline: Option<bool>,
}

struct Section {
    glob: Glob,
    values: Vec<(String, String)>,
}

fn read_policy(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("EditorConfig must be a regular file"));
    }
    const LIMIT: u64 = 1 << 20;
    let mut text = String::new();
    file.take(LIMIT + 1).read_to_string(&mut text)?;
    if text.len() as u64 > LIMIT {
        return Err(std::io::Error::other("EditorConfig exceeds 1 MiB"));
    }
    Ok(text)
}

/// Search from the file's directory up to `root = true` or the filesystem root.
pub fn load(path: Option<&Path>) -> (Properties, Vec<String>) {
    let Some(path) = path else {
        return (Properties::default(), Vec::new());
    };
    let path =
        if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for dir in path.parent().into_iter().flat_map(Path::ancestors) {
        let config = dir.join(".editorconfig");
        match read_policy(&config) {
            Ok(text) => {
                let (root, sections, errors) = parse(&text, &config);
                warnings.extend(errors);
                files.push((dir.to_path_buf(), sections));
                if root {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warnings.push(format!("{}: {e}", config.display())),
        }
    }
    let mut values = HashMap::new();
    for (dir, sections) in files.into_iter().rev() {
        let relative = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy();
        for section in sections {
            if section.glob.matches(&relative) {
                for (key, value) in section.values {
                    if value == "unset" {
                        values.remove(&key);
                    } else {
                        values.insert(key, value);
                    }
                }
            }
        }
    }
    let get = |key: &str| values.get(key).map(String::as_str);
    let number = |key: &str| get(key).and_then(|s| s.parse().ok());
    (
        Properties {
            spaces: get("indent_style").map(|s| s == "space"),
            indent_size: number("indent_size"),
            indent_is_tab: get("indent_size") == Some("tab"),
            tab_width: number("tab_width"),
            line_ending: get("end_of_line").map(|s| match s {
                "crlf" => "\r\n",
                "cr" => "\r",
                _ => "\n",
            }),
            bom: get("charset").map(|s| s == "utf-8-bom"),
            trim: get("trim_trailing_whitespace").map(|s| s == "true"),
            final_newline: get("insert_final_newline").map(|s| s == "true"),
        },
        warnings,
    )
}

fn parse(text: &str, path: &Path) -> (bool, Vec<Section>, Vec<String>) {
    let mut root = false;
    let mut sections = Vec::<Section>::new();
    let mut warnings = Vec::new();
    // `None` before the first `[pattern]`, then whether the current one is valid.
    let mut section = None;
    for (line, raw) in text.strip_prefix('\u{feff}').unwrap_or(text).lines().enumerate() {
        let text = raw.trim();
        let mut warn = |error: String| warnings.push(format!("{}:{}: {error}", path.display(), line + 1));
        if text.is_empty() || text.starts_with(['#', ';']) {
            continue;
        }
        if text.starts_with('[') && text.ends_with(']') {
            let glob = Glob::new(&text[1..text.len() - 1]);
            section = Some(glob.is_ok());
            match glob {
                Ok(glob) => sections.push(Section { glob, values: Vec::new() }),
                Err(e) => warn(e),
            }
            continue;
        }
        let Some((key, value)) = text.split_once('=') else {
            warn("expected [pattern] or key = value".into());
            continue;
        };
        let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
        if section != Some(true) {
            if section.is_none() && key == "root" {
                root = value == "true";
            }
            continue;
        }
        let valid = match key.as_str() {
            "indent_style" => matches!(value.as_str(), "space" | "tab"),
            "indent_size" if value == "tab" => true,
            "indent_size" | "tab_width" => value.parse::<usize>().is_ok_and(|n| (1..=16).contains(&n)),
            "end_of_line" => matches!(value.as_str(), "lf" | "cr" | "crlf"),
            "charset" => matches!(value.as_str(), "utf-8" | "utf-8-bom"),
            "trim_trailing_whitespace" | "insert_final_newline" => {
                matches!(value.as_str(), "true" | "false")
            }
            _ => continue,
        };
        if valid || value == "unset" {
            sections.last_mut().unwrap().values.push((key, value));
        } else if key == "charset" {
            warn(format!("unsupported charset {value:?}; Orlo saves UTF-8 or UTF-8 with BOM"));
        } else {
            warn(format!("unsupported {key} = {value:?} (indentation widths must be 1–16)"));
        }
    }
    (root, sections, warnings)
}

/// A validated, compiled EditorConfig path pattern, reusable during detection.
#[derive(Debug)]
pub struct Glob {
    regex: regex_automata::meta::Regex,
}

impl Glob {
    pub fn new(pattern: &str) -> Result<Self, String> {
        if pattern.len() > 4096 {
            return Err("glob exceeds 4096 bytes".into());
        }
        let body = glob_regex(pattern.trim_start_matches('/'), 0)?;
        let mut in_class = false;
        let slash = pattern.chars().any(|c| {
            in_class = (in_class || c == '[') && c != ']';
            c == '/' && !in_class
        });
        let prefix = if slash { "" } else { "(?:.*/)?" };
        let regex = regex_automata::meta::Regex::new(&format!("(?s)^{prefix}{body}$")).map_err(|e| e.to_string())?;
        Ok(Self { regex })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.regex.is_match(path)
    }
}

/// The same path syntax is used in scoped preferences; malformed patterns never match.
pub fn matches(pattern: &str, path: &str) -> bool {
    Glob::new(pattern).is_ok_and(|g| g.matches(path))
}

pub fn validate_pattern(pattern: &str) -> Result<(), String> {
    Glob::new(pattern).map(|_| ())
}

fn glob_regex(pattern: &str, nesting: usize) -> Result<String, String> {
    if nesting > 32 {
        return Err("glob alternatives exceed 32 nested levels".into());
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '\\' => {
                let c = chars.get(i).copied().unwrap_or('\\');
                out.push_str(&regex_syntax::escape(&c.to_string()));
                i += usize::from(i < chars.len());
            }
            '?' => out.push_str("[^/]"),
            '*' if chars.get(i) == Some(&'*') => {
                i += 1;
                if chars.get(i) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 1;
                } else {
                    out.push_str(".*");
                }
            }
            '*' => out.push_str("[^/]*"),
            '[' => {
                if let Some(n) = chars[i..].iter().position(|&c| c == ']') {
                    out.push('[');
                    let mut start = i;
                    if chars.get(start) == Some(&'!') {
                        out.push('^');
                        start += 1;
                    }
                    for c in &chars[start..i + n] {
                        out.push_str(&regex_syntax::escape(&c.to_string()));
                    }
                    out.push(']');
                    i += n + 1;
                } else {
                    out.push_str("\\[");
                }
            }
            '{' => {
                let mut depth = 1;
                let mut end = i;
                let mut parts = Vec::new();
                let mut start = i;
                while end < chars.len() {
                    match chars[end] {
                        '\\' => {
                            end += 2;
                            continue;
                        }
                        '[' => {
                            if let Some(n) = chars[end + 1..].iter().position(|&c| c == ']') {
                                end += n + 2;
                                continue;
                            }
                        }
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        ',' if depth == 1 => {
                            parts.push(chars[start..end].iter().collect::<String>());
                            start = end + 1;
                        }
                        _ => {}
                    }
                    end += 1;
                }
                if end >= chars.len() {
                    out.push_str("\\{");
                    continue;
                }
                let body: String = chars[i..end].iter().collect();
                if !parts.is_empty() {
                    parts.push(chars[start..end].iter().collect());
                    let choices: Result<Vec<_>, _> = parts.iter().map(|p| glob_regex(p, nesting + 1)).collect();
                    out.push_str(&format!("(?:{})", choices?.join("|")));
                } else if let Some((low, high)) =
                    body.split_once("..").and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?)))
                {
                    if low >= high {
                        return Err(format!("invalid numeric glob range {{{body}}}"));
                    }
                    out.push_str(&numeric_range(low, high));
                } else {
                    out.push_str(&regex_syntax::escape(&format!("{{{body}}}")));
                }
                i = end + 1;
            }
            _ => out.push_str(&regex_syntax::escape(&c.to_string())),
        }
    }
    Ok(out)
}

fn numeric_range(low: i64, high: i64) -> String {
    fn digits(low: &[u8], high: &[u8]) -> String {
        if low.is_empty() {
            return String::new();
        }
        if low[0] == high[0] {
            return format!("{}{}", low[0] as char, digits(&low[1..], &high[1..]));
        }
        let tail = low.len() - 1;
        if low[1..].iter().all(|&d| d == b'0') && high[1..].iter().all(|&d| d == b'9') {
            return format!("[{}-{}][0-9]{{{tail}}}", low[0] as char, high[0] as char);
        }
        let mut pieces = vec![format!("{}{}", low[0] as char, digits(&low[1..], &vec![b'9'; tail]))];
        if high[0] > low[0] + 1 {
            pieces.push(format!("[{}-{}][0-9]{{{tail}}}", (low[0] + 1) as char, (high[0] - 1) as char));
        }
        pieces.push(format!("{}{}", high[0] as char, digits(&vec![b'0'; tail], &high[1..])));
        format!("(?:{})", pieces.join("|"))
    }
    fn positive(low: u64, high: u64) -> String {
        let low = low.to_string();
        let high = high.to_string();
        let pieces: Vec<_> = (low.len()..=high.len())
            .map(|len| {
                let a = if len == low.len() { low.clone() } else { format!("1{}", "0".repeat(len - 1)) };
                let b = if len == high.len() { high.clone() } else { "9".repeat(len) };
                digits(a.as_bytes(), b.as_bytes())
            })
            .collect();
        format!("0*(?:{})", pieces.join("|"))
    }
    let mut pieces = Vec::new();
    if low < 0 {
        pieces.push(format!("-{}", positive(if high < 0 { high.unsigned_abs() } else { 1 }, low.unsigned_abs())));
    }
    if high >= 0 {
        pieces.push(positive(low.max(0) as u64, high as u64));
    }
    format!("(?:{})", pieces.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_and_oversized_editorconfig_files_are_rejected_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let dir = std::env::temp_dir().join(format!("orlo-ec-special-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("fifo");
        let _ = std::fs::remove_file(&fifo);
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(read_policy(&fifo).unwrap_err().to_string().contains("regular file"));
        let big = dir.join("big");
        std::fs::write(&big, "x".repeat((1 << 20) + 1)).unwrap();
        assert!(read_policy(&big).unwrap_err().to_string().contains("1 MiB"));
        let _ = std::fs::remove_dir_all(dir);
        assert!(validate_pattern(&"x".repeat(4097)).is_err());
        let nested = format!("{}x{}", "{a,".repeat(40), "}".repeat(40));
        assert!(validate_pattern(&nested).is_err());
    }

    #[test]
    fn editorconfig_globs_cover_paths_alternatives_classes_ranges_and_escaping() {
        for (glob, path, yes) in [
            ("*.rs", "src/main.rs", true),
            ("src/*.rs", "src/nested/main.rs", false),
            ("src/**/main.rs", "src/main.rs", true),
            ("src/**/main.rs", "src/a/b/main.rs", true),
            ("*.{rs,{c,h}}", "src/main.h", true),
            ("file{1..3}.txt", "file2.txt", true),
            ("file{-2..2}.txt", "file-1.txt", true),
            ("file{1..3}.txt", "file4.txt", false),
            ("file{1..3}*.txt", "file23.txt", true),
            ("file{10..1000000000}.txt", "file999999999.txt", true),
            ("file{10..1000000000}.txt", "file1000000001.txt", false),
            ("[!x]?\\*.txt", "ab*.txt", true),
            ("[a-c].txt", "b.txt", false), // EditorConfig classes contain literal characters.
            ("[a-c].txt", "-.txt", true),
        ] {
            assert_eq!(matches(glob, path), yes, "{glob}: {path}");
        }
    }

    #[test]
    fn editorconfig_nearest_sections_unset_and_root_are_applied() {
        let dir = std::env::temp_dir().join(format!("orlo-editorconfig-{}", std::process::id()));
        let root = dir.join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(dir.join(".editorconfig"), "[*]\nindent_size=8\ncharset=utf-8-bom").unwrap();
        std::fs::write(root.join(".editorconfig"), "root=true\n[*]\nindent_style=space\nindent_size=4\ntab_width=8\nend_of_line=crlf\ncharset=utf-8\ntrim_trailing_whitespace=true\ninsert_final_newline=true\n[*.rs]\nindent_size=2").unwrap();
        std::fs::write(
            root.join("src/.editorconfig"),
            "[*.rs]\nindent_size=unset\ntab_width=3\nindent_style=TAB\ninsert_final_newline=false",
        )
        .unwrap();
        let (p, warnings) = load(Some(&root.join("src/main.rs")));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            p,
            Properties {
                spaces: Some(false),
                tab_width: Some(3),
                line_ending: Some("\r\n"),
                bom: Some(false),
                trim: Some(true),
                final_newline: Some(false),
                ..Properties::default()
            }
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn editorconfig_invalid_values_and_unsupported_encodings_are_visible() {
        let (_, sections, warnings) = parse("[*]\ncharset=utf-16le\nindent_size=zero\nend_of_line=lf # inline comments are values\ninsert_final_newline=true", Path::new(".editorconfig"));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(sections[0].values, [("insert_final_newline".into(), "true".into())]);
        assert!(warnings.iter().any(|w| w.contains("unsupported charset") && w.contains(":2:")));
    }
}
