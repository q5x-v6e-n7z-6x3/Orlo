//! Saved transformation pipelines. A `.orlo-pipeline` file (JSON with comments) lists steps;
//! `Pipeline::run` applies them to a text, and `run_batch` to files.

use crate::buffer::{self as text, Case, Permute};
use crate::editing::convert::{self, Codec};
use crate::editing::transform::{self, DupOpts, MatchAction, MatchOpts, NormForm, SortOpts, Table, TableOpts};
use crate::search::files::{self, Scope};
use orlo_config::settings::{line_of, strip_json};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;

/// The pipelines' file extension.
pub const EXT: &str = "orlo-pipeline";

/// A NUL in a file's first bytes this many makes it binary.
const BINARY_PROBE: usize = 8 << 10;

/// Problems an alert lists before "and N more".
const SHOWN_PROBLEMS: usize = 12;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BEFORE_PUBLISH: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

/// `Application Support/Orlo/Pipelines`
pub fn dir() -> PathBuf {
    orlo_utils::paths::support_dir().join("Pipelines")
}

pub fn is_path(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == EXT)
}

/// The saved pipelines, by name.
pub fn list() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        (std::fs::read_dir(dir()).into_iter().flatten().flatten()).map(|e| e.path()).filter(|p| is_path(p)).collect();
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    files
}

/// The file name (without the `.ext`) for what New Pipeline, or New Snippet, was given; `what`
/// names the file in the message.
pub fn file_name(input: &str, ext: &str, what: &str) -> Result<String, String> {
    let name = input.trim();
    let name = name.strip_suffix(&format!(".{ext}")).unwrap_or(name);
    if name.is_empty() {
        return Err(format!("Enter a name for the {what}"));
    }
    if name.starts_with('.') || name.contains(['/', ':', '\0']) {
        return Err("A name cannot start with a dot or contain / or :".into());
    }
    Ok(name.to_string())
}

/// What New Pipeline opens: one working step, every operation in comments.
pub fn template() -> String {
    r##"// A pipeline runs its steps in order on the document, on each selection, or on files:
// Tools > Text Pipelines. Saving this file checks it. Comments and trailing commas are fine.
// A step that fails (bad Base64, say) stops the run and changes nothing.
{
	// "name": "Tidy up",    // shown in previews and reports; the file name by default
	"steps": [
		{ "op": "trim_trailing_white_space" },

		// Every operation, with its options (the values shown are the defaults where there is one):
		//
		// Lines
		// { "op": "sort_lines", "case_sensitive": false },
		// { "op": "reverse_lines" },
		// { "op": "unique_lines" },
		// { "op": "sort_lines_by_pattern", "pattern": "(\\d+)", "capture": null, "order": "lexical", "case_insensitive": false, "numeric": false, "reverse": false, "keep_header": false },
		//     capture: null = group 1 or the match, 0 = whole match, N = validated group N
		//     order: "lexical", "natural", "locale" (current macOS locale), "length" (graphemes)
		//     numeric: finite f64 with lexical fallback; requires order "lexical"
		//     ties stay stable even in reverse; unmatched lines stay last; keep_header skips each block's first line
		// { "op": "process_duplicate_lines", "pattern": "", "compare": "exact", "action": "keep_first" },
		//     compare: "exact", "ignore_case" or "ignore_space"
		//     action: "keep_first", "keep_last", "delete_all" or "keep_only_duplicated"
		// { "op": "process_lines_matching", "pattern": "TODO", "regex": false, "case_insensitive": false, "invert": false, "action": "delete" },
		//     action: "keep" or "delete" (copy and extract are in the Edit menu only)
		// { "op": "add_line_prefix", "text": "> ", "skip_blank": true },
		// { "op": "add_line_suffix", "text": ";", "skip_blank": true },
		// { "op": "remove_line_prefix", "text": "> " },
		// { "op": "remove_line_suffix", "text": ";" },
		// { "op": "trim_trailing_white_space" },
		// { "op": "remove_blank_lines" },
		//
		// Text
		// { "op": "convert_case", "case": "upper" },
		//     case: "upper", "lower", "title", "swap", "lower_camel", "upper_camel", "snake" or "kebab"
		// { "op": "straighten_quotes" },
		// { "op": "smart_quotes" },
		// { "op": "unicode_normalization", "form": "nfc" },
		//     form: "nfc", "nfd", "nfkc" or "nfkd"
		// { "op": "strip_diacritics" },
		// { "op": "encode", "codec": "base64" },
		// { "op": "decode", "codec": "base64" },
		//     codec: "base64", "base64_url", "hex", "uri_component", "html_entities" or "json_string"
		//     "regex_literal" supports encode only (Escape Regex)
		// { "op": "decode_text_escapes" },
		// { "op": "adjust_values", "step": "1" }, // whole selected value; signed 128-bit string step
		// { "op": "apply_substitution_table", "rows": [["find", "replace"]], "regex": false, "case_insensitive": false, "whole_word": false },
		//     or "file": "~/words.tsv" in place of "rows": a find, a tab and a replacement per line
		//     (a path with no folder is looked up beside the pipelines)
	]
}
"##
    .into()
}

/// One step: an operation and its options. Unknown keys are errors, so a typo is caught.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    SortLinesByPattern(SortOpts),
    ProcessDuplicateLines(DupOpts),
    ProcessLinesMatching(MatchOpts),
    AddLinePrefix {
        text: String,
        #[serde(default = "yes")]
        skip_blank: bool,
    },
    AddLineSuffix {
        text: String,
        #[serde(default = "yes")]
        skip_blank: bool,
    },
    RemoveLinePrefix {
        text: String,
    },
    RemoveLineSuffix {
        text: String,
    },
    StraightenQuotes {},
    SmartQuotes {},
    UnicodeNormalization {
        form: NormForm,
    },
    StripDiacritics {},
    ApplySubstitutionTable(TableStep),
    SortLines {
        #[serde(default)]
        case_sensitive: bool,
    },
    ReverseLines {},
    UniqueLines {},
    ConvertCase {
        case: Case,
    },
    TrimTrailingWhiteSpace {},
    RemoveBlankLines {},
    Encode {
        codec: Codec,
    },
    Decode {
        codec: Codec,
    },
    DecodeTextEscapes {},
    AdjustValues {
        #[serde(default)]
        step: crate::editing::columns::ValueStep,
    },
}

fn yes() -> bool {
    true
}

/// A substitution table given inline (`rows`) or by path (`file`), compiled once.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableStep {
    #[serde(default)]
    rows: Option<Vec<(String, String)>>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    case_insensitive: bool,
    #[serde(default)]
    whole_word: bool,
    #[serde(skip)]
    table: OnceLock<Table>,
}

impl TableStep {
    /// The table, read and compiled the first time it is asked for (a pipeline is validated
    /// before it runs, so a batch does not read the file again for each of its files).
    fn table(&self) -> Result<&Table, Vec<String>> {
        if let Some(t) = self.table.get() {
            return Ok(t);
        }
        let o = TableOpts { regex: self.regex, case_insensitive: self.case_insensitive, whole_word: self.whole_word };
        let built = match (&self.rows, &self.file) {
            (Some(rows), None) => {
                Table::from_rows(rows, &o).map_err(|e| e.iter().map(|m| m.replacen("Line ", "row ", 1)).collect())
            }
            (None, Some(file)) => {
                let path = table_path(file);
                let rows = std::fs::read_to_string(&path).map_err(|e| vec![format!("{}: {e}", path.display())])?;
                Table::parse(rows.trim_start_matches('\u{feff}'), &o)
                    .map_err(|e| e.iter().map(|m| format!("{}: {m}", path.display())).collect())
            }
            _ => Err(vec!["give the table as \"rows\" or as a \"file\", and not both".into()]),
        }?;
        Ok(self.table.get_or_init(|| built))
    }
}

