//! Compose and commit the current index, leaving hooks and signing policy to Git.

use crate::operations::{command, input, output, run};
use crate::repository::Target;
use std::collections::HashSet;
use std::path::Path;

/// An explicit per-commit author override; absence keeps Git's normal authorship policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitAuthor {
    pub name: String,
    pub email: String,
}

impl CommitAuthor {
    pub fn validate(&self) -> Result<(), String> {
        self.identity().map(|_| ())
    }

    fn identity(&self) -> Result<String, String> {
        identity(&self.name, &self.email).ok_or_else(|| "Enter a complete author name and valid email".into())
    }
}

fn identity(name: &str, email: &str) -> Option<String> {
    if name.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')) {
        return None;
    }
    let name = name.trim();
    let (local, domain) = email.split_once('@')?;
    if name.is_empty()
        || name.contains(['<', '>'])
        || local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || email.contains(['<', '>'])
        || email.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    Some(format!("{name} <{email}>"))
}

fn coauthor_key(value: &str) -> Option<String> {
    let (name, email) = value.trim().strip_suffix('>')?.split_once('<')?;
    identity(&name.split_whitespace().collect::<Vec<_>>().join(" "), email)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitDraft {
    pub subject: String,
    pub body: String,
    /// One `Name <email>` per line, without the trailer key.
    pub coauthors: String,
}

impl CommitDraft {
    pub fn validate(&self) -> Result<(), String> {
        self.message_parts().map(|_| ())
    }

    fn message_parts(&self) -> Result<(String, Vec<String>), String> {
        let subject = self.subject.trim();
        if subject.is_empty() {
            return Err("Enter a commit subject".into());
        }
        if self.subject.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')) {
            return Err("The commit subject must be one line without control characters".into());
        }
        if self.body.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return Err("The commit body contains a control character".into());
        }
        let mut message = subject.to_string();
        if !self.body.trim().is_empty() {
            message.push_str("\n\n");
            message.push_str(self.body.trim());
        }
        if self.coauthors.chars().any(|c| (c.is_control() && c != '\n') || matches!(c, '\u{2028}' | '\u{2029}')) {
            return Err("Coauthors must be one Name <email> per line without control characters".into());
        }
        let mut trailers = Vec::new();
        for author in self.coauthors.split('\n').filter(|line| !line.trim().is_empty()) {
            let author = author.trim();
            let Some((name, email)) = author.strip_suffix('>').and_then(|s| s.split_once('<')) else {
                return Err("Enter each coauthor as Name <email>".into());
            };
            if name.contains(':') {
                return Err("Enter a coauthor name and valid email as Name <email>".into());
            }
            let author = identity(name, email).ok_or("Enter a coauthor name and valid email as Name <email>")?;
            trailers.push(format!("Co-authored-by: {author}"));
        }
        Ok((message, trailers))
    }
}

#[derive(Clone, Debug)]
pub struct PreparedCommit {
    target: Target,
    amend: bool,
    pub draft: CommitDraft,
}

impl PreparedCommit {
    /// The original commit shown by the form; absent for an ordinary unborn commit.
    pub fn head(&self) -> Option<&str> {
        self.target.head.as_deref()
    }

    pub fn validate(&self, root: &Path) -> Result<(), String> {
        if Target::read(root)? != self.target {
            return Err("The repository or HEAD changed; reopen the commit form (your draft is retained)".into());
        }
        Ok(())
    }
}

pub fn prepare_commit(root: &Path, amend: bool) -> Result<PreparedCommit, String> {
    let target = Target::read(root)?;
    let draft = if amend {
        let head = target.head.as_ref().ok_or("Cannot amend: HEAD has no commit yet")?;
        let bytes = run(command(root)?.args(["show", "-s", "--format=%B", head, "--"]))?;
        let message = String::from_utf8(bytes).map_err(|_| "HEAD's message is not UTF-8")?;
        let (subject, body) = message.trim_end().split_once('\n').unwrap_or((message.trim_end(), ""));
        CommitDraft { subject: subject.into(), body: body.trim_start_matches('\n').into(), coauthors: String::new() }
    } else {
        CommitDraft::default()
    };
    let prepared = PreparedCommit { target, amend, draft };
    prepared.validate(root)?;
    Ok(prepared)
}

