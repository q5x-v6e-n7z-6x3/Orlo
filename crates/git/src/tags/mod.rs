//! Prepared local tags; no remote or checkout operations.

use crate::operations::{command as git_command, input, run};
use crate::repository::Target;
use std::path::Path;

fn command(root: &Path) -> Result<std::process::Command, String> {
    let mut command = git_command(root)?;
    command.env("GIT_NO_LAZY_FETCH", "1").env("GIT_ALLOW_PROTOCOL", "");
    Ok(command)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub annotated: bool,
    pub target: String,
    pub message: String,
    id: String,
    symbolic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tags {
    target: Target,
    pub entries: Vec<Tag>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTag {
    target: Target,
    commit: String,
}

fn entries(root: &Path) -> Result<Vec<Tag>, String> {
    let bytes = run(command(root)?.args([
        "for-each-ref",
        "--sort=refname",
        "--format=%(refname:lstrip=2)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(symref)%00%(contents)%00",
        "refs/tags/",
    ]))?;
    let mut fields = bytes.split(|b| *b == 0);
    let mut tags = Vec::new();
    while let Some(name) = fields.next() {
        let name = name.strip_prefix(b"\n").unwrap_or(name);
        if name.is_empty() {
            break;
        }
        let id = fields.next().ok_or("Invalid Git tag identifier")?;
        let kind = fields.next().ok_or("Invalid Git tag type")?;
        let peeled = fields.next().ok_or("Invalid Git tag target")?;
        let symbolic = fields.next().ok_or("Invalid Git tag reference")?;
        let message = fields.next().ok_or("Invalid Git tag message")?;
        let annotated = kind == b"tag";
        tags.push(Tag {
            name: String::from_utf8(name.to_vec()).map_err(|_| "A tag name is not valid UTF-8")?,
            id: String::from_utf8(id.to_vec()).map_err(|_| "Invalid Git tag identifier")?,
            annotated,
            target: String::from_utf8(if annotated { peeled } else { id }.to_vec())
                .map_err(|_| "Invalid Git tag target")?,
            message: if annotated { String::from_utf8_lossy(message).into_owned() } else { String::new() },
            symbolic: !symbolic.is_empty(),
        });
    }
    Ok(tags)
}

pub fn prepare_tags(root: &Path) -> Result<Tags, String> {
    let tags = Tags { target: Target::read(root)?, entries: entries(root)? };
    tags.validate(root)?;
    Ok(tags)
}

impl Tags {
    fn validate(&self, root: &Path) -> Result<(), String> {
        validate(root, &self.target)
    }
}

fn validate(root: &Path, target: &Target) -> Result<(), String> {
    if Target::read(root)? != *target {
        return Err("The repository or HEAD changed; reopen Local Tags".into());
    }
    Ok(())
}

/// Resolve once, before opening the name/message form. Creation only uses this commit ID.
pub fn prepare_tag(root: &Path, tags: &Tags, revision: &str) -> Result<PreparedTag, String> {
    tags.validate(root)?;
    if revision.trim().is_empty() || revision.chars().any(char::is_control) {
        return Err("Enter HEAD or a commit revision".into());
    }
    let revision = format!("{revision}^{{commit}}");
    let bytes = run(command(root)?.args(["rev-parse", "--verify", "--end-of-options", &revision]))
        .map_err(|e| format!("Cannot resolve a commit (HEAD may be unborn): {e}"))?;
    let commit = String::from_utf8(bytes).map_err(|_| "Invalid Git commit identifier")?.trim().to_owned();
    tags.validate(root)?;
    Ok(PreparedTag { target: tags.target.clone(), commit })
}

impl PreparedTag {
    pub fn commit(&self) -> &str {
        &self.commit
    }
}

/// Always unsigned: neither tag.gpgSign nor an editor can change the requested tag type.
pub fn create_tag(root: &Path, prepared: &PreparedTag, name: &str, message: &str) -> Result<(), String> {
    validate(root, &prepared.target)?;
    if name.is_empty() || name.starts_with('-') || name == "HEAD" {
        return Err("Enter a valid local tag name".into());
    }
    run(command(root)?.args(["check-ref-format", &format!("refs/tags/{name}")]))
        .map_err(|e| format!("Invalid local tag name: {e}"))?;
    if message.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("The tag message contains a control character".into());
    }
    let mut cmd = command(root)?;
    cmd.args(["-c", "tag.gpgSign=false", "tag", "--no-sign", "--no-edit"]);
    if message.trim().is_empty() {
        run(cmd.args(["--", name, &prepared.commit])).map(|_| ())
    } else {
        input(
            cmd.args(["--annotate", "--cleanup=verbatim", "--file=-", "--", name, &prepared.commit]),
            message.as_bytes(),
        )
        .map(|_| ())
    }
}

/// Caller confirms one selected local tag; compare its raw ref object atomically.
pub fn delete_tag(root: &Path, tags: &Tags, tag: &Tag) -> Result<(), String> {
    tags.validate(root)?;
    if !tags.entries.contains(tag) || !entries(root)?.contains(tag) {
        return Err("The selected tag changed or is missing; reopen Local Tags".into());
    }
    if tag.symbolic {
        return Err("Symbolic tag refs must be managed with terminal Git; no tag deleted".into());
    }
    #[cfg(test)]
    crate::tests::tag_delete_boundary(root, false);
    let removal_error = |e| format!("Could not delete the selected tag; refresh Local Tags: {e}");
    // The same native ref-lock seam as stash removal; unsupported formats refuse safely.
    let repo = git2::Repository::open_ext(root, git2::RepositoryOpenFlags::NO_SEARCH, std::iter::empty::<&Path>())
        .map_err(removal_error)?;
    let reference = format!("refs/tags/{}", tag.name);
    let mut transaction = repo.transaction().map_err(removal_error)?;
    transaction.lock_ref(&reference).map_err(removal_error)?;
    tags.validate(root)?;
    let current = repo.find_reference(&reference).map_err(removal_error)?;
    if current.symbolic_target().is_some() || current.target().is_none_or(|id| id.to_string() != tag.id) {
        return Err("The selected tag changed; no tag deleted. Reopen Local Tags".into());
    }
    #[cfg(test)]
    crate::tests::tag_delete_boundary(root, true);
    transaction.remove(&reference).map_err(removal_error)?;
    transaction.commit().map_err(removal_error)
}