/// A table file's path: `~` is the home folder, and a name with no folder is beside the pipelines.
fn table_path(file: &str) -> PathBuf {
    let p = files::untilde(file);
    match p.is_relative() {
        true => dir().join(p),
        false => p,
    }
}

impl Op {
    /// The name the file gives the operation.
    pub fn name(&self) -> &'static str {
        match self {
            Op::SortLinesByPattern(_) => "sort_lines_by_pattern",
            Op::ProcessDuplicateLines(_) => "process_duplicate_lines",
            Op::ProcessLinesMatching(_) => "process_lines_matching",
            Op::AddLinePrefix { .. } => "add_line_prefix",
            Op::AddLineSuffix { .. } => "add_line_suffix",
            Op::RemoveLinePrefix { .. } => "remove_line_prefix",
            Op::RemoveLineSuffix { .. } => "remove_line_suffix",
            Op::StraightenQuotes {} => "straighten_quotes",
            Op::SmartQuotes {} => "smart_quotes",
            Op::UnicodeNormalization { .. } => "unicode_normalization",
            Op::StripDiacritics {} => "strip_diacritics",
            Op::ApplySubstitutionTable(_) => "apply_substitution_table",
            Op::SortLines { .. } => "sort_lines",
            Op::ReverseLines {} => "reverse_lines",
            Op::UniqueLines {} => "unique_lines",
            Op::ConvertCase { .. } => "convert_case",
            Op::TrimTrailingWhiteSpace {} => "trim_trailing_white_space",
            Op::RemoveBlankLines {} => "remove_blank_lines",
            Op::Encode { .. } => "encode",
            Op::Decode { .. } => "decode",
            Op::DecodeTextEscapes {} => "decode_text_escapes",
            Op::AdjustValues { .. } => "adjust_values",
        }
    }

    /// Whether the step reads its text as lines, so that a selection is widened to whole ones.
    fn works_on_lines(&self) -> bool {
        !matches!(
            self,
            Op::StraightenQuotes {}
                | Op::SmartQuotes {}
                | Op::UnicodeNormalization { .. }
                | Op::StripDiacritics {}
                | Op::ApplySubstitutionTable(_)
                | Op::ConvertCase { .. }
                | Op::Encode { .. }
                | Op::Decode { .. }
                | Op::DecodeTextEscapes {}
                | Op::AdjustValues { .. }
        )
    }

    /// What is wrong with the step's options: a bad pattern or table, or a step a pipeline
    /// cannot take. Found by running it on nothing.
    fn check(&self) -> Result<(), Vec<String>> {
        match self {
            Op::ApplySubstitutionTable(t) => t.table().map(drop),
            // JSON strings cannot decode empty input; validate the direction, not fixture content.
            Op::Decode { codec: Codec::JsonString } | Op::AdjustValues { .. } => Ok(()),
            _ => self.apply("").map(drop).map_err(|e| vec![e]),
        }
    }

    fn apply(&self, text: &str) -> Result<String, String> {
        let affix = |t: &str| match t.is_empty() {
            true => Err("the text is empty".to_string()),
            false => Ok(t.to_string()),
        };
        Ok(match self {
            Op::SortLinesByPattern(o) => transform::sort_by_pattern(text, o)?,
            Op::ProcessDuplicateLines(o) => transform::process_duplicates(text, o)?.0,
            Op::ProcessLinesMatching(o) => {
                let interactive = match o.action {
                    MatchAction::Copy => Some("copy"),
                    MatchAction::Extract => Some("extract"),
                    MatchAction::Keep | MatchAction::Delete => None,
                };
                if let Some(action) = interactive {
                    return Err(format!(
                        "\"{action}\" is interactive (Edit > Transform); a pipeline can \"keep\" or \"delete\" matching lines"
                    ));
                }
                transform::process_matching(text, o)?.0
            }
            Op::AddLinePrefix { text: p, skip_blank } => transform::prefix(text, &affix(p)?, *skip_blank),
            Op::AddLineSuffix { text: s, skip_blank } => transform::suffix(text, &affix(s)?, *skip_blank),
            Op::RemoveLinePrefix { text: p } => transform::remove_prefix(text, &affix(p)?),
            Op::RemoveLineSuffix { text: s } => transform::remove_suffix(text, &affix(s)?),
            Op::StraightenQuotes {} => transform::straighten_quotes(text),
            Op::SmartQuotes {} => transform::smart_quotes(text),
            Op::UnicodeNormalization { form } => transform::normalize(text, *form),
            Op::StripDiacritics {} => transform::strip_diacritics(text),
            Op::ApplySubstitutionTable(t) => t.table().map_err(|e| e.join("; "))?.apply(text),
            Op::SortLines { case_sensitive } => permuted(text, Permute::Sort { case_sensitive: *case_sensitive }),
            Op::ReverseLines {} => permuted(text, Permute::Reverse),
            Op::UniqueLines {} => permuted(text, Permute::Unique),
            Op::ConvertCase { case } => text::convert_case(text, *case),
            Op::TrimTrailingWhiteSpace {} => {
                text.split('\n').map(|l| l.trim_end_matches([' ', '\t'])).collect::<Vec<_>>().join("\n")
            }
            // a line of spaces and tabs goes with its line break, as Edit > Text > Remove Blank Lines
            Op::RemoveBlankLines {} => {
                text.split_inclusive('\n').filter(|l| !l.trim_matches(['\n', ' ', '\t']).is_empty()).collect()
            }
            Op::Encode { codec } => convert::encode(*codec, text),
            Op::Decode { codec } => convert::decode(*codec, text)?,
            Op::DecodeTextEscapes {} => convert::decode_text_escapes(text)?,
            Op::AdjustValues { step } => crate::editing::columns::adjust_value(text, *step)?,
        })
    }
}

/// `text`'s lines through Sort, Reverse or Unique, a final line break kept.
fn permuted(text: &str, op: Permute) -> String {
    let (lines, ends) = transform::split(text);
    let lines = lines.into_iter().map(String::from).collect();
    transform::join(&text::permute(lines, op), ends)
}

#[derive(Debug)]
pub struct Pipeline {
    pub name: String,
    pub steps: Vec<Op>,
}

/// The pipeline a file's text defines, or every problem in it, each naming its line.
pub fn parse(text: &str) -> Result<Pipeline, Vec<String>> {
    // editors on Windows save JSON with a byte order mark, which serde_json rejects
    let json = strip_json(text.strip_prefix('\u{feff}').unwrap_or(text));
    if json.trim().is_empty() {
        return Err(vec!["The pipeline file is empty".into()]);
    }
    let value: Value = serde_json::from_str(&json).map_err(|e| vec![e.to_string()])?;
    let Value::Object(mut map) = value else {
        return Err(vec!["A pipeline file is one JSON object, { \"steps\": [ … ] }".into()]);
    };
    let unknown = map.keys().filter(|k| !["name", "steps"].contains(&k.as_str()));
    let mut errors: Vec<String> = (unknown.map(|key| (line_of(&json, key), key)))
        .map(|(line, key)| format!("Line {line}: unknown key \"{key}\" (a pipeline has \"name\" and \"steps\")"))
        .collect();
    let name = match map.remove("name") {
        None => String::new(),
        Some(Value::String(name)) => name,
        Some(_) => {
            let line = line_of(&json, "name");
            errors.push(format!("Line {line}: \"name\" must be text"));
            String::new()
        }
    };
    let steps = match map.remove("steps") {
        Some(Value::Array(steps)) if !steps.is_empty() => steps,
        Some(steps) => {
            let line = line_of(&json, "steps");
            errors.push(match steps.is_array() {
                true => format!("Line {line}: a pipeline needs at least one step"),
                false => format!("Line {line}: \"steps\" must be a list"),
            });
            Vec::new()
        }
        None => {
            errors.push("A pipeline needs a \"steps\" list".into());
            Vec::new()
        }
    };
    let lines = step_lines(&json);
    let mut ops = Vec::new();
    for (i, step) in steps.into_iter().enumerate() {
        let (n, line) = (i + 1, lines.get(i).copied().unwrap_or(1));
        match serde_json::from_value::<Op>(step) {
            Err(e) => errors.push(format!("Line {line}: step {n}: {e}")),
            Ok(op) => match op.check() {
                Ok(()) => ops.push(op),
                Err(problems) => {
                    errors.extend(problems.into_iter().map(|p| format!("Line {line}: step {n} ({}): {p}", op.name())))
                }
            },
        }
    }
    match errors.is_empty() {
        true => Ok(Pipeline { name, steps: ops }),
        false => Err(errors),
    }
}

