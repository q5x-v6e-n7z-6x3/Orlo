//! Deliberately data-only Vim/Emacs/Kate subset. Unknown tokens are never evaluated.
use super::WordWrap;

pub const EDGE_LINES: usize = 5;
pub const MAX_LINE_BYTES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub tab_width: Option<usize>,
    pub indent_size: Option<usize>,
    pub spaces: Option<bool>,
    pub wrap: Option<bool>,
    /// Sets both the wrap width and a single ruler; zero removes the ruler.
    pub margin: Option<usize>,
    /// Syntactically safe candidates in source order; the registry skips unknown names.
    pub languages: Vec<String>,
}

impl Properties {
    pub fn wrapping(&self, fallback: WordWrap) -> WordWrap {
        self.wrap.map_or(fallback, |on| if on { WordWrap::On } else { WordWrap::Off })
    }
}

/// Visit each edge line once, in source order. The reader may reject oversized lines before
/// allocating; this parser also checks the byte limit. No whole-document string is needed.
pub fn scan(line_count: usize, mut line: impl FnMut(usize) -> Option<String>) -> Properties {
    let mut out = Properties::default();
    for i in (0..line_count.min(EDGE_LINES)).chain(line_count.saturating_sub(EDGE_LINES).max(EDGE_LINES)..line_count) {
        let Some(text) = line(i).filter(|s| s.len() <= MAX_LINE_BYTES) else { continue };
        parse_line(&text, &mut out);
    }
    out
}

fn number(value: &str, min: usize, max: usize) -> Option<usize> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<usize>().ok().filter(|n| (min..=max).contains(n))
}

fn language(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b" _+.#()-".contains(&b)))
    .then(|| value.to_string())
}

