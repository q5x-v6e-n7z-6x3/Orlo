//! Command-line input, parsed before application support or the native UI is touched.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub const MAX_STDIN: usize = 1024 * 1024;
pub const MAX_PATH_BYTES: usize = 1024 * 1024;
pub const MAX_TARGETS: usize = 4096;
pub const HELP: &str = "Usage: Orlo [--wait] [--] [file[:line[:column]] ...] [-]\n\
    \x20      Orlo --check-language folder ...\n\
    \n\
    --help       Show this help without starting Orlo\n\
    --version    Print the version without starting Orlo\n\
    --wait       Wait until the requested buffers' final views close\n\
    -            Read UTF-8 stdin into an unsaved buffer (limit: 1 MiB)\n\
    --           Treat the remaining arguments as literal paths\n\
    --check-language  Load and activate language folders without starting Orlo;\n\
    \x20                 exits non-zero if any has an error\n\
    Locations are one-based and clamped. Existing colon-containing names win.\n\
    A missing file opens as an empty named buffer; saving creates it.\n";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Path {
        #[serde(with = "path_bytes")]
        path: PathBuf,
        position: Option<Position>,
    },
    Stdin(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub targets: Vec<Target>,
    pub wait: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    Help,
    Version,
    /// Language folders to load and activate, then exit.
    CheckLanguage(Vec<PathBuf>),
    Open(Request),
}

pub fn parse(args: impl IntoIterator<Item = OsString>, cwd: &Path, stdin: &mut impl Read) -> Result<Launch, String> {
    let args: Vec<_> = args.into_iter().take(MAX_TARGETS + 9).collect();
    for arg in &args {
        match arg.as_bytes() {
            b"--" => break,
            b"--help" | b"-h" => return Ok(Launch::Help),
            b"--version" | b"-v" => return Ok(Launch::Version),
            _ => {}
        }
    }
    if args.len() > MAX_TARGETS + 8 {
        return Err("too many command-line arguments".into());
    }
    if args.first().is_some_and(|a| a == "--check-language") {
        if args.len() == 1 {
            return Err("--check-language needs at least one language folder".into());
        }
        return Ok(Launch::CheckLanguage(args[1..].iter().map(|a| cwd.join(a)).collect()));
    }
    let mut request = Request::default();
    let (mut literal, mut input, mut bytes) = (false, false, 0);
    for arg in args {
        match arg.as_bytes() {
            b"--" if !literal => {
                literal = true;
                continue;
            }
            b"--check-language" if !literal => return Err("--check-language must be the first argument".into()),
            b"--wait" | b"-w" if !literal => {
                request.wait = true;
                continue;
            }
            b"-" if !literal => {
                if input {
                    return Err("stdin may only be requested once".into());
                }
                input = true;
                let mut text = Vec::new();
                (&mut *stdin)
                    .take((MAX_STDIN + 1) as u64)
                    .read_to_end(&mut text)
                    .map_err(|e| format!("could not read stdin: {e}"))?;
                if text.len() > MAX_STDIN {
                    return Err("stdin exceeds the 1 MiB limit".into());
                }
                request.targets.push(Target::Stdin(
                    String::from_utf8(text).map_err(|_| "stdin must contain valid UTF-8".to_string())?,
                ));
                continue;
            }
            a if !literal && a.starts_with(b"-psn_") => continue,
            a if !literal && a.starts_with(b"-") => {
                return Err(format!("unknown option {}; use -- before a literal path", arg.to_string_lossy()))
            }
            _ => {}
        }
        bytes += arg.as_bytes().len();
        if bytes > MAX_PATH_BYTES || request.targets.len() >= MAX_TARGETS {
            return Err("file arguments exceed the command-line limit".into());
        }
        // an absolute path replaces `cwd`
        let raw = cwd.join(arg);
        let (path, position) =
            if literal || std::fs::symlink_metadata(&raw).is_ok() { (raw, None) } else { location(raw)? };
        request.targets.push(Target::Path { path, position });
    }
    request.validate()?;
    Ok(Launch::Open(request))
}

fn location(path: PathBuf) -> Result<(PathBuf, Option<Position>), String> {
    let bytes = path.as_os_str().as_bytes();
    let Some(last) = bytes.iter().rposition(|&b| b == b':') else {
        return Ok((path, None));
    };
    let number = |s: &[u8]| -> Result<Option<usize>, String> {
        if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
            return Ok(None);
        }
        let n =
            std::str::from_utf8(s).unwrap().parse::<usize>().map_err(|_| "line or column is too large".to_string())?;
        if n == 0 {
            return Err("line and column must be at least 1".into());
        }
        Ok(Some(n))
    };
    let Some(n) = number(&bytes[last + 1..])? else {
        return Ok((path, None));
    };
    let (end, line, column) = match bytes[..last].iter().rposition(|&b| b == b':') {
        Some(previous) => match number(&bytes[previous + 1..last])? {
            Some(line) => (previous, line, n),
            None => (last, n, 1),
        },
        None => (last, n, 1),
    };
    let path = PathBuf::from(OsString::from_vec(bytes[..end].to_vec()));
    Ok((path, Some(Position { line, column })))
}