/// The 1-based line each step of the top-level `"steps"` list starts on, whatever the step is
/// (`json` is valid, with its comments blanked).
fn step_lines(json: &str) -> Vec<usize> {
    let (mut lines, mut depth, mut line) = (Vec::new(), 0, 1);
    // the last string seen is the key a list is the value of; `list` is the depth of the steps;
    // `next` is set where a step may start: after the list's `[` and after its commas
    let (mut last, mut list, mut next) = (String::new(), None, false);
    let mut chars = json.chars();
    while let Some(c) = chars.next() {
        if next && !c.is_whitespace() && c != ']' {
            lines.push(line);
            next = false;
        }
        match c {
            '\n' => line += 1,
            '"' => {
                last.clear();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => last.extend(chars.next()),
                        '"' => break,
                        '\n' => line += 1,
                        c => last.push(c),
                    }
                }
            }
            '[' | '{' => {
                if c == '[' && depth == 1 && last == "steps" {
                    (list, next) = (Some(depth + 1), true);
                }
                depth += 1;
            }
            ']' | '}' => {
                if c == ']' && list == Some(depth) {
                    list = None;
                }
                depth -= 1;
            }
            ',' if list == Some(depth) => next = true,
            _ => {}
        }
    }
    lines
}

/// `path` read and parsed; a pipeline with no name takes its file's.
pub fn load(path: &Path) -> Result<Pipeline, Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|e| vec![format!("{}: {e}", path.display())])?;
    let mut p = parse(&text)?;
    if p.name.is_empty() {
        let stem = path.file_stem().unwrap_or_default();
        p.name = stem.to_string_lossy().into_owned();
    }
    Ok(p)
}

/// `errors` for an alert: the first dozen.
pub fn problems(errors: &[String]) -> String {
    let mut shown: Vec<String> = errors.iter().take(SHOWN_PROBLEMS).cloned().collect();
    if errors.len() > SHOWN_PROBLEMS {
        shown.push(format!("…and {} more", errors.len() - SHOWN_PROBLEMS));
    }
    shown.join("\n")
}

impl Pipeline {
    /// `text` through every step; a failing step is its number (from 1) and what went wrong.
    pub fn run(&self, text: &str) -> Result<String, (usize, String)> {
        let mut out = text.to_string();
        for (i, op) in self.steps.iter().enumerate() {
            out = op.apply(&out).map_err(|e| (i + 1, e))?;
            // line breaks are LF in a buffer, whatever a step wrote (and the next step reads)
            if out.contains('\r') {
                out = text::normalized(&out).0.into_owned();
            }
        }
        Ok(out)
    }

    /// "Step 2 (decode): Invalid Base64", for a failure of `run`.
    pub fn failure(&self, (n, message): (usize, String)) -> String {
        let op = self.steps.get(n.wrapping_sub(1)).map_or("", Op::name);
        format!("Step {n} ({op}): {message}")
    }

    /// Only this new operation needs the additive immutable UI preflight.
    pub fn adjusts_values(&self) -> bool {
        self.steps.iter().any(|op| matches!(op, Op::AdjustValues { .. }))
    }

    /// Some step works on lines, so a selection is widened to whole lines.
    pub fn lines(&self) -> bool {
        self.steps.iter().any(Op::works_on_lines)
    }
}

/// What a batch did to one file.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Changed,
    Unchanged,
    /// Changed in an open buffer, not saved.
    OpenUnsaved,
    Skipped(String),
}

#[derive(Debug, Default)]
pub struct BatchReport {
    pub changed: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
    pub open_unsaved: Vec<PathBuf>,
    pub skipped: Vec<(PathBuf, String)>,
    pub failed: Vec<(PathBuf, String)>,
    /// Why the run ended before it was done, when it did not say so itself.
    pub aborted: Option<String>,
}

impl BatchReport {
    pub fn add(&mut self, path: PathBuf, result: Result<Outcome, String>) {
        match result {
            Ok(Outcome::Changed) => self.changed.push(path),
            Ok(Outcome::Unchanged) => self.unchanged.push(path),
            Ok(Outcome::OpenUnsaved) => self.open_unsaved.push(path),
            Ok(Outcome::Skipped(why)) => self.skipped.push((path, why)),
            Err(why) => self.failed.push((path, why)),
        }
    }

    /// The report tab's text.
    pub fn text(&self, name: &str, cancelled: bool) -> String {
        let mut out = format!("Pipeline: {name}\n");
        if cancelled {
            out += "Cancelled: the files after these were not touched.\n";
        }
        if let Some(why) = &self.aborted {
            out += &format!("Stopped: {why}\n");
        }
        let paths = |l: &[PathBuf]| l.iter().map(|p| p.display().to_string()).collect();
        let whys = |l: &[(PathBuf, String)]| (l.iter()).map(|(p, why)| format!("{}: {why}", p.display())).collect();
        let sections: [(&str, Vec<String>); 5] = [
            ("Changed", paths(&self.changed)),
            ("Unchanged", paths(&self.unchanged)),
            ("Changed in open buffer, not saved", paths(&self.open_unsaved)),
            ("Skipped", whys(&self.skipped)),
            ("Failed", whys(&self.failed)),
        ];
        let stopped = cancelled || self.aborted.is_some();
        if !stopped && sections.iter().all(|(_, rows)| rows.is_empty()) {
            out += "\nNo files matched.\n";
        }
        for (heading, rows) in sections.iter().filter(|(_, rows)| !rows.is_empty()) {
            out += &format!("\n{heading} ({}):\n", rows.len());
            rows.iter().for_each(|row| out += &format!("    {row}\n"));
        }
        out
    }
}

/// One closed file through the pipeline, written back safely when it changed: its line ending
/// and byte order mark stay. An error leaves the file as it was.
#[cfg(any(test, feature = "test-support"))]
pub fn run_file(path: &Path, p: &Pipeline) -> Result<Outcome, String> {
    run_file_with_history(path, p, false)
}

pub fn run_file_with_history(path: &Path, p: &Pipeline, history: bool) -> Result<Outcome, String> {
    run_file_with_hint(path, p, history, None)
}

