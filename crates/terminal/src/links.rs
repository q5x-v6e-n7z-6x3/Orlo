//! Local OSC7 directories and terminal output link detection.

use crate::Screen;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    File { path: PathBuf, line: usize, column: usize },
}

pub fn local_file_url(url: &str) -> Option<PathBuf> {
    decode_local_url(url, false)
}

/// OSC7 may describe real folders with percent-encoded control bytes; links stay strict.
pub(crate) fn directory_url(url: &str) -> Option<PathBuf> {
    decode_local_url(url, true)
}

fn decode_local_url(url: &str, directory: bool) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        let mut hostname = [0u8; 256];
        if unsafe { libc::gethostname(hostname.as_mut_ptr().cast(), hostname.len()) } != 0 {
            return None;
        }
        let end = hostname.iter().position(|&b| b == 0).unwrap_or(hostname.len());
        if !host.as_bytes().eq_ignore_ascii_case(&hostname[..end]) {
            return None;
        }
    }
    let mut bytes = Vec::new();
    let mut input = rest[slash..].bytes();
    while let Some(b) = input.next() {
        let encoded = b == b'%';
        let b = if encoded {
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            hex(input.next()?)? * 16 + hex(input.next()?)?
        } else {
            b
        };
        if b == 0 || (b < 32 || b == 127) && !(directory && encoded) {
            return None;
        }
        bytes.push(b);
    }
    Some(std::ffi::OsString::from_vec(bytes).into())
}

fn parse_link(token: &str, cwd: Option<&Path>) -> Option<Link> {
    let token = token.trim_start_matches(['(', '[', '{']).trim_end_matches([',', ';', '.', '!', '?', ']', '}']);
    if token.chars().any(char::is_control) {
        return None;
    }
    // a quoted URL (JSON's "url": "https://…",) opens without its quotes
    let url = ['"', '\''].into_iter().find_map(|quote| token.strip_prefix(quote)?.strip_suffix(quote)).unwrap_or(token);
    if url.starts_with("http://") || url.starts_with("https://") {
        // a closing parenthesis stays while it closes one the URL opened, as Wikipedia's do
        let mut url = url;
        while url.ends_with(')') && url.matches(')').count() > url.matches('(').count() {
            url = &url[..url.len() - 1];
        }
        return Some(Link::Url(url.to_string()));
    }
    // Diagnostics may quote the complete location or end it with a separator.
    let token = token.strip_suffix(':').unwrap_or(token);
    let token =
        ['"', '\''].into_iter().find_map(|quote| token.strip_prefix(quote)?.strip_suffix(quote)).unwrap_or(token);
    let mut path = token;
    let mut line = 1;
    let mut column = 1;
    let mut has_location = false;
    let number = |s: &str| s.parse::<usize>().ok().filter(|&n| n > 0);
    if let Some((base, location)) = token.strip_suffix(')').and_then(|t| t.rsplit_once('(')) {
        let (row, col) = location.split_once(',').unwrap_or((location, "1"));
        has_location = true;
        line = number(row)?;
        column = number(col)?;
        path = base;
    } else {
        path = path.trim_end_matches(')');
        if let Some((base, suffix)) = path.rsplit_once(':') {
            if let Some(n) = number(suffix) {
                has_location = true;
                path = base;
                line = n;
                if let Some((base, suffix)) = path.rsplit_once(':') {
                    if let Some(n) = number(suffix) {
                        column = line;
                        line = n;
                        path = base;
                    }
                }
            }
        }
    }
    path = path.trim_matches(['\'', '"']);
    let path = if path.starts_with("file://") {
        local_file_url(path)?
    } else {
        // Avoid treating prose or arbitrary URI schemes as files.
        if path.contains(':') || !has_location && !(path.contains('/') || path.contains('.')) {
            return None;
        }
        if let Some(rest) = path.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME")?).join(rest)
        } else {
            if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                cwd?.join(path)
            }
        }
    };
    Some(Link::File { path, line, column })
}