impl Request {
    /// Both the local parser and the socket are trust boundaries.
    pub fn validate(&self) -> Result<(), String> {
        if self.targets.len() > MAX_TARGETS {
            return Err("too many requested files".into());
        }
        let (mut paths, mut inputs) = (0, 0);
        for target in &self.targets {
            match target {
                Target::Path { path, position } => {
                    let bytes = path.as_os_str().as_bytes();
                    paths += bytes.len();
                    if !path.is_absolute() || bytes.contains(&0) || paths > MAX_PATH_BYTES {
                        return Err("invalid or oversized file path".into());
                    }
                    if position.as_ref().is_some_and(|p| p.line == 0 || p.column == 0) {
                        return Err("line and column must be at least 1".into());
                    }
                    match std::fs::metadata(path) {
                        Ok(meta) if !meta.is_file() && !meta.is_dir() => {
                            return Err(format!("{} is not a regular file or folder", path.display()))
                        }
                        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                            return Err(format!("{}: {e}", path.display()))
                        }
                        _ => {}
                    }
                }
                Target::Stdin(text) => {
                    inputs += 1;
                    if inputs > 1 || text.len() > MAX_STDIN {
                        return Err("stdin exceeds the 1 MiB limit or appears more than once".into());
                    }
                }
            }
        }
        Ok(())
    }
}

mod path_bytes {
    use super::*;
    pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().as_bytes().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(|b| PathBuf::from(OsString::from_vec(b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    fn parse_args(args: &[&str]) -> Result<Launch, String> {
        parse_input(args, b"")
    }

    fn parse_input(args: &[&str], mut stdin: &[u8]) -> Result<Launch, String> {
        parse(args.iter().map(OsString::from), Path::new("/tmp"), &mut stdin)
    }

    fn path(path: &str, position: Option<Position>) -> Target {
        Target::Path { path: PathBuf::from(path), position }
    }

    #[test]
    fn help_and_version_do_not_read_stdin() {
        struct NoRead;
        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("help and version must exit before reading stdin")
            }
        }
        assert_eq!(parse(["-", "--help"].map(OsString::from), Path::new("/tmp"), &mut NoRead), Ok(Launch::Help));
        assert_eq!(parse(["--version"].map(OsString::from), Path::new("/tmp"), &mut NoRead), Ok(Launch::Version));
    }

    #[test]
    fn check_language_takes_every_later_argument_as_a_folder() {
        assert_eq!(
            parse_args(&["--check-language", "pony", "/abs/nim", "--wait", "-"]),
            Ok(Launch::CheckLanguage(["/tmp/pony", "/abs/nim", "/tmp/--wait", "/tmp/-"].map(PathBuf::from).to_vec()))
        );
        assert_eq!(parse_args(&["--check-language", "--help"]), Ok(Launch::Help));
        assert!(parse_args(&["--check-language"]).unwrap_err().contains("folder"));
        assert!(parse_args(&["a.rs", "--check-language", "pony"]).unwrap_err().contains("first"));
        assert_eq!(
            parse_args(&["--", "--check-language"]),
            Ok(Launch::Open(Request { targets: vec![path("/tmp/--check-language", None)], wait: false }))
        );
    }

    #[test]
    fn wait_and_one_based_locations_preserve_request_order() {
        assert_eq!(
            parse_args(&["--wait", "a.rs:12:4", "b.rs:8"]),
            Ok(Launch::Open(Request {
                wait: true,
                targets: vec![
                    path("/tmp/a.rs", Some(Position { line: 12, column: 4 })),
                    path("/tmp/b.rs", Some(Position { line: 8, column: 1 }))
                ],
            }))
        );
    }

    #[test]
    fn double_dash_makes_options_stdin_and_colons_literal_paths() {
        assert_eq!(
            parse_args(&["--", "--wait", "-", "draft:3"]),
            Ok(Launch::Open(Request {
                wait: false,
                targets: vec![path("/tmp/--wait", None), path("/tmp/-", None), path("/tmp/draft:3", None)],
            }))
        );
    }

    #[test]
    fn explicit_stdin_is_utf8_and_unsaved_input() {
        assert_eq!(
            parse_input(&["-"], "café\n".as_bytes()),
            Ok(Launch::Open(Request { targets: vec![Target::Stdin("café\n".into())], wait: false }))
        );
    }

    #[test]
    fn stdin_rejects_malformed_unicode_and_oversize_input() {
        assert!(parse_input(&["-"], b"\xff").unwrap_err().contains("UTF-8"));
        let bytes = vec![b'x'; MAX_STDIN + 1];
        assert!(parse_input(&["-"], &bytes).unwrap_err().contains("limit"));
    }

    #[test]
    fn invalid_options_and_zero_locations_are_useful_errors() {
        for args in [vec!["--bogus"], vec!["file:0"], vec!["file:1:0"], vec!["-", "-"]] {
            assert!(parse_args(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn existing_colon_names_take_precedence_over_locations() {
        let dir = std::env::temp_dir().join(format!("orlo-cli-colon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes:12:4");
        std::fs::write(&file, "literal").unwrap();
        assert_eq!(
            parse([file.clone().into_os_string()], &dir, &mut &b""[..]),
            Ok(Launch::Open(Request { targets: vec![Target::Path { path: file, position: None }], wait: false }))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn non_utf8_paths_keep_their_bytes_when_parsing_locations() {
        let arg = OsString::from_vec(b"draft-\xff:3:2".to_vec());
        let Launch::Open(request) = parse([arg], Path::new("/tmp"), &mut &b""[..]).unwrap() else {
            panic!("open request")
        };
        let Target::Path { path, position } = &request.targets[0] else { panic!("file target") };
        assert_eq!(path.as_os_str().as_bytes(), b"/tmp/draft-\xff");
        assert_eq!(*position, Some(Position { line: 3, column: 2 }));
    }
}