pub fn run_file_with_hint(
    path: &Path,
    p: &Pipeline,
    history: bool,
    hint: Option<orlo_project::encoding::Encoding>,
) -> Result<Outcome, String> {
    let skip = |why: &str| Ok(Outcome::Skipped(why.into()));
    let known = orlo_project::watch::stamp(path);
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let file = match files::read_file_with_hint(path, hint) {
        Ok(file) => file,
        Err(None) => return skip("over 100 MB"),
        Err(Some(why)) => return Err(why),
    };
    if file.text.bytes().take(BINARY_PROBE).any(|b| b == 0) {
        return skip("binary file");
    }
    if file.lossy {
        return skip("malformed text; reopen with the correct encoding");
    }
    let old = file.text;
    let new = p.run(&old).map_err(|e| p.failure(e))?;
    if new == old {
        return Ok(Outcome::Unchanged);
    }
    // the safe save replaces the file by renaming over it, which a read-only mode would not stop
    if meta.permissions().readonly() {
        return Err("the file is read-only".into());
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(change) = BEFORE_PUBLISH.take() {
        change(path);
    }
    // A slow transform must not replace an external edit made since its input was read.
    // As with Replace in Files, a final rename can still race a later external writer.
    let changed = || "the file changed on disk while the pipeline ran".to_string();
    let stamped = || orlo_project::watch::stamp(path) == known;
    let same = || stamped() && files::read_bytes(path).map(|b| files::hash(&b)).ok() == Some(file.hash);
    if !same() || !stamped() {
        return Err(changed());
    }
    let (before, after) = (ropey::Rope::from_str(&old), ropey::Rope::from_str(&new));
    let mut endings = file.endings;
    endings.apply_diff(&before, &after, file.line_ending);
    let output = endings.text(&after, 0..after.len_chars()).map_err(|e| e.to_string())?;
    let bytes = orlo_project::encoding::encode_owned(output, file.encoding, file.bom).map_err(|e| e.to_string())?;
    text::capture_history(path, file.encoding, history).map_err(|e| e.to_string())?;
    if !same() {
        return Err(changed());
    }
    let write = |f: &mut std::fs::File| std::io::Write::write_all(f, &bytes);
    text::write_safely(path, &write).map_err(|(e, touched)| match touched {
        // a file Orlo cannot replace (another name for it, another owner) is written in place
        true => format!("write failed, the file may be partly written: {e}"),
        false => e.to_string(),
    })?;
    Ok(Outcome::Changed)
}

/// The batch's files, with symlinks and explicit paths naming the same target taken once.
pub fn collect(scope: &Scope, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    files::collect(scope, cancel, &AtomicUsize::new(0))
        .iter()
        .map(|f| text::resolve(f))
        .filter(|f| seen.insert(f.clone()))
        .collect()
}

/// Each collected file until cancelled. The caller chooses the current buffer or disk;
/// `None` stops without counting an operation that has not started.
pub async fn run_batch(
    files: &[PathBuf],
    cancel: &AtomicBool,
    done: &AtomicUsize,
    total: &AtomicUsize,
    mut run: impl AsyncFnMut(&Path) -> Option<Result<Outcome, String>>,
    mut emit: impl FnMut(PathBuf, Result<Outcome, String>),
) {
    total.store(files.len(), Ordering::Relaxed);
    for f in files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(result) = run(f).await else { break };
        emit(f.clone(), result);
        done.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::Ordering;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orlo-pipeline-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pipeline(steps: &str) -> Pipeline {
        parse(&format!(r#"{{ "name": "t", "steps": [ {steps} ] }}"#)).unwrap()
    }

    fn run(steps: &str, text: &str) -> String {
        pipeline(steps).run(text).unwrap()
    }

    #[test]
    fn n03_text_escapes_decode_exact_unicode_and_control_forms_once() {
        let p = pipeline(r#"{ "op": "decode_text_escapes" }"#);
        assert!(!p.steps[0].works_on_lines());
        assert_eq!(convert::decode_text_escapes(r"\r\n").unwrap(), "\r\n");
        assert_eq!(p.run(r"\r\n").unwrap(), "\n");
        for (input, want) in [
            ("", ""),
            (r#"\\\"\'\n\r\t\b\f"#, "\\\"'\n\r\t\u{8}\u{c}"),
            (r"\x00\x7F\x80\xFF", "\0\u{7f}\u{80}ÿ"),
            (r"\xC3\xA9", "Ã©"),
            (r"\u0000\u00e9\uFFFF", "\0é\u{ffff}"),
            (r"\uD800\uDC00\uDBFF\uDFFF\ud83d\ude00", "\u{10000}\u{10ffff}😀"),
            (r"\U00000041\U0001F600\U0010FFFF", "A😀\u{10ffff}"),
            (r"\\n", r"\n"),
            ("é😀\n\t'\" + &amp; %20", "é😀\n\t'\" + &amp; %20"),
        ] {
            assert_eq!(p.run(input).unwrap(), want, "{input:?}");
        }
    }

    #[test]
    fn n03_text_escapes_reject_malformed_unknown_and_non_scalar_forms() {
        let p = pipeline(r#"{ "op": "decode_text_escapes" }"#);
        for bad in [
            "\\",
            r"\q",
            r"\/",
            r"\0",
            r"\a",
            r"\v",
            r"\N{SPACE}",
            r"\x",
            r"\x0",
            r"\xG0",
            r"\xé0",
            r"\x１２",
            r"\u",
            r"\u123",
            r"\u{41}",
            r"\uGGGG",
            r"\uD800",
            r"\uDC00",
            r"\uDFFF",
            r"\uD800x",
            r"\uD800\x41",
            r"\uD800\u0041",
            r"\uD800\uD800",
            r"\uDC00\uD800",
            r"\uD800\uDC0",
            r"\uD800\U0000DC00",
            r"\U",
            r"\U0000041",
            r"\U00110000",
            r"\U0000D800",
            r"\U0000DC00",
            r"\UFFFFFFFF",
        ] {
            let error = p.run(bad).unwrap_err();
            assert_eq!(error.0, 1, "{bad:?}: {error:?}");
            assert!(!error.1.is_empty());
        }
    }

    #[test]
    fn n03_named_literal_codecs_are_pipeline_steps_without_line_widening() {
        let input = "\"line\n😀\\";
        let quoted = pipeline(r#"{ "op": "encode", "codec": "json_string" }"#);
        let unquoted = pipeline(r#"{ "op": "decode", "codec": "json_string" }"#);
        assert!(!quoted.steps[0].works_on_lines());
        assert!(!unquoted.steps[0].works_on_lines());
        assert_eq!(unquoted.run(&quoted.run(input).unwrap()).unwrap(), input);
        for bad in ["", "null", "[1]", "\"\\q\""] {
            assert!(unquoted.run(bad).is_err());
        }
        assert_eq!(run(r#"{ "op": "encode", "codec": "regex_literal" }"#, "a.b+"), r"a\.b\+");
        assert!(parse(r#"{ "steps": [{ "op": "decode", "codec": "regex_literal" }] }"#).is_err());
        assert!(template().contains("decode_text_escapes"));
        assert!(template().contains("json_string"));
        assert!(template().contains("regex_literal"));
    }

    #[test]
    fn legacy_pipeline_hint_preserves_bytes_and_rejects_unrepresentable_output_before_history() {
        use orlo_project::encoding::Encoding;
        let dir = temp("f01-pipeline");
        let path = dir.join("source.txt");
        std::fs::write(&path, b"caf\xe9\r\n").unwrap();
        let uppercase = pipeline(r#"{ "op": "convert_case", "case": "upper" }"#);
        assert_eq!(run_file_with_hint(&path, &uppercase, false, Some(Encoding::Windows1252)), Ok(Outcome::Changed));
        assert_eq!(std::fs::read(&path).unwrap(), b"CAF\xc9\r\n");
        let p = pipeline(r#"{ "op": "add_line_prefix", "text": "🦀" }"#);
        let history = dir.join("history");
        let result = orlo_project::file_history::with_root(history.clone(), || {
            run_file_with_hint(&path, &p, true, Some(Encoding::Windows1252))
        });
        assert!(result.unwrap_err().contains("'🦀' (U+1F980) at line 1, column 1"));
        assert!(!history.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"CAF\xc9\r\n");
    }

    #[test]
    fn parse_names_each_problem_with_its_line() {
        let text = r#"// my pipeline
{
  "name": "demo",
  "steps": [
    { "op": "convert_case", "case": "upper" },
    { "op": "shout" },
    { "op": "sort_lines_by_pattern", "pattern": "(" },
    { "op": "convert_case", "case": "upper", "extra": 1 },
    { "op": "convert_case" },
  ]
}"#;
        let errors = parse(text).unwrap_err();
        assert_eq!(errors.len(), 4, "{errors:#?}");
        for (e, (start, says)) in errors.iter().zip([
            ("Line 6: step 2: ", "shout"),
            ("Line 7: step 3 (sort_lines_by_pattern): ", "regex"),
            ("Line 8: step 4: ", "extra"),
            ("Line 9: step 5: ", "case"),
        ]) {
            assert!(e.starts_with(start) && e.contains(says), "{e}");
        }
    }

    #[test]
    fn a_steps_line_is_where_it_starts_whatever_its_strings_hold() {
        let text = "{\n  \"steps\": [\n    { \"op\": \"add_line_prefix\", \"text\": \"{[\\\"\" },\n    {\n      \"op\": \"bogus\"\n    },\n    { \"op\": \"add_line_suffix\", \"text\": \"]}\" }, { \"op\": \"nope\" }\n  ]\n}";
        let errors = parse(text).unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:#?}");
        assert!(errors[0].starts_with("Line 4: step 2: "), "{}", errors[0]);
        assert!(errors[1].starts_with("Line 7: step 4: "), "{}", errors[1]);
    }

    #[test]
    fn a_step_that_is_not_an_object_does_not_shift_the_lines_after_it() {
        let text = "{\n  \"steps\": [\n    3,\n    \"x\", [1, 2],\n    { \"op\": \"nope\" }\n  ]\n}";
        let errors = parse(text).unwrap_err();
        assert_eq!(errors.len(), 4, "{errors:#?}");
        for (error, start) in
            errors.iter().zip(["Line 3: step 1: ", "Line 4: step 2: ", "Line 4: step 3: ", "Line 5: step 4: "])
        {
            assert!(error.starts_with(start), "{error}");
        }
        // on one line, whatever the steps are
        let errors = parse(r#"{ "steps": [3, { "op": "nope" }, null] }"#).unwrap_err();
        assert!(errors.iter().all(|e| e.starts_with("Line 1: ")), "{errors:#?}");
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn parse_refuses_what_is_not_a_pipeline() {
        let one = |t: &str| {
            let mut errors = parse(t).unwrap_err();
            assert_eq!(errors.len(), 1, "{errors:?}");
            errors.remove(0)
        };
        // JSON syntax errors say where
        let e = one("{\n  \"steps\": [\n    { \"op\": }\n  ]\n}");
        assert!(e.contains("line 3"), "{e}");
        assert!(one("").contains("empty"));
        assert!(one("// only a comment\n").contains("empty"));
        assert!(one("[1]").contains("object"));
        assert!(one(r#"{ "name": "x" }"#).contains("steps"));
        assert!(one(r#"{ "steps": [] }"#).contains("at least one step"));
        assert!(one(r#"{ "steps": {} }"#).contains("list"));
        assert!(one(r#"{ "name": 3, "steps": [{ "op": "reverse_lines" }] }"#).contains("name"));
        let e = one("{\n  \"stepz\": [],\n  \"steps\": [{ \"op\": \"reverse_lines\" }]\n}");
        assert!(e.starts_with("Line 2: ") && e.contains("stepz"), "{e}");
        let e = one(r#"{ "steps": [ 3 ] }"#);
        assert!(e.starts_with("Line 1: step 1: "), "{e}");
        // a byte order mark and comments, trailing commas: Sublime's JSON
        let p = parse("\u{feff}// c\n{ \"steps\": [ { \"op\": \"reverse_lines\", }, ], }").unwrap();
        assert_eq!((p.name.as_str(), p.steps.len()), ("", 1));
    }

    #[test]
    fn steps_are_validated_before_anything_runs() {
        let err = |step: &str| parse(&format!("{{ \"steps\": [ {step} ] }}")).unwrap_err().join("\n");
        // copy and extract are interactive
        for action in ["copy", "extract"] {
            let e = err(&format!(r#"{{ "op": "process_lines_matching", "pattern": "a", "action": "{action}" }}"#));
            assert!(e.contains("interactive") && e.contains(action), "{e}");
        }
        assert!(err(r#"{ "op": "reverse_lines", "foo": 1 }"#).contains("foo"));
        assert!(err(r#"{ "op": "process_lines_matching" }"#).contains("pattern"));
        assert!(err(r#"{ "op": "add_line_prefix", "text": "" }"#).contains("empty"));
        assert!(err(r#"{ "op": "process_duplicate_lines", "pattern": "[" }"#).contains("regex"));
        // tables: rows or a file, validated with the table's own messages
        let e = err(r#"{ "op": "apply_substitution_table", "rows": [["", "x"], ["a", "1"], ["a", "2"]] }"#);
        assert!(e.contains("row 1") && e.contains("empty"), "{e}");
        assert!(e.contains("row 3") && e.contains("already"), "{e}");
        let e = err(r#"{ "op": "apply_substitution_table", "regex": true, "rows": [["(", "x"]] }"#);
        assert!(e.contains("row 1") && e.contains("regex"), "{e}");
        let e = err(r#"{ "op": "apply_substitution_table" }"#);
        assert!(e.contains("rows") && e.contains("file"), "{e}");
        let e = err(r#"{ "op": "apply_substitution_table", "rows": [], "file": "x.tsv" }"#);
        assert!(e.contains("rows") && e.contains("file"), "{e}");
        let e = err(r#"{ "op": "apply_substitution_table", "file": "/no/such/table.tsv" }"#);
        assert!(e.contains("/no/such/table.tsv"), "{e}");
        let e = err(r#"{ "op": "apply_substitution_table", "rows": [["a"]] }"#);
        assert!(e.contains("step 1"), "{e}");
    }

    #[test]
    fn a_table_file_is_read_once_when_the_pipeline_is_parsed() {
        let dir = temp("table-file");
        let table = dir.join("t.tsv");
        std::fs::write(&table, "# swaps\ncat\tdog\nbad\n").unwrap();
        let text =
            format!(r#"{{ "steps": [ {{ "op": "apply_substitution_table", "file": "{}" }} ] }}"#, table.display());
        let e = parse(&text).unwrap_err().remove(0);
        assert!(e.contains("t.tsv") && e.contains("Line 3"), "{e}");
        std::fs::write(&table, "cat\tdog\n").unwrap();
        let p = parse(&text).unwrap();
        // a later change to the file does not reach a pipeline already validated
        std::fs::remove_file(&table).unwrap();
        assert_eq!(p.run("cat cat").unwrap(), "dog dog");
    }

    #[test]
    fn every_op_parses_is_named_as_in_the_file_and_is_in_the_template() {
        let steps = [
            r#"{ "op": "sort_lines_by_pattern", "pattern": "(\\d+)", "numeric": true, "reverse": true, "case_insensitive": true }"#,
            r#"{ "op": "process_duplicate_lines", "pattern": "", "compare": "ignore_case", "action": "keep_last" }"#,
            r#"{ "op": "process_lines_matching", "pattern": "x", "regex": true, "case_insensitive": true, "invert": true, "action": "delete" }"#,
            r#"{ "op": "add_line_prefix", "text": "> ", "skip_blank": false }"#,
            r#"{ "op": "add_line_suffix", "text": ";" }"#,
            r#"{ "op": "remove_line_prefix", "text": "> " }"#,
            r#"{ "op": "remove_line_suffix", "text": ";" }"#,
            r#"{ "op": "straighten_quotes" }"#,
            r#"{ "op": "smart_quotes" }"#,
            r#"{ "op": "unicode_normalization", "form": "nfkd" }"#,
            r#"{ "op": "strip_diacritics" }"#,
            r#"{ "op": "apply_substitution_table", "rows": [["a", "b"]], "regex": false, "case_insensitive": true, "whole_word": true }"#,
            r#"{ "op": "sort_lines", "case_sensitive": true }"#,
            r#"{ "op": "reverse_lines" }"#,
            r#"{ "op": "unique_lines" }"#,
            r#"{ "op": "convert_case", "case": "lower_camel" }"#,
            r#"{ "op": "trim_trailing_white_space" }"#,
            r#"{ "op": "remove_blank_lines" }"#,
            r#"{ "op": "encode", "codec": "base64_url" }"#,
            r#"{ "op": "decode", "codec": "hex" }"#,
        ];
        let p = pipeline(&steps.join(",\n"));
        let template = template();
        for (step, op) in steps.iter().zip(&p.steps) {
            let value: serde_json::Value = serde_json::from_str(step).unwrap();
            let name = value["op"].as_str().unwrap();
            assert_eq!(op.name(), name);
            assert!(template.contains(&format!("\"{name}\"")), "{name}");
        }
        let template_p = parse(&template).unwrap();
        assert!(!template_p.steps.is_empty());
    }

    #[test]
    fn the_readmes_example_is_a_valid_pipeline() {
        let readme = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"));
        let example = readme.split("```json\n").nth(1).and_then(|s| s.split("```").next());
        let p = parse(example.expect("a JSON example in the README")).unwrap();
        assert_eq!((p.name.as_str(), p.steps.len()), ("Tidy a list", 5));
        assert_eq!(p.run("b 10 \n# c\na 9\ncolour").unwrap(), "- a 9\n- b 10\n- color");
    }

    #[test]
    fn the_cases_and_codecs_have_their_names() {
        for (case, text, want) in [
            ("upper", "ab cd", "AB CD"),
            ("lower", "AB", "ab"),
            ("title", "ab cd", "Ab Cd"),
            ("swap", "aB", "Ab"),
            ("lower_camel", "foo bar", "fooBar"),
            ("upper_camel", "foo bar", "FooBar"),
            ("snake", "Foo Bar", "foo_bar"),
            ("kebab", "Foo Bar", "foo-bar"),
        ] {
            assert_eq!(run(&format!(r#"{{ "op": "convert_case", "case": "{case}" }}"#), text), want);
        }
        for (codec, want) in [("base64", "aGk="), ("base64_url", "aGk"), ("hex", "6869")] {
            let encode = format!(r#"{{ "op": "encode", "codec": "{codec}" }}"#);
            let decode = format!(r#"{{ "op": "decode", "codec": "{codec}" }}"#);
            assert_eq!(run(&encode, "hi"), want);
            assert_eq!(run(&format!("{encode}, {decode}"), "hi"), "hi");
        }
    }

    #[test]
    fn uri_component_pipeline_uses_the_shared_codec_and_preserves_literal_plus() {
        let encode = r#"{ "op": "encode", "codec": "uri_component" }"#;
        let decode = r#"{ "op": "decode", "codec": "uri_component" }"#;
        assert_eq!(run(encode, "a b/é+😀"), "a%20b%2F%C3%A9%2B%F0%9F%98%80");
        assert_eq!(run(&format!("{encode}, {decode}"), "a b/é+😀"), "a b/é+😀");
        assert_eq!(run(decode, " +%2b "), " ++ ");
        assert!(pipeline(decode).run("%GG").is_err());
    }

    #[test]
    fn html_entities_pipeline_uses_shared_single_pass_validation() {
        let encode = r#"{ "op": "encode", "codec": "html_entities" }"#;
        let decode = r#"{ "op": "decode", "codec": "html_entities" }"#;
        assert_eq!(run(encode, "<&é'"), "&lt;&amp;é&#39;");
        assert_eq!(run(&format!("{encode}, {decode}"), "<&é'"), "<&é'");
        assert_eq!(run(decode, " +&amp;lt;&#X1F600;\n"), " +&lt;😀\n");
        assert!(pipeline(decode).run("&lt;&#xD800;").is_err());
    }

    #[test]
    fn n30_pipeline_uses_sort_options_and_validates_capture_before_running() {
        let step = r#"{ "op":"sort_lines_by_pattern", "pattern":"^([^:]+):(.+)$", "capture":2, "order":"natural", "keep_header":true, "reverse":true }"#;
        assert_eq!(
            run(step, "heading\na:v2-part9\nx\nb:v2-part10\nc:v2-part9\ny"),
            "heading\nb:v2-part10\na:v2-part9\nc:v2-part9\nx\ny"
        );
        let bad =
            r#"{ "steps":[ {"op":"reverse_lines"}, {"op":"sort_lines_by_pattern", "pattern":"(a)", "capture":2} ] }"#;
        let errors = parse(bad).unwrap_err().join("\n");
        assert!(errors.contains("step 2") && errors.contains("capture"), "{errors}");
    }

    #[test]
    fn run_applies_the_steps_in_order() {
        let sort = r#"{ "op": "sort_lines" }"#;
        let sort_case = r#"{ "op": "sort_lines", "case_sensitive": true }"#;
        for (steps, text, want) in [
            (
                r#"{ "op": "sort_lines" }, { "op": "unique_lines" }, { "op": "add_line_prefix", "text": "- " }"#,
                "b\na\nb\n",
                "- a\n- b\n",
            ),
            // the transformations of Edit > Transform, with their options
            (r#"{ "op": "sort_lines_by_pattern", "pattern": "(\\d+)", "numeric": true }"#, "x10\nx9\ny", "x9\nx10\ny"),
            (r#"{ "op": "process_duplicate_lines", "compare": "ignore_case" }"#, "A\na\nb", "A\nb"),
            (
                r#"{ "op": "process_lines_matching", "pattern": "^#", "regex": true, "action": "delete" }"#,
                "#a\nb\n",
                "b\n",
            ),
            (r#"{ "op": "process_lines_matching", "pattern": "b", "action": "keep" }"#, "a\nb\n", "b\n"),
            (r#"{ "op": "add_line_prefix", "text": "> " }"#, "a\n\nb", "> a\n\n> b"),
            (r#"{ "op": "add_line_prefix", "text": "> ", "skip_blank": false }"#, "a\n\n", "> a\n> \n"),
            (r#"{ "op": "add_line_suffix", "text": ";" }"#, "a\nb\n", "a;\nb;\n"),
            (r#"{ "op": "remove_line_prefix", "text": "> " }"#, "> a\nb", "a\nb"),
            (r#"{ "op": "remove_line_suffix", "text": ";" }"#, "a;\nb", "a\nb"),
            (r#"{ "op": "straighten_quotes" }"#, "“a” ‘b’", "\"a\" 'b'"),
            (r#"{ "op": "smart_quotes" }"#, "\"a\" it's", "“a” it’s"),
            (r#"{ "op": "strip_diacritics" }"#, "café", "cafe"),
            (r#"{ "op": "reverse_lines" }"#, "a\nb\nc\n", "c\nb\na\n"),
            (sort, "b\nA\nc", "A\nb\nc"),
            (sort_case, "b\nA\nc", "A\nb\nc"),
            (sort, "b\nB\na", "a\nb\nB"),
            (sort_case, "b\nB\na", "B\na\nb"),
        ] {
            assert_eq!(run(steps, text), want, "{steps}");
        }
        assert_eq!(run(r#"{ "op": "unicode_normalization", "form": "nfd" }"#, "é").chars().count(), 2);
    }

    #[test]
    fn tables_inline_apply_literally_or_as_regexes() {
        let table = |rest: &str, text| run(&format!(r#"{{ "op": "apply_substitution_table", {rest} }}"#), text);
        // one pass: a→b and b→c never chain
        assert_eq!(table(r#""rows": [["a", "b"], ["b", "c"]]"#, "ab"), "bc");
        // a tab or a line break in a replacement is fine in a row
        assert_eq!(table(r##""rows": [["#x", "a\tb\nc"]]"##, "#x"), "a\tb\nc");
        assert_eq!(table(r#""regex": true, "rows": [["(\\w+)@", "$1 at "]]"#, "me@x"), "me at x");
        assert_eq!(
            table(r#""whole_word": true, "case_insensitive": true, "rows": [["cat", "dog"]]"#, "Cat concat"),
            "dog concat"
        );
    }

    #[test]
    fn line_breaks_a_step_writes_are_lf_for_the_next_step() {
        // "a \r\nb" in Base64
        let steps = r#"{ "op": "decode", "codec": "base64" }, { "op": "trim_trailing_white_space" }"#;
        assert_eq!(run(steps, "YSANCmI="), "a\nb");
        assert_eq!(run(steps, "YSANYg=="), "a\nb", "decoded CR-only text is split into lines");
    }

    #[test]
    fn a_failing_step_is_its_number_and_message() {
        let p = pipeline(r#"{ "op": "convert_case", "case": "upper" }, { "op": "decode", "codec": "base64" }"#);
        let failed = p.run("not base64!").unwrap_err();
        assert_eq!(failed.0, 2);
        assert!(failed.1.contains("Base64"), "{}", failed.1);
        let message = p.failure(failed);
        assert!(message.starts_with("Step 2 (decode): "), "{message}");
        assert_eq!(pipeline(r#"{ "op": "sort_lines" }"#).run("").unwrap(), "");
    }

    #[test]
    fn lines_is_true_when_any_step_works_on_lines() {
        assert!(!pipeline(r#"{ "op": "convert_case", "case": "upper" }, { "op": "encode", "codec": "hex" }"#).lines());
        assert!(pipeline(r#"{ "op": "convert_case", "case": "upper" }, { "op": "sort_lines" }"#).lines());
        assert!(pipeline(r#"{ "op": "trim_trailing_white_space" }"#).lines());
        assert!(!pipeline(r#"{ "op": "smart_quotes" }, { "op": "strip_diacritics" }"#).lines());
    }

    #[test]
    fn trim_and_remove_blank_lines_do_what_the_edit_text_commands_do() {
        use crate::buffer::EditorState;
        for text in ["a  \n\t\nb\t \n", "a\n  ", "\n\n", "  ", "", "a\n\n\nb", "x \ny", " a \n \n"] {
            let by_command = |remove_blank| {
                let mut s = EditorState::from_text(text);
                s.clean_whitespace(remove_blank);
                s.text()
            };
            assert_eq!(run(r#"{ "op": "trim_trailing_white_space" }"#, text), by_command(false), "{text:?}");
            assert_eq!(run(r#"{ "op": "remove_blank_lines" }"#, text), by_command(true), "{text:?}");
        }
    }

    #[test]
    fn a_closed_file_is_rewritten_keeping_its_line_ending_and_bom() {
        let dir = temp("crlf-bom");
        let f = dir.join("a.txt");
        std::fs::write(&f, b"\xEF\xBB\xBFb\r\na\r\n").unwrap();
        let p = pipeline(r#"{ "op": "sort_lines" }"#);
        assert_eq!(run_file(&f, &p), Ok(Outcome::Changed));
        assert_eq!(std::fs::read(&f).unwrap(), b"\xEF\xBB\xBFa\r\nb\r\n");
        // nothing left to change: the file is not written again
        let before = std::fs::metadata(&f).unwrap().modified().unwrap();
        assert_eq!(run_file(&f, &p), Ok(Outcome::Unchanged));
        assert_eq!(std::fs::metadata(&f).unwrap().modified().unwrap(), before);
        // no temp file is left beside it
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        // a CR file and a plain LF one keep theirs
        std::fs::write(&f, b"b\ra\r").unwrap();
        assert_eq!(run_file(&f, &p), Ok(Outcome::Changed));
        assert_eq!(std::fs::read(&f).unwrap(), b"a\rb\r");
        std::fs::write(&f, b"b\na").unwrap();
        run_file(&f, &p).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"a\nb");
    }

    #[test]
    fn files_that_are_not_text_are_skipped_and_left_alone() {
        let dir = temp("skipped");
        let p = pipeline(r#"{ "op": "convert_case", "case": "upper" }"#);
        let bad = dir.join("latin1.txt");
        std::fs::write(&bad, b"caf\xE9\n").unwrap();
        let nul = dir.join("bin.dat");
        std::fs::write(&nul, b"abc\0def").unwrap();
        for (file, why) in [(&bad, "malformed"), (&nul, "binary")] {
            let before = std::fs::read(file).unwrap();
            match run_file(file, &p) {
                Ok(Outcome::Skipped(reason)) => assert!(reason.contains(why), "{reason}"),
                other => panic!("{other:?}"),
            }
            assert_eq!(std::fs::read(file).unwrap(), before);
        }
        // a NUL after the first 8 KB is text as far as the check goes
        let late = dir.join("late.txt");
        let mut bytes = vec![b'a'; 9000];
        bytes.push(0);
        std::fs::write(&late, &bytes).unwrap();
        assert_eq!(run_file(&late, &p), Ok(Outcome::Changed));
    }

    #[test]
    fn a_file_changed_while_a_pipeline_runs_is_not_overwritten() {
        let dir = temp("external-write");
        let path = dir.join("input.txt");
        let p = pipeline(r#"{ "op": "convert_case", "case": "upper" }"#);
        for replace in [false, true] {
            std::fs::write(&path, "old text").unwrap();
            BEFORE_PUBLISH.set(Some(if replace {
                |path| {
                    let replacement = path.with_extension("replacement");
                    std::fs::write(&replacement, "external").unwrap();
                    std::fs::rename(replacement, path).unwrap();
                }
            } else {
                |path| std::fs::write(path, "external").unwrap()
            }));
            let result = run_file(&path, &p);
            assert!(result.is_err(), "{result:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_read_only_file_fails_with_its_original_intact() {
        let dir = temp("read-only");
        let f = dir.join("locked.txt");
        std::fs::write(&f, "abc\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
        let p = pipeline(r#"{ "op": "convert_case", "case": "upper" }"#);
        let e = run_file(&f, &p).unwrap_err();
        assert!(e.contains("read-only"), "{e}");
        assert_eq!(std::fs::read(&f).unwrap(), b"abc\n");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file");
        // a file the pipeline would not change is no failure
        std::fs::write(dir.join("same.txt"), "ABC").unwrap();
        std::fs::set_permissions(dir.join("same.txt"), std::fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(run_file(&dir.join("same.txt"), &p), Ok(Outcome::Unchanged));
    }

    #[test]
    fn a_write_that_may_have_cut_the_file_short_says_so() {
        let dir = temp("in-place");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "x").unwrap();
        // a file with another name is written in place, so its other name sees the result
        std::fs::hard_link(&a, &b).unwrap();
        let p = pipeline(r#"{ "op": "convert_case", "case": "upper" }"#);
        text::FAULT.set(Some(("write", std::io::ErrorKind::Other)));
        let in_place = run_file(&a, &p);
        let c = dir.join("c.txt");
        std::fs::write(&c, "x").unwrap();
        let atomic = run_file(&c, &p);
        text::FAULT.set(None);
        let e = in_place.unwrap_err();
        assert!(e.contains("partly written"), "{e}");
        // a replacement that fails leaves the file whole
        let e = atomic.unwrap_err();
        assert!(!e.contains("partly"), "{e}");
        assert_eq!(std::fs::read(&c).unwrap(), b"x");
        // and without the fault the in-place write works
        std::fs::write(&a, "x").unwrap();
        assert_eq!(run_file(&a, &p), Ok(Outcome::Changed));
        assert_eq!(std::fs::read(&b).unwrap(), b"X");
    }

    #[test]
    fn a_failing_step_or_a_missing_file_fails_the_file_only() {
        let dir = temp("failing");
        let f = dir.join("a.txt");
        std::fs::write(&f, "not base64!").unwrap();
        let p = pipeline(r#"{ "op": "decode", "codec": "base64" }"#);
        let e = run_file(&f, &p).unwrap_err();
        assert!(e.starts_with("Step 1 (decode): "), "{e}");
        assert_eq!(std::fs::read(&f).unwrap(), b"not base64!");
        assert!(run_file(&dir.join("gone.txt"), &p).is_err());
        assert!(run_file(&dir, &p).is_err(), "a folder is not a file");
    }

    #[test]
    fn a_file_over_100_mb_is_skipped_unread() {
        let dir = temp("huge");
        let f = dir.join("huge.txt");
        let file = std::fs::File::create(&f).unwrap();
        file.set_len((100 << 20) + 1).unwrap();
        let p = pipeline(r#"{ "op": "sort_lines" }"#);
        match run_file(&f, &p) {
            Ok(Outcome::Skipped(reason)) => assert!(reason.contains("100 MB"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    #[gpui::test]
    async fn cancelling_stops_before_the_next_file() {
        let dir = temp("cancel");
        let files: Vec<PathBuf> = ["a", "b", "c"].iter().map(|n| dir.join(format!("{n}.txt"))).collect();
        for f in &files {
            std::fs::write(f, "b\na").unwrap();
        }
        let p = pipeline(r#"{ "op": "sort_lines" }"#);
        let (cancel, done) = (AtomicBool::new(false), AtomicUsize::new(0));
        let mut report = BatchReport::default();
        run_batch(
            &files,
            &cancel,
            &done,
            &AtomicUsize::new(0),
            async |path| Some(run_file(path, &p)),
            |path, result| {
                report.add(path, result);
                cancel.store(true, Ordering::Relaxed);
            },
        )
        .await;
        assert_eq!(report.changed, files[..1]);
        assert_eq!(done.load(Ordering::Relaxed), 1);
        assert_eq!(std::fs::read(&files[0]).unwrap(), b"a\nb");
        assert_eq!(std::fs::read(&files[1]).unwrap(), b"b\na");
        assert_eq!(std::fs::read(&files[2]).unwrap(), b"b\na");
        // already cancelled: nothing is touched
        let mut seen = 0;
        run_batch(
            &files,
            &cancel,
            &done,
            &AtomicUsize::new(0),
            async |path| Some(run_file(path, &p)),
            |_, _| seen += 1,
        )
        .await;
        assert_eq!(seen, 0);
    }

    #[gpui::test]
    async fn a_batch_changes_a_symlink_target_only_once() {
        let dir = temp("symlink-once");
        let file = dir.join("data.txt");
        let alias = dir.join("alias.txt");
        std::fs::write(&file, "a").unwrap();
        std::os::unix::fs::symlink(&file, &alias).unwrap();
        let scope = Scope {
            roots: vec![dir],
            files: vec![file.clone()],
            include: vec!["*.txt".into()],
            exclude: vec![],
            gitignore: true,
            filter_files: false,
            root_excludes: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        let files = collect(&scope, &cancel);
        let p = pipeline(r#"{ "op": "add_line_prefix", "text": ">" }"#);
        let (done, total) = (AtomicUsize::new(0), AtomicUsize::new(0));
        run_batch(&files, &cancel, &done, &total, async |path| Some(run_file(path, &p)), |_, _| {}).await;
        assert_eq!(std::fs::read_to_string(file).unwrap(), ">a");
        assert_eq!(done.load(Ordering::Relaxed), 1);
        assert_eq!(total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_report_lists_every_kind_of_file_and_whether_it_was_cancelled() {
        let mut r = BatchReport::default();
        for (name, outcome) in [
            ("a", Ok(Outcome::Changed)),
            ("b", Ok(Outcome::Unchanged)),
            ("c", Ok(Outcome::OpenUnsaved)),
            ("d", Ok(Outcome::Skipped("malformed text; reopen with the correct encoding".into()))),
            ("e", Err("permission denied".into())),
        ] {
            r.add(PathBuf::from(format!("/x/{name}")), outcome);
        }
        let text = r.text("demo", false);
        assert!(text.starts_with("Pipeline: demo\n"), "{text}");
        for want in [
            "Changed (1):\n    /x/a\n",
            "Unchanged (1):\n    /x/b\n",
            "Changed in open buffer, not saved (1):\n    /x/c\n",
            "Skipped (1):\n    /x/d: malformed text; reopen with the correct encoding\n",
            "Failed (1):\n    /x/e: permission denied\n",
        ] {
            assert!(text.contains(want), "{want:?} in {text}");
        }
        assert!(!text.contains("Cancelled"));
        assert!(r.text("demo", true).contains("Cancelled"));
        let none = BatchReport::default().text("demo", false);
        assert!(none.contains("No files matched"), "{none}");
        // a run that ended without saying so
        r.aborted = Some("the background run stopped unexpectedly".into());
        let text = r.text("demo", false);
        assert!(text.contains("Stopped: the background run stopped unexpectedly\n"), "{text}");
        assert!(text.contains("/x/a"), "what was done is still listed");
        let none = BatchReport { aborted: Some("x".into()), ..BatchReport::default() };
        assert!(!none.text("demo", false).contains("No files matched"));
    }

    #[test]
    fn names_for_new_pipelines_are_plain_file_names() {
        let file_name = |input| file_name(input, EXT, "pipeline");
        assert_eq!(file_name("  Tidy up "), Ok("Tidy up".into()));
        assert_eq!(file_name("x.orlo-pipeline"), Ok("x".into()));
        assert_eq!(file_name(""), Err("Enter a name for the pipeline".into()));
        for bad in ["", "   ", "a/b", ".hidden", "a\0b", "..", "x:y"] {
            assert!(file_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn problems_are_capped_for_an_alert() {
        let many: Vec<String> = (1..=20).map(|i| format!("Line {i}: bad")).collect();
        let text = problems(&many);
        assert!(text.contains("Line 12: bad") && !text.contains("Line 13"), "{text}");
        assert!(text.ends_with("…and 8 more"), "{text}");
        assert_eq!(problems(&many[..2]), "Line 1: bad\nLine 2: bad");
    }

    #[test]
    fn the_pipelines_live_under_application_support() {
        assert_eq!(dir(), orlo_utils::paths::support_dir().join("Pipelines"));
        assert!(is_path(Path::new("/a/b.orlo-pipeline")));
        assert!(!is_path(Path::new("/a/b.json")));
        assert!(!is_path(Path::new("/a/orlo-pipeline")));
    }

    #[test]
    fn list_finds_the_pipeline_files_in_name_order() {
        let support = temp("support");
        orlo_utils::paths::TEST_SUPPORT.set(Some(support));
        assert!(list().is_empty());
        std::fs::create_dir_all(dir()).unwrap();
        for n in ["b.orlo-pipeline", "a.orlo-pipeline", "notes.txt"] {
            std::fs::write(dir().join(n), "").unwrap();
        }
        let names: Vec<_> = list().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.orlo-pipeline", "b.orlo-pipeline"]);
        orlo_utils::paths::TEST_SUPPORT.set(None);
    }

    #[test]
    fn a_loaded_pipeline_without_a_name_takes_its_file_name() {
        let dir = temp("load");
        let f = dir.join("Tidy.orlo-pipeline");
        std::fs::write(&f, r#"{ "steps": [ { "op": "reverse_lines" } ] }"#).unwrap();
        assert_eq!(load(&f).unwrap().name, "Tidy");
        std::fs::write(&f, r#"{ "name": "Own", "steps": [ { "op": "reverse_lines" } ] }"#).unwrap();
        assert_eq!(load(&f).unwrap().name, "Own");
        std::fs::write(&f, "{ nope").unwrap();
        assert!(!load(&f).unwrap_err().is_empty());
        let e = load(&dir.join("gone.orlo-pipeline")).unwrap_err();
        assert!(e[0].contains("gone.orlo-pipeline"), "{e:?}");
    }
}

#[cfg(test)]
mod n09_tests {
    use super::*;

    #[test]
    fn n09_adjust_pipeline_whole_values_and_checked_steps() {
        let pipeline = parse(r#"{"steps":[{"op":"adjust_values","step":"1"}]}"#);
        assert!(pipeline.is_ok(), "N09 is missing its existing pipeline operation: {pipeline:?}");
        let pipeline = pipeline.unwrap();
        for (input, want) in [
            ("009", "010"),
            ("0x0f", "0x10"),
            ("-001", "000"),
            ("9_999", "10_000"),
            ("2000-02-28", "2000-02-29"),
            ("1999-12-31T23:59:17Z", "2000-01-01T00:00:17Z"),
        ] {
            assert_eq!(pipeline.run(input).unwrap(), want, "{input}");
        }
        for input in ["1 2", "é9", "2023-02-29", "170141183460469231731687303715884105727"] {
            assert!(pipeline.run(input).is_err(), "{input}");
        }
    }
}