/// Recover the whole soft-wrapped logical line, including portions outside the viewport.
pub fn link_at(screen: &Screen, point: (u16, u16)) -> Option<Link> {
    let mut copy = screen.clone();
    let viewport = copy.scrollback();
    copy.set_scrollback(usize::MAX);
    let retained = copy.scrollback();
    let clicked = retained - viewport + point.0 as usize;
    let (rows, _) = copy.size();
    let incomplete_start = copy.incomplete_start();
    let position_row = |screen: &mut Screen, row: usize| {
        screen.set_scrollback(retained.saturating_sub(row));
        row.saturating_sub(retained) as u16
    };
    let mut first = clicked;
    while first > 0 {
        let row = position_row(&mut copy, first - 1);
        if !copy.row_wrapped(row) {
            break;
        }
        first -= 1;
    }
    let mut text = String::new();
    let mut at = None;
    let mut provenance = Vec::new();
    let mut previous = 0;
    for index in first..retained + rows as usize {
        let row = position_row(&mut copy, index);
        for col in 0..u16::MAX {
            let Some(cell) = copy.cell(row, col) else { break };
            if index == clicked && col == point.1 {
                at = Some(if cell.is_wide_continuation() { previous } else { text.len() });
            }
            if cell.is_wide_continuation() {
                continue;
            }
            let contents = cell.contents();
            // the screen's own shared path: a copy a cell, over a long wrapped line, ran to gigabytes
            provenance.push((text.len(), copy.directory(row, col).cloned()));
            previous = text.len();
            text.push_str(if contents.is_empty() { " " } else { contents });
        }
        if !copy.row_wrapped(row) {
            break;
        }
    }
    let at = at?;
    let mut start = 0;
    let mut quote = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        if c == '"' || c == '\'' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() && (i == start || text[..i].ends_with(['(', '[', '{'])) {
                quote = Some(c);
            }
        }
        if c.is_whitespace() && quote.is_none() {
            if start <= at && at < i {
                let mut paths = provenance
                    .iter()
                    .filter(|(offset, _)| start <= *offset && *offset < i)
                    .map(|(_, path)| path.as_ref());
                let cwd = paths.next().flatten();
                let same = |a: Option<&std::sync::Arc<std::path::PathBuf>>| match (a, cwd) {
                    (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b) || a == b,
                    (a, b) => a.is_none() && b.is_none(),
                };
                let cwd = if paths.all(same) && !(first == 0 && incomplete_start) { cwd } else { None };
                let cwd = cwd.map(|p| p.as_path());
                return parse_link(&text[start..i], cwd);
            }
            start = i + c.len_utf8();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Emulator;
    use std::path::Path;

    #[test]
    #[ignore = "release performance gate"]
    fn bench_resize_with_full_scrollback() {
        let mut e = Emulator::with_size(50, 200, 10_000);
        e.set_directory("/proj".into());
        for _ in 0..10_100 {
            e.process(format!("src/main.rs:3:5 {}\r\n", "x".repeat(180)).as_bytes());
        }
        let start = std::time::Instant::now();
        for cols in 120..180 {
            e.resize(50, cols);
        }
        let elapsed = start.elapsed() / 60;
        eprintln!("resize with 10k rows of 200 columns retained: {elapsed:?}");
        e.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(
            link_at(e.screen(), (0, 1)),
            Some(Link::File { path: "/proj/src/main.rs".into(), line: 3, column: 5 })
        );
        assert!(elapsed < std::time::Duration::from_millis(4));
    }

    #[test]
    fn urls_keep_their_own_parentheses_and_lose_quotes() {
        let wiki = "https://en.wikipedia.org/wiki/Rust_(programming_language)";
        assert_eq!(parse_link(wiki, None), Some(Link::Url(wiki.into())));
        assert_eq!(parse_link(&format!("({wiki})."), None), Some(Link::Url(wiki.into())));
        assert_eq!(parse_link("(https://example.com/x)", None), Some(Link::Url("https://example.com/x".into())));
        assert_eq!(parse_link("\"https://example.com/x\",", None), Some(Link::Url("https://example.com/x".into())));
    }

    #[test]
    fn erasing_progress_and_full_screen_output_keep_retained_relative_links() {
        let mut e = Emulator::with_size(24, 80, 100);
        e.set_directory("/proj".into());
        e.process(b"src/main.rs:3:5\r\n");
        for i in 0..5000 {
            // grep --color's erases, a progress line rewriting itself, then a full-screen program
            e.process(b"\x1b[01;31m\x1b[Kfn\x1b[m\x1b[K main\x1b[K");
            e.process(format!("\r{i}%\x1b[K").as_bytes());
        }
        e.process(b"\r\n\x1b[?1049h");
        for _ in 0..5000 {
            e.process(b"\x1b[1;1Hx\x1b[K\x1b[5;9Hy");
        }
        e.process(b"\x1b[?1049l");
        e.screen_mut().set_scrollback(usize::MAX);
        let link = Some(Link::File { path: "/proj/src/main.rs".into(), line: 3, column: 5 });
        assert_eq!(link_at(e.screen(), (0, 1)), link);
    }

    #[test]
    fn provenance_keeps_more_than_4096_routine_lines_in_default_scrollback() {
        let mut e = Emulator::with_size(3, 20, 10_000);
        e.set_directory("/one".into());
        for _ in 0..5000 {
            e.process(b"a.rs:2:3\r\n");
        }
        e.screen_mut().set_scrollback(usize::MAX);
        let expected = Some(Link::File { path: "/one/a.rs".into(), line: 2, column: 3 });
        assert_eq!(link_at(e.screen(), (0, 1)), expected);
        e.resize(4, 40);
        assert_eq!(link_at(e.screen(), (0, 1)), expected);
        let found =
            crate::search::SearchResults::find(e.screen().clone(), "a.rs", &std::sync::atomic::AtomicBool::new(false))
                .unwrap();
        assert_eq!(found.len(), 5000);
        assert_eq!(link_at(found.screen(), (0, 1)), expected);
    }

    #[test]
    fn command_review_preserves_in_command_directory_changes_and_refuses_evicted_prefixes() {
        let mut e = Emulator::with_size(4, 20, 10);
        e.process(b"\x1b]133;C\x07\x1b]7;file:///one\x07a.rs:2:3\r\n\x1b]7;file:///two\x07a.rs:2:3\x1b]133;D;0\x07");
        for cols in [20, 6, 40] {
            let review = e.commands().records()[0].review_screen(5, cols);
            assert_eq!(link_at(&review, (0, 1)), Some(Link::File { path: "/one/a.rs".into(), line: 2, column: 3 }));
            let row = if cols == 6 { 2 } else { 1 };
            assert_eq!(link_at(&review, (row, 1)), Some(Link::File { path: "/two/a.rs".into(), line: 2, column: 3 }));
        }
        let mut e = Emulator::with_size(2, 8, 0);
        e.process(b"\x1b]7;file:///one\x07\x1b]133;C\x07prefixxxsuffix.rs:2:3\x1b]133;D;0\x07");
        let review = e.commands().records()[0].review_screen(5, 40);
        assert!(review.contents().contains("suffix.rs"));
        assert_eq!(link_at(&review, (0, 1)), None);
    }

    #[test]
    fn provenance_refuses_truncated_tokens_but_keeps_complete_links_on_resize() {
        let mut e = Emulator::with_size(4, 12, 3);
        e.set_directory("/one".into());
        e.process(b"src/long.rs:2:3\r\na.rs:2:3");
        let review = e.screen().clone();
        e.resize(5, 12); // vt100 clears visible wrap flags, even for height-only changes.
        assert_eq!(link_at(e.screen(), (0, 1)), None);
        assert_eq!(link_at(e.screen(), (1, 0)), None);
        assert_eq!(link_at(e.screen(), (2, 0)), Some(Link::File { path: "/one/a.rs".into(), line: 2, column: 3 }));
        assert_eq!(link_at(&review, (0, 1)), Some(Link::File { path: "/one/src/long.rs".into(), line: 2, column: 3 }));
        e.resize(5, 8);
        assert_eq!(link_at(e.screen(), (2, 0)), Some(Link::File { path: "/one/a.rs".into(), line: 2, column: 3 }));
        e.resize(5, 6); // surviving "a.rs:2" must not invent a default column.
        assert_eq!(link_at(e.screen(), (2, 0)), None);
        e.process(b"\r\nb.rs");
        assert_eq!(link_at(e.screen(), (3, 0)), Some(Link::File { path: "/one/b.rs".into(), line: 1, column: 1 }));
        let mut e = Emulator::with_size(2, 8, 0);
        e.set_directory("/one".into());
        e.process(b"prefixxxsuffix.rs:2:3");
        e.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(link_at(e.screen(), (0, 0)), None, "evicted prefix must not become a different relative file");
        let mut e = Emulator::with_size(2, 30, 10);
        e.process("\x1b]7;file:///one\x07cafe.rs\x1b]7;file:///two\x07\u{301}".as_bytes());
        assert_eq!(link_at(e.screen(), (0, 1)), None, "a combining continuation from another cwd has mixed provenance");
    }

    #[test]
    fn provenance_historical_directories_survive_search_review_wrap_resize_and_eviction() {
        use std::sync::atomic::AtomicBool;
        let root = std::env::temp_dir().join(format!("orlo-link-provenance-{}", std::process::id()));
        let one = root.join("café 中");
        let two = root.join("第二 😀");
        for (path, contents) in [(&one, "first"), (&two, "second")] {
            std::fs::create_dir_all(path.join("src")).unwrap();
            std::fs::write(path.join("src/a.rs"), contents).unwrap();
        }
        let mut e = Emulator::with_size(3, 10, 12);
        for directory in [&one, &two] {
            let output =
                format!("\x1b]7;file://{}\x07\x1b]133;C\x07src/a.rs:2:3\r\n\x1b]133;D;0\x07", directory.display());
            for byte in output.bytes() {
                e.process(&[byte]);
            }
        }
        let mut results =
            crate::search::SearchResults::find(e.screen().clone(), "src/a.rs", &AtomicBool::new(false)).unwrap();
        assert_eq!(results.len(), 2);
        for (index, directory) in [&one, &two].into_iter().enumerate() {
            results.select(index);
            let screen = results.screen();
            let row = (0..3).find(|&row| results.highlights(row, 0)).unwrap();
            assert_eq!(
                link_at(screen, (row, 1)),
                Some(Link::File { path: directory.join("src/a.rs"), line: 2, column: 3 })
            );
            let review = e.commands().records()[index].review_screen(3, 8);
            assert_eq!(
                link_at(&review, (0, 1)),
                Some(Link::File { path: directory.join("src/a.rs"), line: 2, column: 3 })
            );
            assert_eq!(
                std::fs::read_to_string(directory.join("src/a.rs")).unwrap(),
                if index == 0 { "first" } else { "second" }
            );
        }
        e.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(link_at(e.screen(), (0, 1)), Some(Link::File { path: one.join("src/a.rs"), line: 2, column: 3 }));
        let mut snapshot = e.screen().clone();
        e.process(b"\r\nnew\r\nnew\r\n");
        assert_eq!(link_at(&snapshot, (0, 1)), Some(Link::File { path: one.join("src/a.rs"), line: 2, column: 3 }));
        snapshot.set_size(4, 10);
        assert_eq!(link_at(&snapshot, (0, 1)), Some(Link::File { path: one.join("src/a.rs"), line: 2, column: 3 }));
        for _ in 0..30 {
            e.process(b"evict\r\n");
        }
        e.resize(4, 20);
        e.process(b"src/a.rs:2:3");
        e.screen_mut().set_scrollback(0);
        let row = e.screen().cursor_position().0;
        assert_eq!(link_at(e.screen(), (row, 1)), Some(Link::File { path: two.join("src/a.rs"), line: 2, column: 3 }));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provenance_fails_closed_for_mixed_unknown_and_expired_ids_and_tracks_native_controls() {
        let mut e = Emulator::with_size(4, 40, 4);
        e.process(b"unknown.rs https://example.com /absolute.rs");
        assert_eq!(link_at(e.screen(), (0, 1)), None);
        assert!(matches!(link_at(e.screen(), (0, 15)), Some(Link::Url(_))));
        e.process(b"\x1bc\x1b]7;file:///one\x07\x1b[31msrc/\x1b]7;file:///two\x07a.rs");
        assert_eq!(link_at(e.screen(), (0, 1)), None);
        e.process(b"\x1bc\x1b]7;file:///one\x07src/a.rs\x1b7\x1b[?1049h");
        e.process(b"\x1b]7;file:///two\x07src/a.rs");
        assert_eq!(link_at(e.screen(), (0, 1)), Some(Link::File { path: "/two/src/a.rs".into(), line: 1, column: 1 }));
        e.process(b"\x1b[?1049l\x1b8\r\n\x1b[0msrc/a.rs");
        assert_eq!(link_at(e.screen(), (0, 1)), Some(Link::File { path: "/one/src/a.rs".into(), line: 1, column: 1 }));
        assert_eq!(link_at(e.screen(), (1, 1)), Some(Link::File { path: "/two/src/a.rs".into(), line: 1, column: 1 }));
        for n in 0..5000 {
            e.set_directory(format!("/directory/{n}").into());
        }
        assert_eq!(link_at(e.screen(), (0, 1)), None);
        e.process(b"\r\nnew.rs");
        assert_eq!(
            link_at(e.screen(), (2, 1)),
            Some(Link::File { path: "/directory/4999/new.rs".into(), line: 1, column: 1 })
        );
    }

    #[test]
    fn n39_directory_reports_accept_encoded_newlines_but_links_and_invalid_urls_stay_strict() {
        assert_eq!(directory_url("file:///tmp/one%0Atwo"), Some("/tmp/one\ntwo".into()));
        assert_eq!(local_file_url("file:///tmp/one%0Atwo"), None);
        for bad in [
            "file://remote.invalid/tmp/a",
            "file:///tmp/%00",
            "file:///tmp/%GG",
            "file:///tmp/%0",
            "file:relative",
            "file:///tmp/raw\nline",
        ] {
            assert_eq!(directory_url(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn osc7_decodes_local_paths_and_refuses_remote_or_malformed_input() {
        assert_eq!(local_file_url("file://localhost/tmp/caf%C3%A9%20code"), Some("/tmp/café code".into()));
        for bad in [
            "file://remote.invalid/tmp/code",
            "file:///tmp/%GG",
            "file:///tmp/%00",
            "file:relative",
            "https://example.com",
        ] {
            assert_eq!(local_file_url(bad), None, "{bad}");
        }
        let mut emulator = Emulator::with_size(3, 20, 10);
        for b in b"\x1b]7;file:///tmp/one\x1b\\" {
            emulator.process(&[*b]);
        }
        assert_eq!(emulator.working_directory(), Some(Path::new("/tmp/one")));
        emulator.process(b"\x1b]7;file:///tmp/two\x07\x1b]7;file://remote.invalid/x\x07");
        assert_eq!(emulator.working_directory(), Some(Path::new("/tmp/two")));
    }

    #[test]
    fn finds_paths_locations_quoted_spaces_urls_and_soft_wrapped_wide_cells() {
        let cwd = Path::new("/tmp/project");
        for (token, expected) in [
            ("src/main.rs:12:3", Link::File { path: cwd.join("src/main.rs"), line: 12, column: 3 }),
            ("\"src/my file.rs\":2", Link::File { path: cwd.join("src/my file.rs"), line: 2, column: 1 }),
            ("main.rs(4,2)", Link::File { path: cwd.join("main.rs"), line: 4, column: 2 }),
            ("https://example.com/a?q=one", Link::Url("https://example.com/a?q=one".into())),
        ] {
            assert_eq!(parse_link(token, Some(cwd)), Some(expected));
        }
        assert_eq!(parse_link("javascript:alert(1)", Some(cwd)), None);
        let mut emulator = Emulator::with_size(4, 16, 20);
        emulator.set_directory(cwd.into());
        emulator.process("中 src/main.rs:12:3\r\nhttps://example.com/a".as_bytes());
        assert_eq!(
            link_at(emulator.screen(), (0, 4)),
            Some(Link::File { path: cwd.join("src/main.rs"), line: 12, column: 3 })
        );
        assert_eq!(link_at(emulator.screen(), (3, 0)), Some(Link::Url("https://example.com/a".into())));
    }

    #[test]
    fn compiler_trailing_separator_preserves_one_based_location() {
        assert_eq!(
            parse_link("src/file.rs:12:3:", Some(Path::new("/project"))),
            Some(Link::File { path: "/project/src/file.rs".into(), line: 12, column: 3 })
        );
    }

    #[test]
    fn extensionless_files_require_an_explicit_location_or_path_context() {
        for token in ["Makefile:12:3", "Makefile:12:3:", "Makefile(12,3)"] {
            assert_eq!(
                parse_link(token, Some(Path::new("/project"))),
                Some(Link::File { path: "/project/Makefile".into(), line: 12, column: 3 }),
                "{token}"
            );
        }
        for token in ["Makefile", "prose", "Makefile:0", "Makefile:12:0"] {
            assert_eq!(parse_link(token, Some(Path::new("/project"))), None, "{token}");
        }
        for token in ["./Makefile", "file:///project/Makefile"] {
            assert_eq!(
                parse_link(token, Some(Path::new("/project"))),
                Some(Link::File {
                    path: if token.starts_with("./") {
                        "/project/./Makefile".into()
                    } else {
                        "/project/Makefile".into()
                    },
                    line: 1,
                    column: 1
                })
            );
        }
    }

    #[test]
    fn fully_quoted_locations_keep_unicode_spaces_and_output_directory_across_wraps() {
        for token in ["\"my 中 file.rs:2:3\"", "'my 中 file.rs:2:3'", "\"my 中 file.rs\":2:3"] {
            let mut e = Emulator::with_size(4, 9, 20);
            e.process(b"\x1b]7;file:///old\x07");
            e.process(token.as_bytes());
            e.process(b"\r\n\x1b]7;file:///new\x07");
            assert_eq!(
                link_at(e.screen(), (1, 1)),
                Some(Link::File { path: "/old/my 中 file.rs".into(), line: 2, column: 3 }),
                "{token}"
            );
        }
    }

    #[test]
    fn wrapped_location_recovers_retained_prefix_outside_the_viewport() {
        let mut e = Emulator::with_size(2, 9, 10);
        e.process("\x1b]7;file:///old\x07\"long 中 file.rs:2:3\"".as_bytes());
        assert_eq!(e.screen().scrollback(), 0);
        assert_ne!(e.screen().cell(0, 0).unwrap().contents(), "\"", "the prefix is above the visible grid");
        let snapshot = e.screen().clone();
        e.process(b"\x1b]7;file:///new\x07\r\nnew.rs");
        assert_eq!(
            link_at(&snapshot, (0, 1)),
            Some(Link::File { path: "/old/long 中 file.rs".into(), line: 2, column: 3 })
        );
        assert_eq!(
            parse_link("file:///tmp/caf%C3%A9%20code.rs:7:9", None),
            Some(Link::File { path: "/tmp/café code.rs".into(), line: 7, column: 9 })
        );
    }

    #[test]
    fn malformed_directory_updates_do_not_relabel_before_or_after_output() {
        let mut e = Emulator::with_size(12, 40, 10);
        e.process(b"\x1b]7;file:///old\x07old.rs:2:3\r\n");
        for bad in [
            "file://remote.invalid/tmp/new",
            "file:relative",
            "file:///tmp/%",
            "file:///tmp/%GG",
            "file:///tmp/%00",
            "file:///tmp/%0a",
            "https://example.com/tmp",
        ] {
            for byte in format!("\x1b]7;{bad}\x1b\\new.rs:4:5\r\n").bytes() {
                e.process(&[byte]);
            }
        }
        assert_eq!(e.working_directory(), Some(Path::new("/old")));
        assert_eq!(link_at(e.screen(), (0, 1)), Some(Link::File { path: "/old/old.rs".into(), line: 2, column: 3 }));
        for row in 1..=7 {
            assert_eq!(
                link_at(e.screen(), (row, 1)),
                Some(Link::File { path: "/old/new.rs".into(), line: 4, column: 5 })
            );
        }
        assert!(e.take_replies().is_empty());
        for token in ["a.rs:0", "a.rs:1:0", "a.rs:184467440737095516160", "javascript:alert(1)", "word"] {
            assert_eq!(parse_link(token, Some(Path::new("/old"))), None, "{token}");
        }
    }

    #[test]
    fn both_halves_of_wide_paths_and_links_after_prose_apostrophes_are_clickable() {
        let cwd = Path::new("/tmp");
        let mut e = Emulator::with_size(3, 80, 10);
        e.set_directory(cwd.into());
        e.process("/tmp/中\r\ncan't open src/main.rs:12\r\n\"my file.rs\":3".as_bytes());
        for col in [5, 6] {
            assert_eq!(link_at(e.screen(), (0, col)), Some(Link::File { path: "/tmp/中".into(), line: 1, column: 1 }));
        }
        assert_eq!(
            link_at(e.screen(), (1, 12)),
            Some(Link::File { path: cwd.join("src/main.rs"), line: 12, column: 1 })
        );
        assert_eq!(link_at(e.screen(), (2, 5)), Some(Link::File { path: cwd.join("my file.rs"), line: 3, column: 1 }));
    }
}