pub fn commit_draft(root: &Path, prepared: &PreparedCommit, draft: &CommitDraft) -> Result<(), String> {
    commit_draft_as(root, prepared, draft, None)
}

/// Change only this commit's author when the user explicitly supplied a complete identity.
pub fn commit_draft_as(
    root: &Path,
    prepared: &PreparedCommit,
    draft: &CommitDraft,
    author: Option<&CommitAuthor>,
) -> Result<(), String> {
    let (message, trailers) = draft.message_parts()?;
    let author = author.map(CommitAuthor::identity).transpose()?;
    prepared.validate(root)?;
    let message = compose_coauthors(root, &message, &trailers)?;
    prepared.validate(root)?;
    let mut cmd = command(root)?;
    cmd.args(["commit", "--file=-"]);
    if let Some(author) = author {
        cmd.arg("--author").arg(author);
    }
    if prepared.amend {
        cmd.arg("--amend");
    }
    input(&mut cmd, message.as_bytes()).map(|_| ())
}

fn compose_coauthors(root: &Path, message: &str, trailers: &[String]) -> Result<String, String> {
    let config = output(command(root)?.args(["config", "--null", "--get", "trailer.separators"]))?;
    let configured = if config.status.success() {
        String::from_utf8(config.stdout).map_err(|_| "Invalid Git trailer separators")?
    } else if config.status.code() == Some(1) {
        String::new()
    } else {
        return Err(crate::errors::failure(&config));
    };
    // Recognize canonical coauthors as well as native configured separators and aliases.
    let separators = format!(":{}", configured.trim_end_matches('\0'));
    let parse = |text: &str| -> Result<String, String> {
        let parsed = input(
            command(root)?.args(["-c", &format!("trailer.separators={separators}"), "interpret-trailers", "--parse"]),
            text.as_bytes(),
        )?;
        String::from_utf8(parsed).map_err(|_| "Invalid Git trailer output".into())
    };
    let original = parse(message)?;
    if original.is_empty() && trailers.is_empty() {
        return Ok(message.to_owned());
    }
    let lines: Vec<_> = message.split_inclusive('\n').collect();
    let mut offsets = vec![0];
    for line in &lines {
        offsets.push(offsets.last().unwrap() + line.len());
    }
    // Ask Git for its effective comment prefix, including commentString and auto defaults.
    let comment = input(command(root)?.args(["stripspace", "--comment-lines"]), b"ORLO_COMMENT_PREFIX\n")?;
    let comment = String::from_utf8(comment).map_err(|_| "Invalid Git comment prefix")?;
    let comment = comment.strip_suffix(" ORLO_COMMENT_PREFIX\n").ok_or("Invalid Git comment prefix")?;
    let scissors = format!("{comment} ------------------------ >8 ------------------------\n");
    let cutoff = lines
        .iter()
        .position(|line| {
            *line == scissors
                || line
                    .strip_prefix("---")
                    .is_some_and(|tail| tail.is_empty() || tail.as_bytes().first().is_some_and(u8::is_ascii_whitespace))
        })
        .unwrap_or(lines.len());
    // Git also ignores a trailing old-style conflict-note block. Leave its exact delimiter
    // untouched, while still mapping a Conflicts: item followed by ordinary non-ignored prose.
    let mut marker_cutoff = cutoff;
    for index in (0..cutoff).rev() {
        let line = lines[index];
        if line == "Conflicts:\n" {
            marker_cutoff = index;
        } else if line != "\n" && !line.starts_with('\t') && !line.starts_with(comment) {
            break;
        }
    }
    // Draft validation excludes this control character from original message values.
    const MARKER: &str = "\x1eORLO_TRAILER_";
    let mut tagged = String::with_capacity(message.len());
    for (index, line) in lines.iter().enumerate() {
        if index < marker_cutoff && !line.starts_with([' ', '\t']) && line.contains(|c| separators.contains(c)) {
            // Keep the key/value prefix intact: inserting before // would turn a URL into a trailer.
            let content = line.trim_end_matches(['\n', ' ', '\t']);
            tagged.push_str(content);
            tagged.push_str(&format!("{MARKER}{index}:"));
            tagged.push_str(&line[content.len()..]);
        } else {
            tagged.push_str(line);
        }
    }
    let parsed = if original.is_empty() { String::new() } else { parse(&tagged)? };
    let mut mapped = Vec::new();
    for line in parsed.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let Some((before, position)) = value.split_once(MARKER) else { continue };
        let Some((index, after)) = position.split_once(':') else { continue };
        let index: usize = index.parse().map_err(|_| "Invalid Git trailer position")?;
        if index >= marker_cutoff {
            return Err("Invalid Git trailer position".into());
        }
        mapped.push((index, key.to_owned(), format!("{before}{after}")));
    }
    let native: Vec<_> =
        original.lines().filter_map(|line| line.split_once(':')).map(|(k, v)| (k.trim(), v.trim())).collect();
    let recognized: Vec<_> = mapped.iter().map(|(_, k, v)| (k.trim(), v.trim())).collect();
    if native != recognized {
        return Err("Could not map Git trailers without changing the message".into());
    }
    // Recognition comes from unmodified input. Markers only locate those same original spans;
    // neither native parse runs configured trailer commands or reserializes the actual message.
    let mut seen = HashSet::new();
    let mut removed = Vec::new();
    let mut insertion = None;
    for (index, key, value) in mapped {
        let mut end = index + 1;
        while end < cutoff && lines[end].starts_with([' ', '\t']) && !lines[end].trim().is_empty() {
            end += 1;
        }
        insertion = Some(offsets[end]);
        if key.trim().eq_ignore_ascii_case("Co-authored-by") {
            if let Some(identity) = coauthor_key(&value) {
                if !seen.insert(identity) {
                    removed.push(offsets[index]..offsets[end]);
                }
            }
        }
    }
    let added: Vec<_> = trailers
        .iter()
        .filter(|trailer| seen.insert(coauthor_key(trailer.strip_prefix("Co-authored-by: ").unwrap()).unwrap()))
        .collect();
    let footer = insertion.is_some();
    let insertion = insertion.unwrap_or(offsets[cutoff]);
    let mut composed = String::with_capacity(message.len());
    let mut copied = 0;
    for range in &removed {
        composed.push_str(&message[copied..range.start]);
        copied = range.end;
    }
    composed.push_str(&message[copied..]);
    if !added.is_empty() {
        let insertion = insertion - removed.iter().filter(|r| r.end <= insertion).map(|r| r.len()).sum::<usize>();
        let mut addition = String::new();
        if !composed[..insertion].ends_with('\n') {
            addition.push('\n');
        }
        if !footer && !composed[..insertion].ends_with("\n\n") {
            addition.push('\n');
        }
        for trailer in added {
            addition.push_str(trailer);
            addition.push('\n');
        }
        if !footer && insertion < composed.len() && !composed[insertion..].starts_with('\n') {
            addition.push('\n');
        }
        composed.insert_str(insertion, &addition);
    }
    Ok(composed)
}

pub fn commit(root: &Path, message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("Enter a commit message".into());
    }
    if message.contains('\0') {
        return Err("The commit message contains a NUL character".into());
    }
    input(command(root)?.args(["commit", "--file=-"]), message.as_bytes()).map(|_| ())
}