fn parse_line(text: &str, out: &mut Properties) {
    if let Some((_, rest)) = text.split_once("-*-") {
        if let Some((body, _)) = rest.split_once("-*-") {
            if !body.contains([':', ';']) {
                if let Some(name) = language(body.trim_ascii()) {
                    out.languages.push(name);
                }
            }
            for field in body.split(';') {
                let Some((key, value)) = field.split_once(':') else { continue };
                let value = value.trim_ascii();
                match key.trim_ascii() {
                    "mode" => {
                        if let Some(name) = language(value) {
                            out.languages.push(name);
                        }
                    }
                    "tab-width" => {
                        if let Some(n) = number(value, 1, 16) {
                            out.tab_width = Some(n);
                        }
                    }
                    "standard-indent" => {
                        if let Some(n) = number(value, 1, 16) {
                            out.indent_size = Some(n);
                        }
                    }
                    "indent-tabs-mode" => match value {
                        "t" => out.spaces = Some(false),
                        "nil" => out.spaces = Some(true),
                        _ => {}
                    },
                    "truncate-lines" => match value {
                        "t" => out.wrap = Some(false),
                        "nil" => out.wrap = Some(true),
                        _ => {}
                    },
                    "fill-column" => {
                        if let Some(n) = number(value, 0, super::MAX_WRAP_WIDTH) {
                            out.margin = Some(n);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for (start, _) in text.char_indices() {
        if start != 0 && !text.as_bytes()[start - 1].is_ascii_whitespace() {
            continue;
        }
        let Some(rest) = ["vim:", "vi:", "ex:"].iter().find_map(|m| text[start..].strip_prefix(m)) else { continue };
        let rest = rest.trim_ascii_start();
        let Some((command, rest)) = rest.split_once(char::is_whitespace) else { continue };
        if command != "set" && command != "setlocal" {
            continue;
        }
        let Some((body, _)) = rest.split_once(':') else { continue };
        for token in body.split_ascii_whitespace() {
            let (key, value) = token.split_once('=').unwrap_or((token, ""));
            match (key, value) {
                ("ts" | "tabstop", v) => {
                    if let Some(n) = number(v, 1, 16) {
                        out.tab_width = Some(n);
                    }
                }
                ("sw" | "shiftwidth", v) => {
                    if let Some(n) = number(v, 1, 16) {
                        out.indent_size = Some(n);
                    }
                }
                ("et" | "expandtab", "") if !token.contains('=') => out.spaces = Some(true),
                ("noet" | "noexpandtab", "") if !token.contains('=') => out.spaces = Some(false),
                ("wrap", "") if !token.contains('=') => out.wrap = Some(true),
                ("nowrap", "") if !token.contains('=') => out.wrap = Some(false),
                ("tw" | "textwidth", v) => {
                    if let Some(n) = number(v, 0, super::MAX_WRAP_WIDTH) {
                        out.margin = Some(n);
                    }
                }
                ("ft" | "filetype" | "syntax", v) => {
                    if let Some(name) = language(v) {
                        out.languages.push(name);
                    }
                }
                _ => {}
            }
        }
        break;
    }
    if let Some((start, _)) =
        text.match_indices("kate:").find(|(start, _)| *start == 0 || text.as_bytes()[start - 1].is_ascii_whitespace())
    {
        for field in text[start + 5..].split(';') {
            let Some((key, value)) = field.trim_ascii().split_once(char::is_whitespace) else { continue };
            let value = value.trim_ascii();
            match key {
                "tab-width" => {
                    if let Some(n) = number(value, 1, 16) {
                        out.tab_width = Some(n);
                    }
                }
                "indent-width" => {
                    if let Some(n) = number(value, 1, 16) {
                        out.indent_size = Some(n);
                    }
                }
                "replace-tabs" => match value {
                    "on" => out.spaces = Some(true),
                    "off" => out.spaces = Some(false),
                    _ => {}
                },
                "word-wrap" => match value {
                    "on" => out.wrap = Some(true),
                    "off" => out.wrap = Some(false),
                    _ => {}
                },
                "word-wrap-column" => {
                    if let Some(n) = number(value, 0, super::MAX_WRAP_WIDTH) {
                        out.margin = Some(n);
                    }
                }
                "syntax" => {
                    if let Some(name) = language(value) {
                        out.languages.push(name);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(text: &str) -> Properties {
        let lines: Vec<_> = text.lines().collect();
        scan(lines.len(), |i| Some(lines[i].into()))
    }
    #[test]
    fn common_vim_and_emacs_fields_are_independent_and_later_lines_win() {
        assert_eq!(
            read("// vim: set ts=8 sw=2 noet wrap tw=72:"),
            Properties {
                tab_width: Some(8),
                indent_size: Some(2),
                spaces: Some(false),
                wrap: Some(true),
                margin: Some(72),
                ..Default::default()
            }
        );
        assert_eq!(read("# -*- tab-width: 3; standard-indent: 4; indent-tabs-mode: nil; truncate-lines: t; fill-column: 90; -*-"), Properties { tab_width: Some(3), indent_size: Some(4), spaces: Some(true), wrap: Some(false), margin: Some(90), ..Default::default() });
        assert_eq!(
            read(
                "vi: setlocal tabstop=16 shiftwidth=1 expandtab nowrap textwidth=1000:\nex: set noexpandtab tw=0 ts=2:"
            ),
            Properties {
                tab_width: Some(2),
                indent_size: Some(1),
                spaces: Some(false),
                wrap: Some(false),
                margin: Some(0),
                ..Default::default()
            }
        );
    }
    #[test]
    fn malicious_unknown_malformed_and_unicode_values_stay_data() {
        let p = read("猫 👩‍👧 vim: set ts=3 sw=２ tw=1001 ts=-1 ts=0 sw=17 et=true nowrap=true fenc=utf-16 ft=sh shell=sh foldexpr=execute('evil') | !touch /tmp/evil:\n# -*- eval: (shell-command \"evil\"); mode: shell; coding: utf-16; tab-width: +4; fill-column: 99999999999999999999999; indent-tabs-mode: yes; -*-");
        assert_eq!(
            p,
            Properties { tab_width: Some(3), languages: vec!["sh".into(), "shell".into()], ..Default::default() }
        );
        for text in [
            "//vim: set ts=2:",
            "vim: ts=2:",
            "vim: set ts=2",
            "-*- tab-width: 2",
            "vim: set ts=2x:",
            "vim: set ts=2|evil:",
            "vim: set et= nowrap=:",
        ] {
            assert_eq!(read(text), Properties::default(), "{text}");
        }
        assert_eq!(read("é vim: set ts=2: 🦀").tab_width, Some(2));
    }
    #[test]
    fn edge_scan_is_bounded_skips_middle_and_does_not_duplicate_short_files() {
        let mut visited = Vec::new();
        let p = scan(100_000, |i| {
            visited.push(i);
            Some(if i == 99_999 { "vim: set ts=8:" } else { "" }.into())
        });
        assert_eq!(visited, [0, 1, 2, 3, 4, 99_995, 99_996, 99_997, 99_998, 99_999]);
        assert_eq!(p.tab_width, Some(8));
        for n in 0..=10 {
            let mut seen = Vec::new();
            scan(n, |i| {
                seen.push(i);
                None
            });
            assert_eq!(seen, (0..n).collect::<Vec<_>>());
        }
        let marker = " vim: set ts=2:";
        let unicode = "🦀".repeat(1000);
        let exact = format!("{}{}{}", unicode, " ".repeat(MAX_LINE_BYTES - unicode.len() - marker.len()), marker);
        assert_eq!(exact.len(), MAX_LINE_BYTES);
        assert_eq!(read(&exact).tab_width, Some(2));
        assert_eq!(read(&format!(" {exact}")), Properties::default());
        assert_eq!(read(&format!("{}vim: set ts=2:", "x".repeat(MAX_LINE_BYTES))), Properties::default());
        assert_eq!(read("\n\n\n\n\nvim: set ts=2:\n\n\n\n\n\n"), Properties::default());
    }

    #[test]
    fn expanded_kate_fields_are_bounded_independent_and_declarative() {
        let p = read("// kate: tab-width 8; indent-width 3; replace-tabs off; word-wrap on; word-wrap-column 72;");
        assert_eq!(p.tab_width, Some(8));
        assert_eq!(p.indent_size, Some(3));
        assert_eq!(p.spaces, Some(false));
        assert_eq!(p.wrap, Some(true));
        assert_eq!(p.margin, Some(72));
        let p = read("kate: tab-width 2; tab-width 0; indent-width 17; replace-tabs true; word-wrap off; word-wrap-column 1001; command touch /tmp/evil;");
        assert_eq!(p, Properties { tab_width: Some(2), wrap: Some(false), ..Default::default() });
    }

    #[test]
    fn expanded_language_forms_validate_values_without_interpreting_contents() {
        for (text, name) in [
            ("vim: set ft=rust:", "rust"),
            ("vi: setlocal filetype=python:", "python"),
            ("ex: set syntax=c++:", "c++"),
            ("-*- mode: csharp; -*-", "csharp"),
            ("-*- rust-mode -*-", "rust-mode"),
            ("kate: syntax TypeScript (TSX);", "TypeScript (TSX)"),
        ] {
            assert_eq!(read(text).languages.last().map(String::as_str), Some(name), "{text}");
        }
        assert_eq!(
            read("-*- mode: python; -*- vim: set ft=rust: kate: syntax C++;").languages.last().map(String::as_str),
            Some("C++")
        );
        for value in
            ["", "../../bin/sh", "execute('evil')", "$(touch /tmp/evil)", "'rust'", "py\\thon", "py|sh", "日本語"]
        {
            for text in
                [format!("vim: set ft={value}:"), format!("-*- mode: {value}; -*-"), format!("kate: syntax {value};")]
            {
                assert_eq!(read(&text).languages, Vec::<String>::new(), "{text}");
            }
        }
        assert_eq!(read(&format!("kate: syntax {};", "x".repeat(64))).languages, vec!["x".repeat(64)]);
        assert_eq!(read(&format!("kate: syntax {};", "x".repeat(65))).languages, Vec::<String>::new());
        for text in ["//kate: tab-width 2;", "kate: tab-width ２;", "Kate: tab-width 2;", "kate: word-wrap true;"] {
            assert_eq!(read(text), Properties::default(), "{text}");
        }
    }
}
