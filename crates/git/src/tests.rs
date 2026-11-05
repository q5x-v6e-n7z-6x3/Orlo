use crate::operations::path_from;
use crate::status::parse_status;
use std::path::{Path, PathBuf};

use super::git::*;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempRepo(PathBuf);

impl TempRepo {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "orlo-git-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let repo = Self(std::fs::canonicalize(path).unwrap());
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.name", "Orlo test"]);
        repo.git(&["config", "user.email", "orlo@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "core.hooksPath", "/dev/null"]);
        repo
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// A merge into main of a branch, each changing file.txt ("ours" and "theirs"), that stops
    /// at the conflict.
    fn conflicting_merge(&self) {
        self.git(&["switch", "-qc", "other"]);
        self.write("file.txt", "theirs\n");
        self.git(&["commit", "-qam", "theirs"]);
        switch(&self.0, "main").unwrap();
        self.write("file.txt", "ours\n");
        self.git(&["commit", "-qam", "ours"]);
        assert!(!self.run(&["merge", "other"]).status.success());
    }

    fn commit_all(&self, message: &str) {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", message]);
    }

    fn write(&self, path: impl AsRef<Path>, text: impl AsRef<[u8]>) {
        std::fs::write(self.0.join(path), text).unwrap();
    }

    fn read(&self, path: impl AsRef<Path>) -> String {
        std::fs::read_to_string(self.0.join(path)).unwrap()
    }

    fn index(&self, path: impl AsRef<Path>) -> Option<String> {
        index_text(&self.0, path.as_ref()).unwrap()
    }

    fn snapshot(&self) -> Repository {
        discover(&self.0).unwrap().unwrap()
    }

    fn status(&self, path: impl AsRef<Path>) -> FileStatus {
        let path = path.as_ref();
        (self.snapshot().files.into_iter())
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{} has a status", path.display()))
    }

    fn baseline(&self) {
        self.write("file.txt", "base\n");
        self.git(&["add", "file.txt"]);
        self.git(&["commit", "-m", "base"]);
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn selected(old_side: bool, line: usize, whole_hunks: bool) -> StageSelection {
    StageSelection { old_side, lines: line..line + 1, whole_hunks }
}

fn partial(repo: &TempRepo, staged: bool, selection: StageSelection) -> Result<(), String> {
    let file = repo.status("file.txt");
    let shown = versions(&repo.0, &file, staged, 1 << 20)?;
    apply_selection(&repo.0, &file, staged, &shown, &selection)
}

#[test]
fn partial_staging_preserves_worktree_and_other_index_changes() {
    let repo = TempRepo::new();
    repo.write("file.txt", "a\nb\nc\nd\ne\n");
    repo.commit_all("base");
    let working = "a\nB\nc\nD\ne\n";
    repo.write("file.txt", working);
    partial(&repo, false, selected(false, 1, true)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "a\nB\nc\nd\ne\n");
    partial(&repo, false, selected(false, 3, true)).unwrap();
    partial(&repo, true, selected(false, 1, true)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "a\nb\nc\nD\ne\n");
    assert_eq!(repo.read("file.txt"), working);
}

#[test]
fn individual_added_and_deleted_lines_are_independent() {
    let repo = TempRepo::new();
    repo.write("file.txt", "a\nold\nz\n");
    repo.commit_all("base");
    repo.write("file.txt", "a\nnew\nextra\nz\n");
    partial(&repo, false, selected(false, 2, false)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "a\nold\nextra\nz\n");
    partial(&repo, false, selected(true, 1, false)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "a\nextra\nz\n");
    assert_eq!(repo.read("file.txt"), "a\nnew\nextra\nz\n");
}

#[test]
fn selecting_an_added_line_cannot_join_it_to_an_unselected_unterminated_line() {
    let repo = TempRepo::new();
    repo.write("file.txt", "old");
    repo.commit_all("base");
    repo.write("file.txt", "new\nextra");
    assert!(partial(&repo, false, selected(false, 1, false)).is_err());
    assert_eq!(repo.git(&["show", ":file.txt"]), "old");
    partial(&repo, false, selected(false, 1, true)).unwrap();
    assert_eq!(repo.git(&["show", ":file.txt"]), "new\nextra");
}

#[test]
fn partial_staging_handles_new_deleted_crlf_and_unterminated_files() {
    let repo = TempRepo::new();
    repo.write("file.txt", "猫\r\nlast");
    partial(&repo, false, selected(false, 0, false)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "猫\r\n");
    partial(&repo, false, selected(false, 1, true)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "猫\r\nlast");
    partial(&repo, true, selected(false, 0, true)).unwrap();
    assert!(repo.index("file.txt").is_none());
    repo.commit_all("base");
    std::fs::remove_file(repo.0.join("file.txt")).unwrap();
    partial(&repo, false, selected(true, 0, true)).unwrap();
    assert!(repo.index("file.txt").is_none());
    partial(&repo, true, selected(true, 0, true)).unwrap();
    assert_eq!(repo.index("file.txt").unwrap(), "猫\r\nlast");
    assert!(!repo.0.join("file.txt").exists());
}

#[test]
fn stale_partial_staging_refuses_to_change_the_index() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "changed\n");
    let file = repo.snapshot().files.remove(0);
    let shown = versions(&repo.0, &file, false, 100).unwrap();
    repo.write("file.txt", "newer\n");
    assert!(apply_selection(&repo.0, &file, false, &shown, &selected(false, 0, true)).is_err());
    assert_eq!(repo.git(&["show", ":file.txt"]), "base\n");
}

#[test]
fn a_recreated_deleted_file_invalidates_its_partial_staging_snapshot() {
    let repo = TempRepo::new();
    repo.baseline();
    std::fs::remove_file(repo.0.join("file.txt")).unwrap();
    let file = repo.snapshot().files.remove(0);
    let shown = versions(&repo.0, &file, false, 100).unwrap();
    repo.write("file.txt", "recreated\n");
    assert!(apply_selection(&repo.0, &file, false, &shown, &selected(true, 0, true)).is_err());
    assert_eq!(repo.git(&["show", ":file.txt"]), "base\n");
}

#[test]
fn partial_staging_preserves_literal_paths_executable_modes_and_git_cleaning() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TempRepo::new();
    let path = PathBuf::from("odd \"\t\n猫.txt");
    repo.write(&path, "one\ntwo\n");
    std::fs::set_permissions(repo.0.join(&path), std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = repo.snapshot().files.remove(0);
    let shown = versions(&repo.0, &file, false, 100).unwrap();
    apply_selection(&repo.0, &file, false, &shown, &selected(false, 1, false)).unwrap();
    assert_eq!(repo.index(&path).unwrap(), "two\n");
    assert!(repo.git(&["ls-files", "--stage"]).starts_with("100755"));
    repo.write(".gitattributes", "file.txt text eol=lf\n");
    repo.write("file.txt", "one\r\ntwo\r\n");
    repo.commit_all("base");
    repo.write("file.txt", "ONE\r\ntwo\r\n");
    partial(&repo, false, selected(false, 0, true)).unwrap();
    assert_eq!(repo.git(&["show", ":file.txt"]), "ONE\ntwo\n");
    assert_eq!(repo.read("file.txt"), "ONE\r\ntwo\r\n");
}

#[test]
fn partial_staging_rejects_binary_conflicts_and_index_changes() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "shown\n");
    let file = repo.snapshot().files.remove(0);
    let shown = versions(&repo.0, &file, false, 100).unwrap();
    repo.write("file.txt", "new index\n");
    repo.git(&["add", "."]);
    repo.write("file.txt", "shown\n");
    let refused = |file: &FileStatus| apply_selection(&repo.0, file, false, &shown, &selected(false, 0, true)).is_err();
    assert!(refused(&file));
    assert_eq!(repo.git(&["show", ":file.txt"]), "new index\n");
    repo.write("file.txt", b"binary\0data");
    let file = repo.snapshot().files.remove(0);
    assert!(refused(&file));
    assert!(refused(&FileStatus { index: 'U', worktree: 'U', ..file }));
}

#[test]
fn local_branch_management_validates_names_and_protects_unmerged_work() {
    let repo = TempRepo::new();
    repo.baseline();
    create_branch(&repo.0, "topic").unwrap();
    assert_eq!(repo.snapshot().branch, "main");
    rename_branch(&repo.0, "topic", "renamed").unwrap();
    assert!(create_branch(&repo.0, "renamed").is_err());
    assert!(create_branch(&repo.0, "--force").is_err());
    assert!(rename_branch(&repo.0, "renamed", "bad..name").is_err());
    switch(&repo.0, "renamed").unwrap();
    repo.write("file.txt", "unmerged\n");
    repo.git(&["commit", "-am", "unmerged"]);
    assert!(delete_branch(&repo.0, "renamed").is_err());
    switch(&repo.0, "main").unwrap();
    assert!(delete_branch(&repo.0, "renamed").is_err());
    create_branch(&repo.0, "disposable").unwrap();
    delete_branch(&repo.0, "disposable").unwrap();
    assert_eq!(branches(&repo.0).unwrap(), ["main", "renamed"]);
}

#[test]
fn file_history_follows_renames_and_shows_each_commits_changes() {
    let repo = TempRepo::new();
    let new_name = Path::new("new name.txt");
    assert!(file_history(&repo.0, Path::new("file.txt"), 0).unwrap().is_empty());
    repo.baseline();
    repo.git(&["mv", "file.txt", "new name.txt"]);
    repo.git(&["commit", "-m", "rename"]);
    repo.write(new_name, "changed\n");
    repo.git(&["commit", "-am", "change"]);
    let history = file_history(&repo.0, new_name, 0).unwrap();
    let subjects: Vec<_> = history.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["change", "rename", "base"]);
    assert_eq!(history[2].path, Path::new("file.txt"));
    assert!(history[0].date.contains('T'));
    assert_eq!(history[0].author, "Orlo test");
    assert!(commit_diff(&repo.0, &history[2]).unwrap().contains("+base"));
    assert_eq!(file_history(&repo.0, new_name, 2).unwrap(), history[2..]);
}

#[test]
fn file_history_includes_merge_resolution_changes() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.conflicting_merge();
    repo.write("file.txt", "resolved\n");
    repo.commit_all("merge resolution");
    let history = file_history(&repo.0, Path::new("file.txt"), 0).unwrap();
    assert_eq!(history[0].subject, "merge resolution");
    assert!(commit_diff(&repo.0, &history[0]).unwrap().contains("+resolved"));
}

#[test]
fn blame_attributes_each_saved_line_and_marks_uncommitted_changes() {
    let repo = TempRepo::new();
    repo.write("file.txt", "one\ntwo\n");
    repo.commit_all("first");
    let first = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    repo.write("file.txt", "one\nTWO\nnew\n");
    let lines = blame(&repo.0, Path::new("file.txt")).unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].commit, first);
    assert_eq!(lines[0].author, "Orlo test");
    assert!(lines[0].timestamp > 0);
    assert_eq!(lines[0].text, "one");
    assert!(lines[1].commit.chars().all(|c| c == '0'));
    assert_eq!(lines[2].text, "new");
    assert!(blame(&repo.0, Path::new("../outside")).is_err());
}

#[test]
fn initializes_only_an_existing_folder_without_reinitializing_repositories() {
    let repo = TempRepo::new();
    std::fs::remove_dir_all(repo.0.join(".git")).unwrap();
    repo.write("keep.txt", "keep this text\n");
    assert!(discover(&repo.0).unwrap().is_none());
    assert!(init(&repo.0.join("missing")).is_err());
    assert!(!repo.0.join("missing").exists());
    assert!(init(&repo.0.join("keep.txt")).is_err());

    init(&repo.0).unwrap();
    let snapshot = repo.snapshot();
    assert_eq!(snapshot.root, repo.0);
    assert!(snapshot.branch.contains("unborn"));
    assert_eq!(snapshot.files[0].path, Path::new("keep.txt"));
    assert_eq!(repo.read("keep.txt"), "keep this text\n");
    let config = std::fs::read(repo.0.join(".git/config")).unwrap();
    assert!(!String::from_utf8_lossy(&config).contains("[user]"));
    assert!(!String::from_utf8_lossy(&config).contains("[remote "));
    assert!(init(&repo.0).is_err());
    assert_eq!(std::fs::read(repo.0.join(".git/config")).unwrap(), config);

    std::fs::create_dir(repo.0.join("nested")).unwrap();
    assert!(init(&repo.0.join("nested")).is_err());
    assert!(!repo.0.join("nested/.git").exists());
    repo.git(&["init", "--bare", "bare.git"]);
    let bare_head = repo.read("bare.git/HEAD");
    assert!(init(&repo.0.join("bare.git")).is_err());
    assert_eq!(repo.read("bare.git/HEAD"), bare_head);
    assert!(!repo.0.join("bare.git/.git").exists());
}

#[test]
fn discovers_subfolders_active_and_missing_files_and_unborn_branch() {
    let repo = TempRepo::new();
    std::fs::create_dir(repo.0.join("sub")).unwrap();
    repo.write("sub/a b.txt", "new\n");
    for path in ["sub", "sub/a b.txt", "sub/gone.txt"] {
        let snapshot = discover(&repo.0.join(path)).unwrap().unwrap();
        assert_eq!(snapshot.root, repo.0);
        assert!(snapshot.branch.contains("main"));
        assert!(snapshot.branch.contains("unborn"));
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].path, Path::new("sub/a b.txt"));
        assert!(snapshot.files[0].is_untracked());
        assert_eq!(snapshot.staged_count(), 0);
        assert_eq!(snapshot.unstaged_count(), 1);
    }
    assert!(discover(&std::env::temp_dir()).unwrap().is_none());
}

#[test]
fn literal_unusual_paths_stage_and_unstage_without_deleting_files() {
    let repo = TempRepo::new();
    // APFS rejects invalid UTF-8 names; the raw-byte parser is covered separately.
    let names = [":(glob)*", "-option\nname.txt"];
    for name in names {
        repo.write(name, "original\n");
    }
    for name in names {
        stage(&repo.0, &repo.status(name)).unwrap();
        assert_eq!(repo.snapshot().staged_count(), 1);
        assert_eq!(repo.index(name).as_deref(), Some("original\n"));
        repo.write(name, "later edit\n");
        unstage(&repo.0, &repo.status(name)).unwrap();
        assert_eq!(repo.snapshot().staged_count(), 0);
        assert_eq!(repo.read(name), "later edit\n");
    }
}

#[test]
fn status_keeps_raw_non_utf8_newline_and_renamed_path_bytes() {
    let bytes = b"# branch.oid abcdef0123456789\0# branch.head main\x002 R. N... 100644 100644 100644 abcdef abcdef R100 new\n\xff.txt\0old \xfe.txt\0? untracked\n\xfd\0";
    let repo = parse_status(PathBuf::from("/repo"), bytes).unwrap();
    assert_eq!(repo.files[0].path, path_from(b"new\n\xff.txt"));
    assert_eq!(repo.files[0].original_path, Some(path_from(b"old \xfe.txt")));
    assert_eq!(repo.files[1].path, path_from(b"untracked\n\xfd"));
    assert_eq!(repo.files[0].index, 'R');
    assert_eq!(repo.files[0].worktree, ' ');
}

#[test]
fn rename_paths_and_deletions_stage_and_unstage_together() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.git(&["mv", "file.txt", "renamed\nfile.txt"]);
    let renamed = repo.snapshot().files.remove(0);
    assert_eq!(renamed.path, Path::new("renamed\nfile.txt"));
    assert_eq!(renamed.original_path.as_deref(), Some(Path::new("file.txt")));
    assert_eq!(renamed.index, 'R');
    assert!(diff(&repo.0, &renamed, true).unwrap().contains("rename from"));
    repo.write(&renamed.path, "base\nedit\n");
    stage(&repo.0, &renamed).unwrap();
    assert_eq!(repo.index(&renamed.path).as_deref(), Some("base\nedit\n"));
    unstage(&repo.0, &renamed).unwrap();
    assert_eq!(repo.index("file.txt").as_deref(), Some("base\n"));
    assert!(repo.0.join(&renamed.path).exists());
    stage(&repo.0, &repo.status("file.txt")).unwrap();
    assert!(repo.index("file.txt").is_none());
    unstage(&repo.0, &repo.status("file.txt")).unwrap();
    assert!(repo.index("file.txt").is_some());
    assert!(!repo.0.join("file.txt").exists());
}

#[test]
fn commit_uses_only_staged_content_and_rejects_empty_messages() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "staged\n");
    stage(&repo.0, &repo.snapshot().files[0]).unwrap();
    repo.write("file.txt", "unstaged\n");
    repo.write("other.txt", "leave untracked\n");
    let file = repo.status("file.txt");
    assert!(file.has_staged() && file.has_unstaged());
    assert!(diff(&repo.0, &file, false).unwrap().contains("+unstaged"));
    assert!(diff(&repo.0, &file, true).unwrap().contains("+staged"));
    assert!(commit(&repo.0, " \n\t").is_err());
    commit(&repo.0, "only the index").unwrap();
    assert_eq!(repo.git(&["show", "HEAD:file.txt"]), "staged\n");
    assert_eq!(repo.snapshot().staged_count(), 0);
    assert_eq!(repo.snapshot().unstaged_count(), 2);
    assert_eq!(repo.read("file.txt"), "unstaged\n");
}

#[test]
fn unstaging_a_copy_does_not_unstage_its_source() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "source change\n");
    repo.write("copy.txt", "copied content\n");
    repo.git(&["add", "."]);
    let copied = FileStatus {
        path: PathBuf::from("copy.txt"),
        original_path: Some(PathBuf::from("file.txt")),
        index: 'C',
        worktree: ' ',
    };
    assert!(!diff(&repo.0, &copied, true).unwrap().contains("+source change"));
    unstage(&repo.0, &copied).unwrap();
    assert_eq!(repo.index("file.txt").as_deref(), Some("source change\n"));
    assert!(repo.index("copy.txt").is_none());
    assert!(repo.0.join("copy.txt").exists());
}

#[test]
fn index_text_distinguishes_absent_binary_and_unreadable_repositories() {
    let repo = TempRepo::new();
    repo.write("text.txt", "one\r\ntwo\r\n");
    repo.write("binary.dat", b"abc\0def");
    repo.git(&["add", "."]);
    assert_eq!(repo.index("text.txt").as_deref(), Some("one\r\ntwo\r\n"));
    assert!(repo.index("missing").is_none());
    assert!(index_text(&repo.0, Path::new("binary.dat")).unwrap_err().contains("binary"));
    assert!(index_text(&repo.0.join("missing-dir"), Path::new("text.txt")).is_err());
}

#[test]
fn branch_switch_keeps_dirty_files_and_reports_detached_head() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.git(&["switch", "-c", "other"]);
    repo.write("file.txt", "other branch\n");
    repo.git(&["commit", "-am", "other"]);
    switch(&repo.0, "main").unwrap();
    repo.git(&["tag", "main"]);
    assert_eq!(branches(&repo.0).unwrap(), vec!["main", "other"]);
    repo.write("file.txt", "keep dirty\n");
    assert!(switch(&repo.0, "other").is_err());
    assert_eq!(repo.read("file.txt"), "keep dirty\n");
    assert_eq!(repo.snapshot().branch, "main");
    assert!(switch(&repo.0, "--discard-changes").is_err());
    repo.write("file.txt", "base\n");
    repo.git(&["checkout", "--detach"]);
    assert!(repo.snapshot().branch.contains("detached"));
}

#[test]
fn linked_worktree_gitfile_discovers_its_own_root_and_index() {
    let repo = TempRepo::new();
    repo.baseline();
    let linked = repo.0.join("linked");
    repo.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    assert!(linked.join(".git").is_file());
    std::fs::write(linked.join("file.txt"), "linked edit\n").unwrap();
    let snapshot = discover(&linked.join("file.txt")).unwrap().unwrap();
    assert_eq!(snapshot.root, linked);
    assert_eq!(snapshot.branch, "linked");
    assert_eq!(snapshot.unstaged_count(), 1);
    assert_eq!(index_text(&snapshot.root, Path::new("file.txt")).unwrap().as_deref(), Some("base\n"));
}

#[test]
fn mutations_reject_paths_outside_the_repository() {
    let repo = TempRepo::new();
    for path in ["../outside", "/tmp/outside", ""] {
        let file = FileStatus { path: PathBuf::from(path), original_path: None, index: '?', worktree: '?' };
        assert!(stage(&repo.0, &file).is_err());
        assert!(unstage(&repo.0, &file).is_err());
        assert!(index_text(&repo.0, &file.path).is_err());
        assert!(diff(&repo.0, &file, false).is_err());
    }
}

#[test]
fn untracked_diff_shows_only_the_requested_file() {
    let repo = TempRepo::new();
    repo.write(":(glob)*", "new file\n");
    repo.write("other.txt", "other file\n");
    let file = repo.status(":(glob)*");
    let patch = diff(&repo.0, &file, false).unwrap();
    assert!(patch.contains("+new file"));
    assert!(!patch.contains("other file"));
    assert!(diff(&repo.0, &file, true).unwrap().is_empty());
}

#[test]
fn merge_conflict_status_and_marker_guard_preserve_the_worktree() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.conflicting_merge();
    let snapshot = repo.snapshot();
    assert_eq!(snapshot.conflict_count(), 1);
    assert_eq!(snapshot.staged_count(), 0);
    let file = &snapshot.files[0];
    assert!(file.is_conflicted());
    let text = repo.read("file.txt");
    assert!(has_conflict_markers(&text));
    assert!(stage(&repo.0, file).is_err());
    let stale = FileStatus { index: 'M', worktree: ' ', ..file.clone() };
    assert!(stage(&repo.0, &stale).is_err());
    assert_eq!(repo.read("file.txt"), text);
    repo.write("file.txt", resolve_conflict(&text, 0, ConflictChoice::Both).unwrap());
    stage(&repo.0, file).unwrap();
    assert_eq!(repo.snapshot().conflict_count(), 0);
    assert_eq!(repo.index("file.txt").as_deref(), Some("ours\ntheirs\n"));
}

#[test]
fn resolving_one_diff3_conflict_preserves_crlf_and_other_blocks() {
    let first = "<<<<<<< HEAD\r\nours\r\n||||||| base\r\nbase\r\n=======\r\ntheirs\r\n>>>>>>> other\r\n";
    let second = "<<<<<<< HEAD\nsecond ours\n=======\nsecond theirs\n>>>>>>> other\n";
    let text = format!("prefix\r\n{first}middle\r\n{second}suffix");
    assert_eq!(conflicts(&text).len(), 2);
    assert_eq!(
        resolve_conflict(&text, 0, ConflictChoice::Ours).unwrap(),
        format!("prefix\r\nours\r\nmiddle\r\n{second}suffix")
    );
    assert_eq!(
        resolve_conflict(&text, 0, ConflictChoice::Theirs).unwrap(),
        format!("prefix\r\ntheirs\r\nmiddle\r\n{second}suffix")
    );
    assert_eq!(
        resolve_conflict(&text, 1, ConflictChoice::Both).unwrap(),
        format!("prefix\r\n{first}middle\r\nsecond ours\nsecond theirs\nsuffix")
    );
    assert!(resolve_conflict(&text, 2, ConflictChoice::Ours).is_err());
}

#[test]
fn malformed_and_nested_markers_are_never_resolved_as_complete_blocks() {
    for text in [
        "<<<<<<< ours\nunfinished",
        "=======\n",
        ">>>>>>> theirs\n",
        "<<<<<<< ours\nx\n======= bad separator\ny\n>>>>>>> theirs\n",
        "<<<<<<< ours\n<<<<<<< nested\nx\n=======\ny\n>>>>>>> nested\n=======\nz\n>>>>>>> theirs\n",
    ] {
        assert!(has_conflict_markers(text));
        assert!(conflicts(text).is_empty(), "{text:?}");
        assert!(resolve_conflict(text, 0, ConflictChoice::Ours).is_err());
    }
    assert!(!has_conflict_markers("let operator = \"=======\";\n<<<<<< not a marker\n"));
}

/// The two texts as (old, new).
fn texts(repo: &TempRepo, path: &str, staged: bool) -> (String, String) {
    let v = versions(&repo.0, &repo.status(path), staged, 1 << 20).unwrap();
    (v.old, v.new)
}

fn pair(old: &str, new: &str) -> (String, String) {
    (old.into(), new.into())
}

#[test]
fn versions_are_the_texts_each_side_of_the_index_compares() {
    let repo = TempRepo::new();
    repo.baseline();
    // the working tree side: the index's text against the file as saved
    repo.write("file.txt", "base\nedit\n");
    assert_eq!(texts(&repo, "file.txt", false), pair("base\n", "base\nedit\n"));
    // the staged side: HEAD's text against the index's; each side its own
    repo.git(&["add", "file.txt"]);
    repo.write("file.txt", "base\nedit\nmore\n");
    assert_eq!(texts(&repo, "file.txt", true), pair("base\n", "base\nedit\n"));
    assert_eq!(texts(&repo, "file.txt", false), pair("base\nedit\n", "base\nedit\nmore\n"));
    // untracked: nothing against the file; staged new: nothing at HEAD against the index
    repo.write("new.txt", "new\n");
    assert_eq!(texts(&repo, "new.txt", false), pair("", "new\n"));
    repo.git(&["add", "new.txt"]);
    assert_eq!(texts(&repo, "new.txt", true), pair("", "new\n"));
    // deleted in the working tree, then staged as deleted
    repo.write("keep.txt", "one\ntwo\nthree\nfour\n");
    repo.git(&["add", "keep.txt"]);
    repo.git(&["commit", "-qam", "edits"]);
    std::fs::remove_file(repo.0.join("file.txt")).unwrap();
    assert_eq!(texts(&repo, "file.txt", false), pair("base\nedit\nmore\n", ""));
    repo.git(&["rm", "-q", "file.txt"]);
    assert_eq!(texts(&repo, "file.txt", true), pair("base\nedit\nmore\n", ""));
    // a staged rename compares the old path at HEAD with the new one in the index
    repo.git(&["mv", "keep.txt", "kept.txt"]);
    repo.write("kept.txt", "one\ntwo\nthree\nfour\nfive\n");
    repo.git(&["add", "kept.txt"]);
    assert_eq!(repo.status("kept.txt").index, 'R');
    assert_eq!(texts(&repo, "kept.txt", true), pair("one\ntwo\nthree\nfour\n", "one\ntwo\nthree\nfour\nfive\n"));
}

#[test]
fn the_saved_side_is_the_file_as_git_cleans_it_for_the_patch() {
    let repo = TempRepo::new();
    // checked out with CRLF line endings, stored with LF; "$Id$" expanded when checked out
    repo.write(".gitattributes", "*.txt text eol=crlf ident\n");
    repo.write("file.txt", "one\r\ntwo\r\n$Id$\r\nfour\r\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "attributes"]);
    std::fs::remove_file(repo.0.join("file.txt")).unwrap();
    repo.git(&["checkout", "--", "file.txt"]);
    let saved = repo.read("file.txt");
    assert!(saved.contains("\r\n$Id: "), "{saved:?}");
    // one line edited: the patch has one hunk, and so do the sides
    repo.write("file.txt", saved.replace("two", "TWO"));
    let patch = diff(&repo.0, &repo.status("file.txt"), false).unwrap();
    assert_eq!(patch.matches("\n@@ ").count(), 1, "{patch}");
    assert_eq!(texts(&repo, "file.txt", false), pair("one\ntwo\n$Id$\nfour\n", "one\nTWO\n$Id$\nfour\n"));
    // no final line break, and blank lines kept as they are, with or without their space
    repo.write("file.txt", saved.replace("two", "\r\n").trim_end());
    for blank in ["false", "true"] {
        repo.git(&["config", "diff.suppressBlankEmpty", blank]);
        assert_eq!(texts(&repo, "file.txt", false), pair("one\ntwo\n$Id$\nfour\n", "one\n\n\n$Id$\nfour"));
    }
    // a file Git diffs as binary shows as the patch
    repo.write(".gitattributes", "*.txt text eol=crlf ident -diff\n");
    let why = versions(&repo.0, &repo.status("file.txt"), false, 1 << 20).unwrap_err();
    assert!(why.contains("binary"), "{why}");
}

#[test]
fn versions_on_an_unborn_branch_compare_against_nothing() {
    let repo = TempRepo::new();
    repo.write("first.txt", "first\n");
    repo.git(&["add", "first.txt"]);
    assert_eq!(texts(&repo, "first.txt", true), pair("", "first\n"));
    assert!(head_text(&repo.0, Path::new("first.txt")).unwrap().is_none());
}

#[test]
fn versions_refuse_binary_oversized_conflicted_and_non_utf8_files() {
    let repo = TempRepo::new();
    repo.baseline();
    let refused =
        |path: &str, staged: bool, limit: usize| versions(&repo.0, &repo.status(path), staged, limit).unwrap_err();
    repo.write("binary.dat", b"abc\0def");
    assert!(refused("binary.dat", false, 1 << 20).contains("binary"));
    repo.git(&["add", "binary.dat"]);
    assert!(refused("binary.dat", true, 1 << 20).contains("binary"));
    repo.write("latin1.txt", b"caf\xe9\n");
    assert!(refused("latin1.txt", false, 1 << 20).contains("UTF-8"));
    repo.write("file.txt", "base\n".repeat(100));
    assert!(refused("file.txt", false, 100).contains("large"));
    repo.git(&["add", "file.txt"]);
    assert!(refused("file.txt", true, 100).contains("large"));
    // a merge conflict
    repo.git(&["commit", "-qm", "more"]);
    repo.conflicting_merge();
    assert!(refused("file.txt", false, 1 << 20).contains("conflict"));
}

#[test]
fn composition_commits_index_only_and_amends_message_without_staging() {
    let r = TempRepo::new();
    r.baseline();
    r.write("file.txt", "staged\n");
    r.git(&["add", "file.txt"]);
    r.write("file.txt", "unstaged\n");
    let prepared = prepare_commit(&r.0, false).unwrap();
    let draft = CommitDraft {
        subject: "界 subject $(false)".into(),
        body: "line one\nline two 🦀".into(),
        coauthors: "Zoë 界 <zoe@example.invalid>\nDeux <deux@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    assert_eq!(r.git(&["show", "HEAD:file.txt"]), "staged\n");
    assert_eq!(r.read("file.txt"), "unstaged\n");
    let message = r.git(&["log", "-1", "--format=%B"]);
    assert!(message.contains("line one\nline two 🦀\n\nCo-authored-by: Zoë 界 <zoe@example.invalid>\nCo-authored-by: Deux <deux@example.invalid>"));
    let prepared = prepare_commit(&r.0, true).unwrap();
    assert_eq!(prepared.draft.subject, draft.subject);
    assert!(prepared.draft.body.contains("Co-authored-by:"));
    let original = r.git(&["rev-parse", "HEAD"]);
    commit_draft(&r.0, &prepared, &CommitDraft { subject: "amended".into(), ..prepared.draft.clone() }).unwrap();
    assert_ne!(r.git(&["rev-parse", "HEAD"]), original);
    assert_eq!(r.git(&["rev-list", "--count", "HEAD"]).trim(), "2");
    assert_eq!(r.git(&["show", "HEAD:file.txt"]), "staged\n");
}

#[test]
fn composition_amend_keeps_existing_trailers_when_adding_coauthors() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&[
        "commit",
        "--amend",
        "-m",
        "Original subject\n\nKeep the body.\n\nCo-authored-by: Alice <alice@example.invalid>\nSigned-off-by: Maintainer <maintainer@example.invalid>\nReviewed-by: Reviewer\n continued review",
    ]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        coauthors: "Bob <bob@example.invalid>\nZoë 界 <zoe@example.invalid>".into(),
        ..prepared.draft.clone()
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();

    let parsed = r.git(&["log", "-1", "--format=%(trailers:only,unfold)"]);
    for trailer in [
        "Co-authored-by: Alice <alice@example.invalid>",
        "Signed-off-by: Maintainer <maintainer@example.invalid>",
        "Reviewed-by: Reviewer continued review",
        "Co-authored-by: Bob <bob@example.invalid>",
        "Co-authored-by: Zoë 界 <zoe@example.invalid>",
    ] {
        assert!(parsed.lines().any(|line| line == trailer), "missing parsed trailer {trailer:?}: {parsed:?}");
    }
    assert!(r.git(&["log", "-1", "--format=%B"]).starts_with("Original subject\n\nKeep the body.\n\n"));
    assert_eq!(r.git(&["rev-list", "--count", "HEAD"]).trim(), "1");
}

#[test]
fn composition_rejects_unborn_stale_targets_and_invalid_input() {
    let r = TempRepo::new();
    assert!(prepare_commit(&r.0, true).unwrap_err().contains("HEAD"));
    let prepared = prepare_commit(&r.0, false).unwrap();
    let draft = CommitDraft { subject: "  ".into(), ..Default::default() };
    assert!(commit_draft(&r.0, &prepared, &draft).unwrap_err().contains("subject"));
    for (subject, body, coauthors) in [
        ("subject\ninjected", "", ""),
        ("subject", "nul\0body", ""),
        ("subject", "", "Name\r <a@b>"),
        ("subject", "", "Name <a@b> extra"),
        ("subject", "", " <a@b>"),
        ("subject", "", "Name <bad>"),
        ("subject", "", "Name <a@b>\nSigned-off-by: Inject <c@d>"),
    ] {
        assert!(CommitDraft { subject: subject.into(), body: body.into(), coauthors: coauthors.into() }
            .validate()
            .is_err());
    }
    r.baseline();
    assert!(commit_draft(&r.0, &prepared, &CommitDraft { subject: "stale".into(), ..Default::default() })
        .unwrap_err()
        .contains("changed"));
    let prepared = prepare_commit(&r.0, true).unwrap();
    r.git(&["switch", "-qc", "same-object"]);
    assert!(commit_draft(&r.0, &prepared, &CommitDraft { subject: "stale".into(), ..Default::default() })
        .unwrap_err()
        .contains("changed"));
    let prepared = prepare_commit(&r.0, true).unwrap();
    r.write("next", "next");
    r.commit_all("next");
    assert!(commit_draft(&r.0, &prepared, &CommitDraft { subject: "stale".into(), ..Default::default() })
        .unwrap_err()
        .contains("changed"));
}

#[test]
fn composition_preserves_hook_and_signing_failures() {
    use std::os::unix::fs::PermissionsExt;
    let r = TempRepo::new();
    r.baseline();
    let hooks = r.0.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'fixture hook refused' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    r.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft { subject: "retained".into(), ..Default::default() };
    let before = r.git(&["rev-parse", "HEAD"]);
    assert!(commit_draft(&r.0, &prepared, &draft).unwrap_err().contains("fixture hook refused"));
    r.git(&["config", "core.hooksPath", "/dev/null"]);
    r.git(&["config", "commit.gpgsign", "true"]);
    r.git(&["config", "gpg.program", "/usr/bin/false"]);
    assert!(commit_draft(&r.0, &prepared, &draft).is_err());
    assert_eq!(r.git(&["rev-parse", "HEAD"]), before);
    r.git(&["config", "commit.gpgsign", "false"]);
    assert!(commit(&r.0, "ordinary empty index").is_err());
    let ordinary = prepare_commit(&r.0, false).unwrap();
    assert!(commit_draft(&r.0, &ordinary, &draft).is_err());
}

#[test]
fn composition_rejects_controls_in_blank_coauthor_lines_and_replaced_repository() {
    for coauthors in ["\r", "\t\n", "Name\u{2028}Injected <a@b>", "Name <a@b>\n\0"] {
        assert!(
            CommitDraft { subject: "subject".into(), coauthors: coauthors.into(), ..Default::default() }
                .validate()
                .is_err(),
            "accepted {coauthors:?}"
        );
    }
    let r = TempRepo::new();
    r.baseline();
    let prepared = prepare_commit(&r.0, true).unwrap();
    std::fs::rename(r.0.join(".git"), r.0.join("original-git")).unwrap();
    r.git(&["init", "--initial-branch=main"]);
    assert!(prepared.validate(&r.0).unwrap_err().contains("changed"));
}

#[test]
fn composition_large_body_and_noisy_hook_do_not_deadlock() {
    use std::os::unix::fs::PermissionsExt;
    let r = TempRepo::new();
    r.baseline();
    let hook = r.0.join(".git/hooks/pre-commit");
    std::fs::write(r.0.join(".git/hook-output"), vec![b'x'; 256 * 1024]).unwrap();
    std::fs::write(&hook, "#!/bin/sh\ncat .git/hook-output\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    r.git(&["config", "core.hooksPath", ".git/hooks"]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft =
        CommitDraft { subject: "large body".into(), body: "body 界\n".repeat(64 * 1024), ..Default::default() };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    assert_eq!(prepare_commit(&r.0, true).unwrap().draft.body, draft.body.trim_end());
}

#[test]
fn composition_deduplicates_separated_folded_and_field_coauthors_without_rewriting_other_text() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    let body = "Literal example:\nCo-authored-by: Zoë 界 <zoe@example.invalid>\n\nKeep  this prose 🦀.\n\nCo-authored-by: Zoë 界 <zoe@example.invalid>\nSigned-off-by: Maintainer <m@example.invalid>\nReviewed-by: Reviewer\n continued review\nco-authored-by: Zoë  界\n <zoe@example.invalid>";
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Unicode 界".into(),
        body: body.into(),
        coauthors: "Zoë 界 <zoe@example.invalid>\nDeux <deux@example.invalid>\nZoë  界 <zoe@example.invalid>\nDeux <deux@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    let parsed = r.git(&["log", "-1", "--format=%(trailers:only,unfold)"]);
    assert_eq!(parsed.lines().filter(|s| s.contains("<zoe@example.invalid>")).count(), 1, "{parsed}");
    assert_eq!(parsed.lines().filter(|s| s.contains("<deux@example.invalid>")).count(), 1, "{parsed}");
    let message = r.git(&["log", "-1", "--format=%B"]);
    assert!(message.starts_with(
        "Unicode 界\n\nLiteral example:\nCo-authored-by: Zoë 界 <zoe@example.invalid>\n\nKeep  this prose 🦀.\n\n"
    ));
    assert!(
        message.contains("Signed-off-by: Maintainer <m@example.invalid>\nReviewed-by: Reviewer\n continued review\n")
    );

    // A manual duplicate in the editable body is removed even without new field input.
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft { coauthors: String::new(), ..draft };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    let message = r.git(&["log", "-1", "--format=%B"]);
    assert!(!message.contains("co-authored-by: Zoë  界"));
    assert!(message.contains("Reviewed-by: Reviewer\n continued review"));
}

#[test]
fn composition_deduplicates_native_separator_aliases_without_running_trailer_commands_during_parse() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "trailer.separators", "%"]);
    r.git(&["config", "trailer.co.key", "Co-authored-by"]);
    r.git(&["config", "trailer.co.ifExists", "add"]);
    r.git(&[
        "config",
        "trailer.co.cmd",
        "echo called >> trailer-command-calls; printf 'Policy <policy@example.invalid>'",
    ]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Configured trailers".into(),
        body: "Keep: this body\n\nco% Zoë <zoe@example.invalid>\nSigned-off-by% Maintainer <m@example.invalid>\nCo-authored-by% Zoë <zoe@example.invalid>".into(),
        coauthors: "Zoë <zoe@example.invalid>\nDeux <deux@example.invalid>\nDeux <deux@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    assert!(!r.0.join("trailer-command-calls").exists(), "composition must not run trailer commands");
    let message = r.git(&["log", "-1", "--format=%B"]);
    assert_eq!(message.lines().filter(|s| s.contains("<zoe@example.invalid>")).count(), 1, "{message}");
    assert_eq!(message.lines().filter(|s| s.contains("<deux@example.invalid>")).count(), 1, "{message}");
    assert!(message.contains("Co-authored-by: Deux <deux@example.invalid>"), "{message}");
    assert!(message.contains("Signed-off-by% Maintainer <m@example.invalid>"), "{message}");
    assert!(message.starts_with("Configured trailers\n\nKeep: this body\n\n"));
}

#[test]
fn composition_amend_preserves_previous_author_with_and_without_staged_changes() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&[
        "commit",
        "--amend",
        "--author=Original 界 <original@example.invalid>",
        "--date=2001-02-03T04:05:06Z",
        "--no-edit",
    ]);
    let original = r.git(&["show", "-s", "--format=%an%x00%ae%x00%at"]);
    r.git(&["config", "user.name", "Different committer"]);
    r.git(&["config", "user.email", "different@example.invalid"]);
    for staged in [false, true] {
        if staged {
            r.write("file.txt", "staged 界\n");
            r.git(&["add", "file.txt"]);
        }
        r.write("file.txt", "unstaged 🦀\n");
        let prepared = prepare_commit(&r.0, true).unwrap();
        let draft = CommitDraft { subject: format!("Amend {staged}"), ..prepared.draft.clone() };
        commit_draft(&r.0, &prepared, &draft).unwrap();
        assert_eq!(r.git(&["show", "-s", "--format=%an%x00%ae%x00%at"]), original);
        assert_eq!(
            r.git(&["show", "-s", "--format=%cn <%ce>"]).trim(),
            "Different committer <different@example.invalid>"
        );
        assert_eq!(r.git(&["show", "HEAD:file.txt"]), if staged { "staged 界\n" } else { "base\n" });
        assert_eq!(r.read("file.txt"), "unstaged 🦀\n");
        assert_eq!(r.git(&["rev-list", "--count", "HEAD"]).trim(), "1");
    }
}

#[test]
fn composition_explicit_author_validates_before_git_and_never_changes_configured_identity() {
    use std::os::unix::fs::PermissionsExt;
    let r = TempRepo::new();
    r.baseline();
    let hook = r.0.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho called > hook-called\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    r.git(&["config", "core.hooksPath", ".git/hooks"]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft { subject: "Explicit author".into(), ..Default::default() };
    let before = r.git(&["rev-parse", "HEAD"]);
    for (name, email) in [
        ("", "author@example.invalid"),
        ("Author", ""),
        ("Author\nInjected", "author@example.invalid"),
        ("Author\n", "author@example.invalid"),
        ("Author\t", "author@example.invalid"),
        ("Author\u{2028}Injected", "author@example.invalid"),
        ("Author\u{2028}", "author@example.invalid"),
        ("Author <Injected>", "author@example.invalid"),
        ("Author", "author@@example.invalid"),
        ("Author", "author@ example.invalid"),
        ("Author", "author@example.invalid "),
        ("Author", "author\0@example.invalid"),
    ] {
        let author = CommitAuthor { name: name.into(), email: email.into() };
        assert!(commit_draft_as(&r.0, &prepared, &draft, Some(&author)).unwrap_err().contains("author"));
        assert!(!r.0.join("hook-called").exists());
        assert_eq!(r.git(&["rev-parse", "HEAD"]), before);
    }
    let author = CommitAuthor { name: "Zoë 界: $(false)".into(), email: "zoe@example.invalid".into() };
    commit_draft_as(&r.0, &prepared, &draft, Some(&author)).unwrap();
    assert_eq!(r.git(&["show", "-s", "--format=%an <%ae>"]).trim(), "Zoë 界: $(false) <zoe@example.invalid>");
    assert_eq!(r.git(&["show", "-s", "--format=%cn <%ce>"]).trim(), "Orlo test <orlo@example.invalid>");
    assert_eq!(r.git(&["config", "user.name"]).trim(), "Orlo test");
    assert_eq!(r.git(&["config", "user.email"]).trim(), "orlo@example.invalid");
    assert_eq!(r.read("hook-called"), "called\n");
}

#[test]
fn composition_native_hook_and_signing_failures_preserve_index_worktree_and_head() {
    use std::os::unix::fs::PermissionsExt;
    for amend in [false, true] {
        for signing in [false, true] {
            let r = TempRepo::new();
            r.baseline();
            r.write("file.txt", "staged 界\n");
            r.git(&["add", "file.txt"]);
            r.write("file.txt", "unstaged 🦀\n");
            r.write("untracked.txt", "keep untracked\n");
            let fixture = r.0.join(".git/failing-policy");
            std::fs::write(&fixture, "#!/bin/sh\necho 'fixture policy refused 界' >&2\nexit 1\n").unwrap();
            std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
            if signing {
                r.git(&["config", "commit.gpgsign", "true"]);
                r.git(&["config", "gpg.format", "openpgp"]);
                r.git(&["config", "gpg.program", fixture.to_str().unwrap()]);
                r.git(&["config", "user.signingkey", "fixture-key"]);
            } else {
                std::fs::rename(&fixture, r.0.join(".git/hooks/pre-commit")).unwrap();
                r.git(&["config", "core.hooksPath", ".git/hooks"]);
            }
            let before_head = r.git(&["rev-parse", "HEAD"]);
            let before_index = r.git(&["ls-files", "--stage"]);
            let before_staged = r.git(&["diff", "--cached", "--binary"]);
            let before_unstaged = r.git(&["diff", "--binary"]);
            let prepared = prepare_commit(&r.0, amend).unwrap();
            let draft = CommitDraft {
                subject: "Failure stays recoverable 界".into(),
                body: "Keep\nfull body".into(),
                coauthors: "Zoë <zoe@example.invalid>".into(),
            };
            let author = CommitAuthor { name: "Explicit Author".into(), email: "explicit@example.invalid".into() };
            let error = commit_draft_as(&r.0, &prepared, &draft, Some(&author)).unwrap_err();
            assert!(error.contains("fixture policy refused"), "{error}");
            assert_eq!(r.git(&["rev-parse", "HEAD"]), before_head);
            assert_eq!(r.git(&["ls-files", "--stage"]), before_index);
            assert_eq!(r.git(&["diff", "--cached", "--binary"]), before_staged);
            assert_eq!(r.git(&["diff", "--binary"]), before_unstaged);
            assert_eq!(r.index("file.txt").unwrap(), "staged 界\n");
            assert_eq!(r.read("file.txt"), "unstaged 🦀\n");
            assert_eq!(r.read("untracked.txt"), "keep untracked\n");
        }
    }
}

#[test]
fn composition_canonical_coauthors_bypass_configured_commands_and_preserve_divider_and_prose() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    r.git(&["config", "trailer.co.key", "Co-authored-by"]);
    r.git(&["config", "trailer.co.cmd", "echo called > trailer-command-called; echo changed"]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Subject: keep colon".into(),
        body: "https://example.invalid/path\n\nSigned-off-by: Maintainer <m@example.invalid>\n\n---\nLiteral patch Co-authored-by: Zoë <zoe@example.invalid>\n+keep: text".into(),
        coauthors: "Zoë <zoe@example.invalid>\nOther <other@example.invalid>\nZoë <zoe@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    assert!(!r.0.join("trailer-command-called").exists());
    let message = r.git(&["show", "-s", "--format=%B"]);
    assert!(message.starts_with("Subject: keep colon\n\nhttps://example.invalid/path\n\nSigned-off-by: Maintainer <m@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>\nCo-authored-by: Other <other@example.invalid>\n\n---\n"), "{message}");
    assert!(message.contains("Literal patch Co-authored-by: Zoë <zoe@example.invalid>\n+keep: text"));
}

#[test]
fn composition_duplicate_removal_preserves_whitespace_separator_body_and_patch_exactly() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    for add_coauthor in [false, true] {
        let prepared = prepare_commit(&r.0, true).unwrap();
        let draft = CommitDraft {
            subject: "Keep exact message 界".into(),
            body: "Keep  this prose.\n\nCo-authored-by: Zoë <zoe@example.invalid>\nco-authored-by: Zoë\n <zoe@example.invalid>\n \t \n---\nLiteral Co-authored-by: Zoë <zoe@example.invalid>\n+patch: 🦀".into(),
            coauthors: if add_coauthor { "Other <other@example.invalid>".into() } else { String::new() },
        };
        commit_draft(&r.0, &prepared, &draft).unwrap();
        let addition = if add_coauthor { "Co-authored-by: Other <other@example.invalid>\n" } else { "" };
        let expected = format!("Keep exact message 界\n\nKeep  this prose.\n\nCo-authored-by: Zoë <zoe@example.invalid>\n{addition} \t \n---\nLiteral Co-authored-by: Zoë <zoe@example.invalid>\n+patch: 🦀");
        let message = r.git(&["show", "-s", "--format=%B"]);
        assert_eq!(message.trim_end_matches('\n'), expected);
    }
}

#[test]
fn composition_url_body_examples_remain_literal_without_native_footer() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    let expected = "URL prose 界\n\nhttps://example.invalid/path\nCo-authored-by: Zoë <zoe@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>";
    let before = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        expected.as_bytes(),
    )
    .unwrap();
    assert!(before.is_empty(), "literal URL paragraph is not a native footer");
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "URL prose 界".into(),
        body: "https://example.invalid/path\nCo-authored-by: Zoë <zoe@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>".into(),
        ..Default::default()
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    assert_eq!(r.git(&["show", "-s", "--format=%B"]).trim_end_matches('\n'), expected);
}

#[test]
fn composition_url_only_body_adds_a_separate_native_visible_footer() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "URL only 界".into(),
        body: "https://example.invalid/path".into(),
        coauthors: "Zoë <zoe@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    let message = r.git(&["show", "-s", "--format=%B"]);
    let parsed = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        message.as_bytes(),
    )
    .unwrap();
    assert_eq!(String::from_utf8(parsed).unwrap(), "Co-authored-by: Zoë <zoe@example.invalid>\n");
    assert_eq!(
        message.trim_end_matches('\n'),
        "URL only 界\n\nhttps://example.invalid/path\n\nCo-authored-by: Zoë <zoe@example.invalid>"
    );
}

#[test]
fn composition_configured_scissors_keep_literal_ignored_coauthors() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    r.git(&["config", "core.commentChar", ":"]);
    let expected = "Cutoff 界\n\nBody 🦀\n\n: ------------------------ >8 ------------------------\nCo-authored-by: Zoë <zoe@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>";
    let parsed = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        expected.as_bytes(),
    )
    .unwrap();
    assert!(parsed.is_empty(), "native scissors exclude the literal examples");
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Cutoff 界".into(),
        body: "Body 🦀\n\n: ------------------------ >8 ------------------------\nCo-authored-by: Zoë <zoe@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>".into(),
        ..Default::default()
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    assert_eq!(r.git(&["show", "-s", "--format=%B"]).trim_end_matches('\n'), expected);
}

#[test]
fn composition_native_cutoff_scissors_receive_coauthors_before_ignored_tail() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Cut 界".into(),
        body: "Body 🦀\n\n# ------------------------ >8 ------------------------\nignored text".into(),
        coauthors: "Zoë <zoe@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    let message = r.git(&["show", "-s", "--format=%B"]);
    let parsed = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        message.as_bytes(),
    )
    .unwrap();
    assert_eq!(String::from_utf8(parsed).unwrap(), "Co-authored-by: Zoë <zoe@example.invalid>\n");
    assert_eq!(message.trim_end_matches('\n'), "Cut 界\n\nBody 🦀\n\nCo-authored-by: Zoë <zoe@example.invalid>\n\n# ------------------------ >8 ------------------------\nignored text");
}

#[test]
fn composition_native_cutoff_tab_divider_receives_coauthors_before_patch() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Patch 界".into(),
        body: "Body 🦀\n\n---\tpatch text\n+ literal Co-authored-by: Tail <tail@example.invalid>".into(),
        coauthors: "Zoë <zoe@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    let message = r.git(&["show", "-s", "--format=%B"]);
    let parsed = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        message.as_bytes(),
    )
    .unwrap();
    assert_eq!(String::from_utf8(parsed).unwrap(), "Co-authored-by: Zoë <zoe@example.invalid>\n");
    assert_eq!(message.trim_end_matches('\n'), "Patch 界\n\nBody 🦀\n\nCo-authored-by: Zoë <zoe@example.invalid>\n\n---\tpatch text\n+ literal Co-authored-by: Tail <tail@example.invalid>");
}

#[test]
fn composition_native_cutoff_configured_scissors_preserve_real_footer_and_ignored_examples() {
    for (key, setting, prefix) in [
        ("core.commentChar", ":", ":"),
        ("core.commentString", "//", "//"),
        ("core.commentString", "界", "界"),
        ("core.commentChar", "auto", "#"),
    ] {
        let r = TempRepo::new();
        r.baseline();
        r.git(&["config", "commit.cleanup", "verbatim"]);
        r.git(&["config", key, setting]);
        let tail = format!("{prefix} ------------------------ >8 ------------------------\nCo-authored-by: Tail <tail@example.invalid>\nCo-authored-by: Tail <tail@example.invalid>");
        let prepared = prepare_commit(&r.0, true).unwrap();
        let draft = CommitDraft {
            subject: "Configured cutoff 界".into(),
            body: format!("Body 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\n\n{tail}"),
            coauthors: "Zoë <zoe@example.invalid>".into(),
        };
        commit_draft(&r.0, &prepared, &draft).unwrap();
        let message = r.git(&["show", "-s", "--format=%B"]);
        let parsed = crate::operations::input(
            crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
            message.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(parsed).unwrap(),
            "Signed-off-by: Maintainer <m@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>\n"
        );
        assert_eq!(message.trim_end_matches('\n'), format!("Configured cutoff 界\n\nBody 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\nCo-authored-by: Zoë <zoe@example.invalid>\n\n{tail}"));
    }
}

#[test]
fn composition_legacy_conflicts_tail_keeps_valid_commits_and_native_coauthors() {
    for amend in [false, true] {
        for assisted in [false, true] {
            let r = TempRepo::new();
            r.baseline();
            r.git(&["config", "commit.cleanup", "verbatim"]);
            if !amend {
                r.write("file.txt", "staged 界\n");
                r.git(&["add", "file.txt"]);
            }
            let original = "Legacy notes 界\n\nBody 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\n\nConflicts:\n\t界 file.txt\n\tpath:other";
            let before = crate::operations::input(
                crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
                original.as_bytes(),
            )
            .unwrap();
            assert_eq!(String::from_utf8(before).unwrap(), "Signed-off-by: Maintainer <m@example.invalid>\n");
            let prepared = prepare_commit(&r.0, amend).unwrap();
            let draft = CommitDraft {
                subject: "Legacy notes 界".into(),
                body: "Body 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\n\nConflicts:\n\t界 file.txt\n\tpath:other".into(),
                coauthors: if assisted { "Zoë <zoe@example.invalid>".into() } else { String::new() },
            };
            let result = commit_draft(&r.0, &prepared, &draft);
            assert!(result.is_ok(), "valid legacy-note message must commit: {result:?}");
            let message = r.git(&["show", "-s", "--format=%B"]);
            let addition = if assisted { "Co-authored-by: Zoë <zoe@example.invalid>\n" } else { "" };
            assert_eq!(message.trim_end_matches('\n'), format!("Legacy notes 界\n\nBody 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\n{addition}\nConflicts:\n\t界 file.txt\n\tpath:other"));
            let parsed = crate::operations::input(
                crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
                message.as_bytes(),
            )
            .unwrap();
            assert_eq!(
                String::from_utf8(parsed).unwrap(),
                format!("Signed-off-by: Maintainer <m@example.invalid>\n{addition}")
            );
            assert_eq!(r.read("file.txt"), if amend { "base\n" } else { "staged 界\n" });
        }
    }
}

#[test]
fn composition_conflicts_header_remains_metadata_when_its_following_prose_is_not_ignored() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "commit.cleanup", "verbatim"]);
    let original =
        "Literal metadata 界\n\nBody 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\nConflicts:\nKeep this prose";
    let before = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        original.as_bytes(),
    )
    .unwrap();
    assert_eq!(String::from_utf8(before).unwrap(), "Signed-off-by: Maintainer <m@example.invalid>\nConflicts: \n");
    let prepared = prepare_commit(&r.0, true).unwrap();
    let draft = CommitDraft {
        subject: "Literal metadata 界".into(),
        body: "Body 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\nConflicts:\nKeep this prose".into(),
        coauthors: "Zoë <zoe@example.invalid>".into(),
    };
    commit_draft(&r.0, &prepared, &draft).unwrap();
    let message = r.git(&["show", "-s", "--format=%B"]);
    assert_eq!(message.trim_end_matches('\n'), "Literal metadata 界\n\nBody 🦀\n\nSigned-off-by: Maintainer <m@example.invalid>\nConflicts:\nCo-authored-by: Zoë <zoe@example.invalid>\nKeep this prose");
    let parsed = crate::operations::input(
        crate::operations::command(&r.0).unwrap().args(["interpret-trailers", "--parse"]),
        message.as_bytes(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(parsed).unwrap(),
        "Signed-off-by: Maintainer <m@example.invalid>\nConflicts: \nCo-authored-by: Zoë <zoe@example.invalid>\n"
    );
}

#[test]
fn stash_restores_unicode_partial_index_and_optional_untracked_files() {
    let repo = TempRepo::new();
    repo.write("界.txt", "base\n");
    repo.commit_all("base");
    repo.write("界.txt", "staged\n");
    repo.git(&["add", "."]);
    repo.write("界.txt", "working\n");
    repo.write("new 🦀.txt", "untracked\n");
    let before = prepare_stashes(&repo.0).unwrap();
    create_stash(&repo.0, &before, "draft 界", false).unwrap();
    assert_eq!(repo.read("界.txt"), "base\n");
    assert_eq!(repo.read("new 🦀.txt"), "untracked\n");
    let stashes = prepare_stashes(&repo.0).unwrap();
    assert_eq!(stashes.entries.len(), 1);
    assert!(stashes.entries[0].subject.contains("draft 界"));
    assert!(stash_preview(&repo.0, &stashes, &stashes.entries[0]).unwrap().contains("+working"));
    apply_stash(&repo.0, &stashes, &stashes.entries[0], true, true).unwrap();
    assert_eq!(repo.index("界.txt").unwrap(), "staged\n");
    assert_eq!(repo.read("界.txt"), "working\n");
    assert!(prepare_stashes(&repo.0).unwrap().entries.is_empty());
    let stashes = prepare_stashes(&repo.0).unwrap();
    create_stash(&repo.0, &stashes, "all", true).unwrap();
    assert!(!repo.0.join("new 🦀.txt").exists());
    let stashes = prepare_stashes(&repo.0).unwrap();
    assert!(stash_preview(&repo.0, &stashes, &stashes.entries[0]).unwrap().contains("+untracked"));
    apply_stash(&repo.0, &stashes, &stashes.entries[0], false, false).unwrap();
    assert_eq!(repo.index("界.txt").unwrap(), "base\n");
    assert_eq!(repo.read("new 🦀.txt"), "untracked\n");
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries.len(), 1);
}

#[test]
fn stash_stale_and_missing_entries_never_apply_or_drop_a_new_stack() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "first\n");
    repo.git(&["stash", "push", "-m", "first"]);
    let stale = prepare_stashes(&repo.0).unwrap();
    repo.write("file.txt", "second\n");
    repo.git(&["stash", "push", "-m", "second"]);
    for pop in [false, true] {
        assert!(apply_stash(&repo.0, &stale, &stale.entries[0], false, pop).unwrap_err().contains("changed"));
    }
    assert!(drop_stash(&repo.0, &stale, &stale.entries[0]).unwrap_err().contains("changed"));
    assert!(stash_preview(&repo.0, &stale, &stale.entries[0]).is_err());
    assert_eq!(repo.read("file.txt"), "base\n");
    let current = prepare_stashes(&repo.0).unwrap();
    assert_eq!(current.entries.len(), 2);
    drop_stash(&repo.0, &current, &current.entries[1]).unwrap();
    let current = prepare_stashes(&repo.0).unwrap();
    repo.git(&["stash", "clear"]);
    assert!(drop_stash(&repo.0, &current, &current.entries[0]).is_err());
}

#[test]
fn stash_conflicted_pop_keeps_stash_and_exposes_git_error() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "stashed\n");
    repo.git(&["stash", "push"]);
    repo.write("file.txt", "new head\n");
    repo.commit_all("new head");
    let stashes = prepare_stashes(&repo.0).unwrap();
    let error = apply_stash(&repo.0, &stashes, &stashes.entries[0], false, true).unwrap_err();
    assert!(error.contains("CONFLICT") || error.contains("conflict"), "{error}");
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries, stashes.entries);
    assert!(repo.read("file.txt").contains("<<<<<<<"));
}

#[test]
fn stash_preview_is_bounded_and_binary_safe() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("binary.bin", b"a\0b");
    repo.write("large.txt", "0123456789\n".repeat(250_000));
    repo.git(&["stash", "push", "-u"]);
    let stashes = prepare_stashes(&repo.0).unwrap();
    let preview = stash_preview(&repo.0, &stashes, &stashes.entries[0]).unwrap();
    assert!(preview.contains("Binary files"));
    assert!(preview.contains("truncated"));
    assert!(preview.len() < (2 << 20) + 1024);
}

#[test]
fn stash_context_refuses_head_or_repository_replacement_and_empty_creation() {
    let repo = TempRepo::new();
    repo.baseline();
    let before = prepare_stashes(&repo.0).unwrap();
    assert!(create_stash(&repo.0, &before, "", false).unwrap_err().contains("No"));
    repo.write("file.txt", "changed\n");
    repo.commit_all("advance");
    assert!(create_stash(&repo.0, &before, "", false).unwrap_err().contains("repository"));
    let current = prepare_stashes(&repo.0).unwrap();
    std::fs::rename(repo.0.join(".git"), repo.0.join("old-git")).unwrap();
    repo.git(&["init", "-q"]);
    assert!(create_stash(&repo.0, &current, "", false).is_err());
}

#[test]
fn stash_creation_preserves_literal_looking_untracked_names() {
    let repo = TempRepo::new();
    repo.baseline();
    let names = ["--index", "*?[界.txt", ":(literal)name", "line\nbreak"];
    for name in names {
        repo.write(name, format!("content of {name}"));
    }
    let before = prepare_stashes(&repo.0).unwrap();
    create_stash(&repo.0, &before, "--not-an-option", true).unwrap();
    for name in names {
        assert!(!repo.0.join(name).exists(), "{name}");
    }
    let stashes = prepare_stashes(&repo.0).unwrap();
    apply_stash(&repo.0, &stashes, &stashes.entries[0], false, true).unwrap();
    for name in names {
        assert_eq!(repo.read(name), format!("content of {name}"));
    }
}

type StashBoundary = (bool, Box<dyn FnOnce(&Path)>);

thread_local! {
    static STASH_DROP_BOUNDARY: std::cell::RefCell<Option<StashBoundary>> = Default::default();
}

pub(crate) fn stash_drop_boundary(root: &Path, locked: bool) {
    let callback = STASH_DROP_BOUNDARY.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook.as_ref().is_some_and(|(phase, _)| *phase == locked) {
            hook.take()
        } else {
            None
        }
    });
    if let Some((_, callback)) = callback {
        callback(root);
    }
}

fn terminal_push_at_drop_boundary(pop: bool) {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "selected A\n");
    repo.git(&["stash", "push", "-m", "selected A"]);
    let selected = prepare_stashes(&repo.0).unwrap();
    STASH_DROP_BOUNDARY.with(|hook| {
        *hook.borrow_mut() = Some((
            false,
            Box::new(|root| {
                std::fs::write(root.join("file.txt"), "unconfirmed B\n").unwrap();
                let pushed = Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["stash", "push", "-m", "unconfirmed B"])
                    .output()
                    .unwrap();
                assert!(pushed.status.success(), "{}", String::from_utf8_lossy(&pushed.stderr));
            }),
        ))
    });
    let result = if pop {
        apply_stash(&repo.0, &selected, &selected.entries[0], false, true)
    } else {
        drop_stash(&repo.0, &selected, &selected.entries[0])
    };
    let after = prepare_stashes(&repo.0).unwrap();
    assert_eq!(after.entries.len(), 2, "unconfirmed terminal stash must never be deleted: {result:?}");
    assert!(after.entries[0].subject.contains("unconfirmed B"));
    assert_eq!(after.entries[1].id, selected.entries[0].id);
    assert!(result.unwrap_err().contains("changed"));
}

#[test]
fn stash_drop_keeps_terminal_push_between_validation_and_deletion() {
    terminal_push_at_drop_boundary(false);
}

#[test]
fn stash_pop_keeps_terminal_push_between_validation_and_deletion() {
    terminal_push_at_drop_boundary(true);
}

#[test]
fn stash_locked_drop_excludes_real_terminal_push_and_drop() {
    let repo = TempRepo::new();
    repo.baseline();
    for name in ["A", "B", "C"] {
        repo.write("file.txt", format!("{name}\n"));
        repo.git(&["stash", "push", "-m", name]);
    }
    let selected = prepare_stashes(&repo.0).unwrap();
    let expected = selected.entries.clone();
    STASH_DROP_BOUNDARY.with(|hook| {
        *hook.borrow_mut() = Some((
            true,
            Box::new(move |root| {
                std::fs::write(root.join("file.txt"), "terminal work\n").unwrap();
                for args in [vec!["stash", "push", "-m", "terminal"], vec!["stash", "drop", "stash@{0}"]] {
                    let result = Command::new("git")
                        .arg("-C")
                        .arg(root)
                        .args(["-c", "core.filesRefLockTimeout=0"])
                        .args(args)
                        .output()
                        .unwrap();
                    assert!(!result.status.success(), "terminal mutation must respect held ref lock");
                    // Git stash push reduces its ref-lock error to "Cannot save the current status".
                    assert!(!result.stderr.is_empty());
                }
                assert_eq!(prepare_stashes(root).unwrap().entries, expected);
            }),
        ))
    });
    drop_stash(&repo.0, &selected, &selected.entries[1]).unwrap();
    let after = prepare_stashes(&repo.0).unwrap();
    assert_eq!(
        after.entries.iter().map(|s| &s.id).collect::<Vec<_>>(),
        [&selected.entries[0].id, &selected.entries[2].id]
    );
    assert_eq!(repo.read("file.txt"), "terminal work\n");
    assert!(!repo.0.join(".git/refs/stash.lock").exists());
    // The identical terminal operation succeeds once this transaction releases its lock.
    repo.git(&["stash", "push", "-m", "terminal"]);
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries.len(), 3);
}

#[test]
fn stash_locked_drop_handles_top_older_and_last_without_identity_configuration() {
    let repo = TempRepo::new();
    repo.baseline();
    for name in ["A", "B", "C"] {
        repo.write("file.txt", format!("{name}\n"));
        repo.git(&["stash", "push", "-m", name]);
    }
    // Empty local values also mask any developer-global identity read by libgit2.
    repo.git(&["config", "user.name", ""]);
    repo.git(&["config", "user.email", ""]);
    repo.git(&["config", "user.useConfigOnly", "true"]);
    let before = prepare_stashes(&repo.0).unwrap();
    drop_stash(&repo.0, &before, &before.entries[0]).unwrap();
    let current = prepare_stashes(&repo.0).unwrap();
    assert_eq!(current.entries[0].id, before.entries[1].id);
    assert_eq!(current.entries[1].id, before.entries[2].id);
    drop_stash(&repo.0, &current, &current.entries[1]).unwrap();
    let current = prepare_stashes(&repo.0).unwrap();
    assert_eq!(current.entries.len(), 1);
    assert_eq!(current.entries[0].id, before.entries[1].id);
    drop_stash(&repo.0, &current, &current.entries[0]).unwrap();
    assert!(prepare_stashes(&repo.0).unwrap().entries.is_empty());
    assert!(!repo.run(&["rev-parse", "--verify", "refs/stash"]).status.success());
    assert!(!repo.0.join(".git/refs/stash.lock").exists());
}

#[test]
fn stash_linked_worktree_drop_locks_the_shared_common_ref() {
    let repo = TempRepo::new();
    repo.baseline();
    let linked = repo.0.join("linked");
    repo.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    for name in ["A", "B"] {
        repo.write("file.txt", format!("{name}\n"));
        repo.git(&["stash", "push", "-m", name]);
    }
    let selected = prepare_stashes(&linked).unwrap();
    let main = repo.0.clone();
    STASH_DROP_BOUNDARY.with(|hook| {
        *hook.borrow_mut() = Some((
            true,
            Box::new(move |_| {
                let result = Command::new("git")
                    .arg("-C")
                    .arg(&main)
                    .args(["-c", "core.filesRefLockTimeout=0", "stash", "drop", "stash@{0}"])
                    .output()
                    .unwrap();
                assert!(!result.status.success(), "main worktree must see linked worktree's ref lock");
                assert!(
                    String::from_utf8_lossy(&result.stderr).contains("lock"),
                    "stdout={} stderr={}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
            }),
        ))
    });
    drop_stash(&linked, &selected, &selected.entries[1]).unwrap();
    let after = prepare_stashes(&repo.0).unwrap();
    assert_eq!(after.entries.len(), 1);
    assert_eq!(after.entries[0].id, selected.entries[0].id);
    let linked_context = prepare_stashes(&linked).unwrap();
    drop_stash(&linked, &linked_context, &linked_context.entries[0]).unwrap();
    assert!(prepare_stashes(&repo.0).unwrap().entries.is_empty());
    assert_eq!(repo.read("file.txt"), "base\n");
    assert_eq!(std::fs::read_to_string(linked.join("file.txt")).unwrap(), "base\n");
}

#[test]
fn stash_ref_lock_and_reflog_write_failures_release_ownership_without_deleting_entries() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "stashed\n");
    repo.git(&["stash", "push"]);
    let selected = prepare_stashes(&repo.0).unwrap();
    let reflog = std::fs::read(repo.0.join(".git/logs/refs/stash")).unwrap();
    for relative in [".git/refs/stash.lock", ".git/logs/refs/stash.lock"] {
        let lock = repo.0.join(relative);
        std::fs::write(&lock, "external lock").unwrap();
        let error = drop_stash(&repo.0, &selected, &selected.entries[0]).unwrap_err();
        assert!(error.contains("lock"), "{error}");
        assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
        assert_eq!(std::fs::read(repo.0.join(".git/logs/refs/stash")).unwrap(), reflog);
        std::fs::remove_file(lock).unwrap();
        assert!(!repo.0.join(".git/refs/stash.lock").exists());
    }
    drop_stash(&repo.0, &selected, &selected.entries[0]).unwrap();
    assert!(prepare_stashes(&repo.0).unwrap().entries.is_empty());
}

#[test]
fn stash_pop_reports_applied_changes_when_reflog_write_fails() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "stashed\n");
    repo.git(&["stash", "push"]);
    let selected = prepare_stashes(&repo.0).unwrap();
    let lock = repo.0.join(".git/logs/refs/stash.lock");
    std::fs::write(&lock, "external lock").unwrap();
    let error = apply_stash(&repo.0, &selected, &selected.entries[0], false, true).unwrap_err();
    assert!(error.contains("Changes applied, but stash removal failed"), "{error}");
    assert!(error.contains("do not apply these changes again"));
    assert_eq!(repo.read("file.txt"), "stashed\n");
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
    assert!(!repo.0.join(".git/refs/stash.lock").exists());
    std::fs::remove_file(lock).unwrap();
}

#[test]
fn stash_unsupported_native_formats_fail_closed_before_pop_applies() {
    for option in ["--object-format=sha256", "--ref-format=reftable"] {
        let repo = TempRepo::new();
        std::fs::remove_dir_all(repo.0.join(".git")).unwrap();
        repo.git(&["init", "-b", "main", option]);
        repo.git(&["config", "user.name", "Orlo test"]);
        repo.git(&["config", "user.email", "orlo@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "core.hooksPath", "/dev/null"]);
        repo.baseline();
        repo.write("file.txt", "stashed\n");
        repo.git(&["stash", "push"]);
        let selected = prepare_stashes(&repo.0).unwrap();
        assert!(drop_stash(&repo.0, &selected, &selected.entries[0]).unwrap_err().contains("Cannot safely remove"));
        let error = apply_stash(&repo.0, &selected, &selected.entries[0], false, true).unwrap_err();
        assert!(error.contains("no changes applied"), "{option}: {error}");
        assert_eq!(repo.read("file.txt"), "base\n");
        assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
        apply_stash(&repo.0, &selected, &selected.entries[0], false, false).unwrap();
        assert_eq!(repo.read("file.txt"), "stashed\n");
    }
}

#[test]
fn stash_drop_does_not_remove_next_entry_after_terminal_drops_selected_entry() {
    let repo = TempRepo::new();
    repo.baseline();
    for name in ["older", "selected"] {
        repo.write("file.txt", format!("{name}\n"));
        repo.git(&["stash", "push", "-m", name]);
    }
    let selected = prepare_stashes(&repo.0).unwrap();
    STASH_DROP_BOUNDARY.with(|hook| {
        *hook.borrow_mut() = Some((
            false,
            Box::new(|root| {
                let result =
                    Command::new("git").arg("-C").arg(root).args(["stash", "drop", "stash@{0}"]).output().unwrap();
                assert!(result.status.success());
            }),
        ))
    });
    assert!(drop_stash(&repo.0, &selected, &selected.entries[0]).unwrap_err().contains("changed"));
    let after = prepare_stashes(&repo.0).unwrap();
    assert_eq!(after.entries.len(), 1);
    assert_eq!(after.entries[0].id, selected.entries[1].id);
    assert!(!repo.0.join(".git/refs/stash.lock").exists());
}

#[test]
fn stash_packed_last_ref_io_error_reports_possible_partial_removal() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "stashed\n");
    repo.git(&["stash", "push"]);
    repo.git(&["pack-refs", "--all"]);
    let selected = prepare_stashes(&repo.0).unwrap();
    assert!(!repo.0.join(".git/refs/stash").exists());
    let lock = repo.0.join(".git/packed-refs.lock");
    std::fs::write(&lock, "external lock").unwrap();
    let error = drop_stash(&repo.0, &selected, &selected.entries[0]).unwrap_err();
    assert!(error.contains("may be incomplete"), "{error}");
    assert!(error.contains(&selected.entries[0].id));
    assert!(!repo.0.join(".git/refs/stash.lock").exists());
    // This documents the library's limit: reflog publication precedes ref removal.
    assert!(prepare_stashes(&repo.0).unwrap().entries.is_empty());
    assert_eq!(repo.git(&["rev-parse", "refs/stash"]).trim(), selected.entries[0].id);
    std::fs::remove_file(lock).unwrap();
}

#[test]
fn stash_conflicted_apply_preserves_exact_stack_and_native_unmerged_status() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "stashed 界\n");
    repo.git(&["stash", "push"]);
    repo.write("file.txt", "new head 🦀\n");
    repo.commit_all("new head");
    let selected = prepare_stashes(&repo.0).unwrap();
    let error = apply_stash(&repo.0, &selected, &selected.entries[0], false, false).unwrap_err();
    assert!(error.contains("kept"), "{error}");
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
    assert_eq!(repo.snapshot().conflict_count(), 1);
    let text = repo.read("file.txt");
    assert_eq!(resolve_conflict(&text, 0, ConflictChoice::Both).unwrap(), "new head 🦀\nstashed 界\n");
}

#[test]
fn stash_selection_requires_matching_display_index_and_object_identity() {
    let repo = TempRepo::new();
    repo.baseline();
    for text in ["first", "second"] {
        repo.write("file.txt", format!("{text}\n"));
        repo.git(&["stash", "push", "-m", text]);
    }
    let selected = prepare_stashes(&repo.0).unwrap();
    assert_eq!(selected.entries.len(), 2);
    assert_ne!(selected.entries[0].id, selected.entries[1].id);
    for wrong_object in [false, true] {
        let mut forged = selected.entries[0].clone();
        if wrong_object {
            forged.id = selected.entries[1].id.clone();
        } else {
            forged.selector = selected.entries[1].selector.clone();
        }
        for pop in [false, true] {
            assert!(apply_stash(&repo.0, &selected, &forged, false, pop).unwrap_err().contains("current stash list"));
        }
        assert!(stash_preview(&repo.0, &selected, &forged).unwrap_err().contains("current stash list"));
        assert!(drop_stash(&repo.0, &selected, &forged).unwrap_err().contains("current stash list"));
        assert_eq!(repo.read("file.txt"), "base\n");
        assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
    }
    // A current older entry still applies its own object, rather than the top display row.
    apply_stash(&repo.0, &selected, &selected.entries[1], false, false).unwrap();
    assert_eq!(repo.read("file.txt"), "first\n");
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
}

#[test]
fn stash_operations_refuse_replaced_checkout_root_with_same_git_directory() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.write("file.txt", "stashed\n");
    repo.git(&["stash", "push"]);
    let selected = prepare_stashes(&repo.0).unwrap();
    let moved = TempRepo(repo.0.with_extension("moved"));
    std::fs::rename(&repo.0, &moved.0).unwrap();
    std::fs::create_dir(&repo.0).unwrap();
    std::fs::rename(moved.0.join(".git"), repo.0.join(".git")).unwrap();
    repo.write("file.txt", "base\n");
    for pop in [false, true] {
        assert!(apply_stash(&repo.0, &selected, &selected.entries[0], false, pop).unwrap_err().contains("repository"));
    }
    assert!(drop_stash(&repo.0, &selected, &selected.entries[0]).unwrap_err().contains("repository"));
    assert!(stash_preview(&repo.0, &selected, &selected.entries[0]).unwrap_err().contains("repository"));
    assert!(create_stash(&repo.0, &selected, "", true).unwrap_err().contains("repository"));
    assert_eq!(repo.read("file.txt"), "base\n");
    assert_eq!(prepare_stashes(&repo.0).unwrap().entries, selected.entries);
}

#[path = "worktrees/tests.rs"]
mod worktrees_tests;

#[test]
fn remote_fetch_pull_push_and_upstream_use_only_explicit_local_remote() {
    let source = TempRepo::new();
    source.baseline();
    let bare = TempRepo::new();
    // Convert only this fresh disposable fixture into a bare remote.
    bare.git(&["config", "core.bare", "true"]);
    let local = TempRepo::new();
    for repo in [&source, &local] {
        let context = prepare_remotes(&repo.0).unwrap();
        remote_operation(
            &repo.0,
            &context,
            &RemoteOperation::AddRemote { name: "origin".into(), url: bare.0.join(".git").to_str().unwrap().into() },
        )
        .unwrap();
    }
    let c = prepare_remotes(&source.0).unwrap();
    remote_operation(
        &source.0,
        &c,
        &RemoteOperation::Push { remote: "origin".into(), branch: "main".into(), set_upstream: true },
    )
    .unwrap();
    let c = prepare_remotes(&local.0).unwrap();
    remote_operation(&local.0, &c, &RemoteOperation::Fetch { remote: "origin".into() }).unwrap();
    assert!(!local.0.join("file.txt").exists());
    local.git(&["switch", "-C", "main", "origin/main"]);
    source.write("file.txt", "remote 界\n");
    source.commit_all("remote edit");
    source.git(&["push", "origin", "main"]);
    // A merge-preferring configuration must still produce an exact fast-forward.
    local.git(&["config", "pull.ff", "false"]);
    local.git(&["config", "pull.rebase", "false"]);
    let c = prepare_remotes(&local.0).unwrap();
    remote_operation(&local.0, &c, &RemoteOperation::Pull).unwrap();
    assert_eq!(local.git(&["rev-parse", "HEAD"]), source.git(&["rev-parse", "HEAD"]));
    assert_eq!(local.read("file.txt"), "remote 界\n");
    remote_operation(&local.0, &c, &RemoteOperation::Pull).unwrap_err();
    let c = prepare_remotes(&local.0).unwrap();
    remote_operation(&local.0, &c, &RemoteOperation::SetUpstream { upstream: None }).unwrap();
    let c = prepare_remotes(&local.0).unwrap();
    assert!(remote_operation(&local.0, &c, &RemoteOperation::Pull).unwrap_err().contains("upstream"));
    remote_operation(&local.0, &c, &RemoteOperation::SetUpstream { upstream: Some("origin/main".into()) }).unwrap();
}

#[test]
fn remote_configuration_is_local_and_rejects_changed_url_and_branch() {
    let repo = TempRepo::new();
    repo.baseline();
    let c = prepare_remotes(&repo.0).unwrap();
    remote_operation(
        &repo.0,
        &c,
        &RemoteOperation::AddRemote { name: "origin".into(), url: "/does-not-exist/界".into() },
    )
    .unwrap();
    let c = prepare_remotes(&repo.0).unwrap();
    assert_eq!(c.remotes[0].fetch_urls, vec!["/does-not-exist/界"]);
    remote_operation(
        &repo.0,
        &c,
        &RemoteOperation::EditRemote {
            name: "origin".into(),
            url: "/new-missing".into(),
            push_url: Some("/push-missing".into()),
        },
    )
    .unwrap();
    assert!(remote_operation(&repo.0, &c, &RemoteOperation::Fetch { remote: "origin".into() })
        .unwrap_err()
        .contains("changed"));
    let c = prepare_remotes(&repo.0).unwrap();
    repo.git(&["switch", "-c", "other"]);
    assert!(remote_operation(&repo.0, &c, &RemoteOperation::RemoveRemote { name: "origin".into() })
        .unwrap_err()
        .contains("changed"));
    let c = prepare_remotes(&repo.0).unwrap();
    remote_operation(&repo.0, &c, &RemoteOperation::RemoveRemote { name: "origin".into() }).unwrap();
    assert!(prepare_remotes(&repo.0).unwrap().remotes.is_empty());
}

#[test]
fn remote_pull_is_fast_forward_only_and_push_never_forces() {
    let source = TempRepo::new();
    source.baseline();
    let bare = TempRepo::new();
    bare.git(&["config", "core.bare", "true"]);
    let url = bare.0.join(".git");
    source.git(&["remote", "add", "origin", url.to_str().unwrap()]);
    source.git(&["push", "-u", "origin", "main"]);
    let local = TempRepo::new();
    local.git(&["remote", "add", "origin", url.to_str().unwrap()]);
    local.git(&["fetch", "origin"]);
    local.git(&["switch", "-C", "main", "origin/main"]);
    local.write("local.txt", "local\n");
    local.commit_all("local");
    source.write("remote.txt", "remote\n");
    source.commit_all("remote");
    source.git(&["push", "origin", "main"]);
    let before = local.git(&["rev-parse", "HEAD"]);
    let index = std::fs::read(local.0.join(".git/index")).unwrap();
    // Configured ff-only, merge and rebase all retain the same conservative UI policy.
    for (ff, rebase) in [("only", "false"), ("false", "true"), ("false", "false")] {
        local.git(&["config", "pull.ff", ff]);
        local.git(&["config", "pull.rebase", rebase]);
        local.git(&["config", "branch.main.rebase", rebase]);
        let c = prepare_remotes(&local.0).unwrap();
        let error = remote_operation(&local.0, &c, &RemoteOperation::Pull).expect_err("divergence must be refused");
        assert!(error.contains("fast-forward"), "{error}");
        assert_eq!(before, local.git(&["rev-parse", "HEAD"]));
        assert_eq!(std::fs::read(local.0.join(".git/index")).unwrap(), index);
        assert_eq!(local.read("local.txt"), "local\n");
        assert!(!local.0.join("remote.txt").exists());
        assert!(!local.0.join(".git/MERGE_HEAD").exists());
        assert!(!local.0.join(".git/rebase-merge").exists());
        assert!(!local.0.join(".git/rebase-apply").exists());
    }
    source.write("file.txt", "remote conflict\n");
    source.commit_all("remote conflict");
    source.git(&["push", "origin", "main"]);
    local.write("file.txt", "local conflict\n");
    local.commit_all("local conflict");
    let c = prepare_remotes(&local.0).unwrap();
    let remote_head = bare.git(&["rev-parse", "main"]);
    assert!(remote_operation(
        &local.0,
        &c,
        &RemoteOperation::Push { remote: "origin".into(), branch: "main".into(), set_upstream: false }
    )
    .is_err());
    assert_eq!(bare.git(&["rev-parse", "main"]), remote_head);
    assert!(remote_operation(&local.0, &c, &RemoteOperation::Pull).is_err());
    assert_eq!(local.read("file.txt"), "local conflict\n");
}

#[test]
fn remote_unborn_detached_and_local_discovery_never_contact_transport() {
    let repo = TempRepo::new();
    repo.git(&["remote", "add", "origin", "/does-not-exist/no-transport"]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(remote_operation(&repo.0, &c, &RemoteOperation::Pull).unwrap_err().contains("Commit"));
    assert!(remote_operation(
        &repo.0,
        &c,
        &RemoteOperation::Push { remote: "origin".into(), branch: "main".into(), set_upstream: true }
    )
    .unwrap_err()
    .contains("Commit"));
    repo.baseline();
    // A local tracking ref permits local upstream configuration despite an unreachable URL.
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let c = prepare_remotes(&repo.0).unwrap();
    remote_operation(&repo.0, &c, &RemoteOperation::SetUpstream { upstream: Some("origin/main".into()) }).unwrap();
    assert_eq!(prepare_remotes(&repo.0).unwrap().upstream.as_deref(), Some("origin/main"));
    repo.git(&["checkout", "--detach"]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(remote_operation(&repo.0, &c, &RemoteOperation::Pull).unwrap_err().contains("detached"));
    assert!(remote_operation(&repo.0, &c, &RemoteOperation::Fetch { remote: "origin".into() })
        .unwrap_err()
        .contains("Git failed"));
}

#[test]
fn remote_push_preserves_native_hooks_and_bounds_noisy_errors() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TempRepo::new();
    repo.baseline();
    let bare = TempRepo::new();
    bare.git(&["config", "core.bare", "true"]);
    repo.git(&["remote", "add", "origin", bare.0.join(".git").to_str().unwrap()]);
    let hooks = repo.0.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("pre-push");
    std::fs::write(&hook, "#!/bin/sh\necho called > hook-called\ni=0\nwhile [ $i -lt 3000 ]; do echo hook-diagnostic >&2; i=$((i+1)); done\nexit 42\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    repo.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let c = prepare_remotes(&repo.0).unwrap();
    let error = remote_operation(
        &repo.0,
        &c,
        &RemoteOperation::Push { remote: "origin".into(), branch: "main".into(), set_upstream: true },
    )
    .unwrap_err();
    assert_eq!(repo.read("hook-called"), "called\n");
    assert!(error.contains("hook-diagnostic"));
    assert!(error.len() < 17000);
    assert!(!bare.run(&["rev-parse", "--verify", "main"]).status.success());
}

#[test]
fn remote_push_preserves_signing_policy_and_never_uses_configured_mirror() {
    let repo = TempRepo::new();
    repo.baseline();
    let bare = TempRepo::new();
    bare.baseline();
    bare.write("remote-only.txt", "keep remote bytes\n");
    bare.commit_all("remote-only history");
    bare.git(&["config", "core.bare", "true"]);
    let before = bare.git(&["rev-parse", "main"]);
    repo.git(&["remote", "add", "origin", bare.0.join(".git").to_str().unwrap()]);
    repo.git(&["config", "push.gpgSign", "true"]);
    let c = prepare_remotes(&repo.0).unwrap();
    let op = RemoteOperation::Push { remote: "origin".into(), branch: "published".into(), set_upstream: false };
    let error = remote_operation(&repo.0, &c, &op).unwrap_err();
    assert!(error.contains("signed push"), "{error}");
    assert!(!bare.run(&["rev-parse", "--verify", "published"]).status.success());
    repo.git(&["config", "push.gpgSign", "false"]);
    repo.git(&["config", "remote.origin.mirror", "true"]);
    let c = prepare_remotes(&repo.0).unwrap();
    remote_operation(&repo.0, &c, &op).unwrap();
    assert_eq!(bare.git(&["rev-parse", "main"]), before);
    assert_eq!(bare.git(&["rev-parse", "published"]), repo.git(&["rev-parse", "main"]));
    assert_eq!(bare.git(&["show", "main:remote-only.txt"]), "keep remote bytes\n");
    bare.git(&["update-ref", "refs/heads/published", "main"]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(remote_operation(&repo.0, &c, &op).is_err());
    assert_eq!(bare.git(&["rev-parse", "published"]), before);
    assert_eq!(bare.git(&["rev-parse", "main"]), before);
    assert_eq!(bare.git(&["show", "published:remote-only.txt"]), "keep remote bytes\n");
}

#[test]
fn remote_include_targets_cover_empty_nested_conditional_and_missing_paths() {
    let repo = TempRepo::new();
    let external = TempRepo::new();
    external.write("empty 界.config", "# comments only\n\n");
    external.write("active.config", " \n# active but empty\n");
    external.write("inactive.config", "# inactive\n");
    let nested = external.0.join("nested.config");
    external.write("nested.config", "[include]\npath = empty 界.config\n[includeIf \"onbranch:main\"]\npath = active.config\n[includeIf \"onbranch:other\"]\npath = inactive.config\n[include]\npath = future/nested.config\n");
    repo.git(&["config", "include.path", nested.to_str().unwrap()]);
    let c = prepare_remotes(&repo.0).unwrap();
    for name in ["nested.config", "empty 界.config", "active.config", "inactive.config", "future/nested.config"] {
        assert!(c.config_files.contains(&external.0.join(name)), "missing guarded include {name}");
    }
    // Optional missing includes remain valid, and an alias resolves the future save path.
    std::os::unix::fs::symlink(&external.0, repo.0.join("alias")).unwrap();
    repo.git(&["config", "--add", "include.path", "../alias/missing.config"]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(c.config_files.contains(&external.0.join("missing.config")));
    std::fs::create_dir(external.0.join("future")).unwrap();
    external.write("future/nested.config", "[remote \"future\"]\nurl = /not-contacted\n");
    assert!(c.validate(&repo.0).is_err());
}

#[test]
fn remote_include_targets_use_git_tilde_expansion_and_relative_origins() {
    let repo = TempRepo::new();
    let external = TempRepo::new();
    external.write("empty.config", "# nothing\n");
    let home = std::env::home_dir().unwrap();
    let mut tilde = PathBuf::from("~");
    for part in home.components() {
        if matches!(part, std::path::Component::Normal(_)) {
            tilde.push("..");
        }
    }
    tilde.push(external.0.strip_prefix("/").unwrap());
    tilde.push("empty.config");
    repo.git(&["config", "include.path", tilde.to_str().unwrap()]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(c.config_files.contains(&external.0.join("empty.config")));
    // Traversal relative to .git/config is interpreted by Git, not relative to the checkout.
    repo.write("relative.config", "# also empty\n");
    repo.git(&["config", "--add", "include.path", "../relative.config"]);
    assert!(prepare_remotes(&repo.0).unwrap().config_files.contains(&repo.0.join("relative.config")));
}

#[test]
fn remote_include_targets_cover_dangling_symlink_future_saves_and_optional_non_directories() {
    let repo = TempRepo::new();
    let external = TempRepo::new();
    let future = external.0.join("future.config");
    let alias = repo.0.join("include-alias");
    std::os::unix::fs::symlink(&future, &alias).unwrap();
    repo.git(&["config", "include.path", "../include-alias"]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(c.config_files.contains(&future), "lease the future symlink target");
    assert!(c.config_files.contains(&alias), "lease a save through the still-dangling alias too");
    repo.write("plain-file", "not a directory");
    repo.git(&["config", "--add", "include.path", "../plain-file/optional.config"]);
    repo.git(&["config", "--list"]); // Git treats this missing include as optional too.
    assert!(prepare_remotes(&repo.0).unwrap().config_files.contains(&repo.0.join("plain-file/optional.config")));
}

#[test]
fn remote_include_targets_preserve_newlines_in_paths() {
    let repo = TempRepo::new();
    let external = TempRepo::new();
    let target = external.0.join("empty\ninclude.config");
    std::fs::write(&target, "# empty\n").unwrap();
    repo.git(&["config", "includeIf.onbranch:main.path", target.to_str().unwrap()]);
    let c = prepare_remotes(&repo.0).unwrap();
    assert!(c.config_files.contains(&target), "native config records must preserve path newlines");
}

#[test]
fn remote_top_level_config_roots_are_native_and_include_empty_or_missing_files() {
    const CASE: &str = "ORLO_TEST_CONFIG_ROOT_CASE";
    if let Some(case) = std::env::var_os(CASE) {
        let base = PathBuf::from(std::env::var_os("ORLO_TEST_CONFIG_ROOT").unwrap());
        let repo = base.join("repository");
        let case = case.to_str().unwrap();
        let result = prepare_remotes(&repo);
        if case.starts_with("newline-") {
            assert!(result.unwrap_err().contains("newline"));
            return;
        }
        let result = result.unwrap();
        let global = base.join("home/.gitconfig");
        let xdg = base.join(if case == "xdg-fallback" { "home/.config/git/config" } else { "xdg/git/config" });
        let system = base.join("system.config");
        let override_ = base.join("override 界.config");
        for (path, expected) in [
            (global, case != "override" && case != "disabled-global" && case != "no-home"),
            (xdg, case != "override" && case != "disabled-global"),
            (system, case != "no-system"),
            (override_, case == "override"),
        ] {
            assert_eq!(
                result.config_files.contains(&path),
                expected,
                "{case}: {} in {:?}",
                path.display(),
                result.config_files
            );
        }
        // The same identities are retained when an optional root is created with no settings.
        let missing = result.config_files.iter().find(|path| !path.exists()).cloned();
        if let Some(path) = missing {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "# newly created, still no values\n").unwrap();
            assert!(prepare_remotes(&repo).unwrap().config_files.contains(&path));
        }
        return;
    }
    for case in [
        "missing",
        "comments",
        "override",
        "disabled-global",
        "no-system",
        "false-no-system",
        "xdg-fallback",
        "no-home",
        "newline-home",
        "newline-global",
        "newline-system",
    ] {
        let fixture = TempRepo::new();
        let base = fixture.0.clone();
        for dir in ["home/.config/git", "xdg/git", "repository"] {
            std::fs::create_dir_all(base.join(dir)).unwrap();
        }
        let mut init = Command::new("git");
        let status = init
            .args(["init", "--initial-branch=main"])
            .arg(base.join("repository"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(status.status.success());
        if case == "comments" {
            for path in ["home/.gitconfig", "xdg/git/config", "system.config"] {
                std::fs::write(base.join(path), "# comments only\n\n").unwrap();
            }
        }
        let mut child = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_CONFIG")) {
            child.env_remove(key);
        }
        child
            .args([
                "--exact",
                "tests::remote_top_level_config_roots_are_native_and_include_empty_or_missing_files",
                "--nocapture",
            ])
            .env(CASE, case)
            .env("ORLO_TEST_CONFIG_ROOT", &base)
            .env("HOME", base.join("home"))
            .env("XDG_CONFIG_HOME", base.join("xdg"))
            .env("GIT_CONFIG_SYSTEM", base.join("system.config"));
        match case {
            "override" => {
                child.env("GIT_CONFIG_GLOBAL", base.join("override 界.config"));
            }
            "disabled-global" => {
                child.env("GIT_CONFIG_GLOBAL", "");
            }
            "no-system" => {
                child.env("GIT_CONFIG_NOSYSTEM", "true");
            }
            "false-no-system" => {
                child.env("GIT_CONFIG_NOSYSTEM", "false");
            }
            "xdg-fallback" => {
                child.env_remove("XDG_CONFIG_HOME");
            }
            "no-home" => {
                child.env_remove("HOME");
            }
            "newline-home" => {
                child.env("HOME", base.join("new\nline"));
            }
            "newline-global" => {
                child.env("GIT_CONFIG_GLOBAL", base.join("new\nline"));
            }
            "newline-system" => {
                child.env("GIT_CONFIG_SYSTEM", base.join("new\nline"));
            }
            _ => {}
        }
        let result = child.output().unwrap();
        assert!(
            result.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn remote_inherited_credential_helper_cancellation_is_a_bounded_failure() {
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "ORLO_TEST_REMOTE_CREDENTIALS";
    if std::env::var_os(CHILD).is_none() {
        let fixture = TempRepo::new();
        let helper = fixture.0.join("credential-helper");
        std::fs::write(&helper, "#!/bin/sh\necho $$ > \"$ORLO_TEST_REMOTE_CREDENTIALS/helper.pid\"\ncat > \"$ORLO_TEST_REMOTE_CREDENTIALS/helper-input\"\necho 'Authentication cancelled by fixture credential helper' >&2\nprintf 'quit=true\\n\\n'\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = fixture.0.join("inherited.config");
        fixture.git(&["config", "--file", config.to_str().unwrap(), "credential.helper", helper.to_str().unwrap()]);
        let mut child = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_CONFIG")) {
            child.env_remove(key);
        }
        let result = child
            .args([
                "--exact",
                "tests::remote_inherited_credential_helper_cancellation_is_a_bounded_failure",
                "--nocapture",
            ])
            .env(CHILD, &fixture.0)
            .env("HOME", fixture.0.join("home"))
            .env("XDG_CONFIG_HOME", fixture.0.join("xdg"))
            .env("GIT_CONFIG_GLOBAL", config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ASKPASS", "/usr/bin/false")
            .env("NO_PROXY", "*")
            .env("no_proxy", "*")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let fixture = PathBuf::from(std::env::var_os(CHILD).unwrap());
    let repo = TempRepo::new();
    repo.baseline();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/fixture.git", listener.local_addr().unwrap());
    repo.git(&["remote", "add", "origin", &url]);
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false).unwrap();
                    socket.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
                    let mut request = [0; 8192];
                    let mut size = 0;
                    while !request[..size].windows(4).any(|part| part == b"\r\n\r\n") {
                        let n = socket.read(&mut request[size..]).unwrap();
                        assert!(n > 0, "fixture request must fit the bounded header buffer");
                        size += n;
                    }
                    assert!(String::from_utf8_lossy(&request[..size]).contains("/fixture.git/info/refs"));
                    socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => panic!("fixture HTTP accept: {e}"),
            }
        }
    });
    let context = prepare_remotes(&repo.0).unwrap();
    assert!(!fixture.join("helper.pid").exists(), "preparation must not request credentials");
    let head = repo.git(&["rev-parse", "HEAD"]);
    let content = repo.read("file.txt");
    let error = remote_operation(&repo.0, &context, &RemoteOperation::Fetch { remote: "origin".into() }).unwrap_err();
    server.join().unwrap();
    assert!(error.contains("Authentication cancelled by fixture credential helper"), "{error}");
    assert!(error.contains("Git failed"), "{error}");
    assert!(error.len() < 17000);
    assert!(std::fs::read_to_string(fixture.join("helper-input")).unwrap().contains("protocol=http"));
    let pid = std::fs::read_to_string(fixture.join("helper.pid")).unwrap();
    assert!(
        !Command::new("/bin/kill").args(["-0", pid.trim()]).output().unwrap().status.success(),
        "cancelled helper must have terminated"
    );
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(repo.read("file.txt"), content);
}

#[test]
fn remote_prepared_refspec_and_upstream_changes_refuse_before_transport() {
    let repo = TempRepo::new();
    repo.baseline();
    let bare = TempRepo::new();
    bare.git(&["config", "core.bare", "true"]);
    repo.git(&["remote", "add", "origin", bare.0.join(".git").to_str().unwrap()]);
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    repo.git(&["branch", "--set-upstream-to=origin/main"]);
    let before = prepare_remotes(&repo.0).unwrap();
    repo.git(&["config", "remote.origin.fetch", "+refs/heads/topic:refs/remotes/origin/main"]);
    assert!(remote_operation(&repo.0, &before, &RemoteOperation::Fetch { remote: "origin".into() })
        .unwrap_err()
        .contains("changed"));
    assert!(!repo.0.join(".git/FETCH_HEAD").exists());
    repo.git(&["config", "branch.main.merge", "refs/heads/topic"]);
    let before = prepare_remotes(&repo.0).unwrap();
    repo.git(&["config", "branch.main.merge", "refs/heads/changed"]);
    assert!(remote_operation(&repo.0, &before, &RemoteOperation::Pull).unwrap_err().contains("changed"));
    assert!(!repo.0.join(".git/FETCH_HEAD").exists());
}

#[test]
fn remote_pull_uses_native_ref_for_custom_tracking_and_local_upstreams() {
    let source = TempRepo::new();
    source.baseline();
    let bare = TempRepo::new();
    bare.git(&["config", "core.bare", "true"]);
    let local = TempRepo::new();
    for repo in [&source, &local] {
        repo.git(&["remote", "add", "origin", bare.0.join(".git").to_str().unwrap()]);
        repo.git(&["config", "remote.origin.fetch", "+refs/heads/published:refs/remotes/origin/review"]);
    }
    source.git(&["push", "origin", "main:published"]);
    local.git(&["fetch", "origin"]);
    local.git(&["switch", "-C", "main", "origin/review"]);
    source.write("file.txt", "mapped remote\n");
    source.commit_all("mapped remote");
    source.git(&["push", "origin", "main:published"]);
    let context = prepare_remotes(&local.0).unwrap();
    assert_eq!(context.upstream.as_deref(), Some("origin/review"));
    assert_eq!(context.upstream_remote.as_deref(), Some("origin"));
    assert_eq!(context.upstream_ref.as_deref(), Some("refs/heads/published"));
    remote_operation(&local.0, &context, &RemoteOperation::Pull).unwrap();
    assert_eq!(local.git(&["rev-parse", "HEAD"]), source.git(&["rev-parse", "HEAD"]));
    local.git(&["switch", "-c", "local-upstream"]);
    local.write("file.txt", "local upstream\n");
    local.commit_all("local upstream");
    local.git(&["switch", "main"]);
    local.git(&["branch", "--set-upstream-to=local-upstream"]);
    let context = prepare_remotes(&local.0).unwrap();
    assert_eq!(context.upstream_remote.as_deref(), Some("."));
    assert_eq!(context.upstream_ref.as_deref(), Some("refs/heads/local-upstream"));
    remote_operation(&local.0, &context, &RemoteOperation::Pull).unwrap();
    assert_eq!(local.read("file.txt"), "local upstream\n");
}

#[test]
fn tags_missing_promisor_objects_never_start_a_remote_transport() {
    use std::os::unix::fs::PermissionsExt;

    let remote = TempRepo::new();
    remote.baseline();
    remote.write("file.txt", "remote-only object\n");
    remote.commit_all("remote-only commit");
    let missing = remote.git(&["rev-parse", "HEAD"]).trim().to_owned();
    for mode in ["resolve", "list", "lightweight", "annotated"] {
        let local = TempRepo::new();
        local.baseline();
        let prepared = if matches!(mode, "lightweight" | "annotated") {
            local.git(&["fetch", "--no-tags", remote.0.to_str().unwrap(), "main"]);
            let tags = prepare_tags(&local.0).unwrap();
            let prepared = prepare_tag(&local.0, &tags, &missing).unwrap();
            std::fs::remove_file(local.0.join(".git/objects").join(&missing[..2]).join(&missing[2..])).unwrap();
            Some(prepared)
        } else {
            None
        };
        let contacted = local.0.join(".git/remote-contacted");
        let ssh = local.0.join(".git/local-ssh-fixture");
        let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
        std::fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nprintf contacted >> {}\nexec git upload-pack {}\n",
                quote(&contacted),
                quote(&remote.0)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
        local.git(&["remote", "add", "origin", "ssh://orlo-local-fixture/unused"]);
        local.git(&["config", "core.sshCommand", &quote(&ssh)]);
        local.git(&["config", "ssh.variant", "simple"]);
        local.git(&["config", "core.repositoryFormatVersion", "1"]);
        local.git(&["config", "extensions.partialClone", "origin"]);
        local.git(&["config", "remote.origin.promisor", "true"]);
        let tags = prepare_tags(&local.0).unwrap();
        let result = match mode {
            "resolve" => prepare_tag(&local.0, &tags, &missing).map(|_| ()),
            "list" => {
                std::fs::write(local.0.join(".git/refs/tags/missing"), format!("{missing}\n")).unwrap();
                prepare_tags(&local.0).map(|_| ())
            }
            "lightweight" => create_tag(&local.0, prepared.as_ref().unwrap(), "missing", ""),
            _ => create_tag(&local.0, prepared.as_ref().unwrap(), "missing", "annotation"),
        };
        assert!(!contacted.exists(), "local tag {mode} started a remote transport");
        assert!(result.is_err(), "missing promisor objects must fail locally for {mode}");
        // Positive control: unguarded Git really invokes this entirely local fake SSH service.
        let resolved = Command::new("git")
            .arg("-C")
            .arg(&local.0)
            .args(["rev-parse", "--verify", &format!("{missing}^{{commit}}")])
            .env("GIT_NO_LAZY_FETCH", "0")
            .env_remove("GIT_ALLOW_PROTOCOL")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(resolved.status.success(), "{}", String::from_utf8_lossy(&resolved.stderr));
        assert!(contacted.exists(), "the local promisor control must exercise transport for {mode}");
    }
}

#[test]
fn tags_local_creation_lists_exact_type_target_and_message_without_source_changes() {
    let repo = TempRepo::new();
    repo.baseline();
    let first = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.write("file.txt", "next\n");
    repo.commit_all("next");
    repo.write("file.txt", "dirty source\n");
    let before = (repo.snapshot(), repo.index("file.txt"), repo.read("file.txt"));
    repo.git(&["config", "tag.gpgSign", "true"]);
    repo.git(&["config", "core.editor", "false"]);
    repo.git(&["config", "gpg.program", "/orlo-must-not-invoke-gpg"]);
    repo.git(&["remote", "add", "must-not-contact", "orlo-no-network://local-tags-only"]);
    let tags = prepare_tags(&repo.0).unwrap();
    let head = prepare_tag(&repo.0, &tags, "HEAD").unwrap();
    let chosen = prepare_tag(&repo.0, &tags, "HEAD~1").unwrap();
    assert_eq!(chosen.commit(), first);
    create_tag(&repo.0, &head, "release/界", "").unwrap();
    create_tag(&repo.0, &chosen, "annotated", "Message 界\n\nBody 🦀").unwrap();
    let tags = prepare_tags(&repo.0).unwrap();
    assert_eq!(tags.entries.len(), 2);
    let annotated = &tags.entries[0];
    assert_eq!(annotated.name, "annotated");
    assert!(annotated.annotated);
    assert_eq!(annotated.target, first);
    assert_eq!(annotated.message.trim_end(), "Message 界\n\nBody 🦀");
    assert!(!tags.entries[1].annotated);
    assert_eq!(tags.entries[1].target, head.commit());
    assert_eq!(tags.entries[1].message, "");
    assert_eq!((repo.snapshot(), repo.index("file.txt"), repo.read("file.txt")), before);
    delete_tag(&repo.0, &tags, &tags.entries[0]).unwrap();
    assert_eq!(prepare_tags(&repo.0).unwrap().entries, vec![tags.entries[1].clone()]);
    assert_eq!((repo.snapshot(), repo.index("file.txt"), repo.read("file.txt")), before);
}

type TagBoundary = (bool, Box<dyn FnOnce(&Path)>);
thread_local! {
    static TAG_DELETE_BOUNDARY: std::cell::RefCell<Option<TagBoundary>> = Default::default();
}
pub(crate) fn tag_delete_boundary(root: &Path, locked: bool) {
    let callback = TAG_DELETE_BOUNDARY.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook.as_ref().is_some_and(|(phase, _)| *phase == locked) {
            hook.take()
        } else {
            None
        }
    });
    if let Some((_, callback)) = callback {
        callback(root);
    }
}

#[test]
fn tags_invalid_duplicate_unborn_and_non_commit_targets_never_create_refs() {
    let repo = TempRepo::new();
    let unborn = prepare_tags(&repo.0).unwrap();
    assert!(unborn.entries.is_empty());
    assert!(prepare_tag(&repo.0, &unborn, "HEAD").unwrap_err().contains("unborn"));
    repo.baseline();
    let tags = prepare_tags(&repo.0).unwrap();
    for revision in ["", "missing", "HEAD:file.txt", "--help", "HEAD\n"] {
        assert!(prepare_tag(&repo.0, &tags, revision).is_err(), "{revision}");
    }
    let prepared = prepare_tag(&repo.0, &tags, "HEAD").unwrap();
    for name in ["", "-option", "HEAD", "bad name", "x..y", "x.lock", "x\ny", "refs//tag"] {
        assert!(create_tag(&repo.0, &prepared, name, "").is_err(), "{name}");
    }
    assert!(create_tag(&repo.0, &prepared, "control", "bad\0message").is_err());
    assert!(prepare_tags(&repo.0).unwrap().entries.is_empty());
    create_tag(&repo.0, &prepared, "unique", " \n\t").unwrap();
    let before = prepare_tags(&repo.0).unwrap().entries;
    for message in ["", "replacement annotation"] {
        assert!(create_tag(&repo.0, &prepared, "unique", message).unwrap_err().contains("already exists"));
        assert_eq!(prepare_tags(&repo.0).unwrap().entries, before);
    }
}

#[test]
fn tags_pin_chosen_commit_and_reject_stale_head_and_repository_identity() {
    let repo = TempRepo::new();
    repo.baseline();
    let first = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["branch", "chosen"]);
    repo.write("file.txt", "next\n");
    repo.commit_all("next");
    let tags = prepare_tags(&repo.0).unwrap();
    let chosen = prepare_tag(&repo.0, &tags, "chosen").unwrap();
    repo.git(&["branch", "-f", "chosen", "HEAD"]);
    create_tag(&repo.0, &chosen, "pinned", "").unwrap();
    assert_eq!(repo.git(&["rev-parse", "refs/tags/pinned"]).trim(), first);
    let current = prepare_tag(&repo.0, &tags, "HEAD").unwrap();
    repo.write("file.txt", "third\n");
    repo.commit_all("third");
    assert!(create_tag(&repo.0, &current, "stale", "").unwrap_err().contains("changed"));
    assert!(prepare_tag(&repo.0, &tags, "HEAD").unwrap_err().contains("changed"));
    assert!(delete_tag(&repo.0, &tags, &prepare_tags(&repo.0).unwrap().entries[0]).is_err());
    let tags = prepare_tags(&repo.0).unwrap();
    let current = prepare_tag(&repo.0, &tags, "HEAD").unwrap();
    let old = repo.0.with_extension("old-root");
    std::fs::rename(&repo.0, &old).unwrap();
    std::fs::create_dir(&repo.0).unwrap();
    std::fs::rename(old.join(".git"), repo.0.join(".git")).unwrap();
    assert!(create_tag(&repo.0, &current, "replacement", "").unwrap_err().contains("changed"));
    assert!(delete_tag(&repo.0, &tags, &tags.entries[0]).unwrap_err().contains("changed"));
    std::fs::rename(repo.0.join(".git"), old.join(".git")).unwrap();
    std::fs::remove_dir(&repo.0).unwrap();
    std::fs::rename(old, &repo.0).unwrap();
}

#[test]
fn tags_delete_refuses_missing_and_replaced_annotation_even_at_same_commit() {
    let repo = TempRepo::new();
    repo.baseline();
    let tags = prepare_tags(&repo.0).unwrap();
    let prepared = prepare_tag(&repo.0, &tags, "HEAD").unwrap();
    create_tag(&repo.0, &prepared, "selected", "old annotation").unwrap();
    let selected = prepare_tags(&repo.0).unwrap();
    repo.git(&["-c", "tag.gpgSign=false", "tag", "-f", "-a", "selected", "-m", "new annotation"]);
    let replacement = prepare_tags(&repo.0).unwrap().entries;
    assert_eq!(replacement[0].target, selected.entries[0].target);
    assert!(delete_tag(&repo.0, &selected, &selected.entries[0]).unwrap_err().contains("changed"));
    assert_eq!(prepare_tags(&repo.0).unwrap().entries, replacement);
    repo.git(&["tag", "-d", "selected"]);
    assert!(delete_tag(&repo.0, &selected, &selected.entries[0]).unwrap_err().contains("missing"));
}

#[test]
fn tags_atomic_delete_keeps_replacement_after_freshness_check() {
    for symbolic in [false, true] {
        let repo = TempRepo::new();
        repo.baseline();
        repo.git(&["-c", "tag.gpgSign=false", "tag", "selected"]);
        let selected = prepare_tags(&repo.0).unwrap();
        let blob = repo.git(&["rev-parse", "HEAD:file.txt"]).trim().to_owned();
        let replacement = blob.clone();
        TAG_DELETE_BOUNDARY.with(|hook| {
            *hook.borrow_mut() = Some((
                false,
                Box::new(move |root| {
                    if symbolic {
                        run_git_test(root, &["symbolic-ref", "refs/tags/selected", "refs/heads/main"]);
                    } else {
                        run_git_test(root, &["update-ref", "refs/tags/selected", &replacement]);
                    }
                }),
            ));
        });
        assert!(delete_tag(&repo.0, &selected, &selected.entries[0]).is_err());
        if symbolic {
            assert_eq!(repo.git(&["symbolic-ref", "refs/tags/selected"]).trim(), "refs/heads/main");
            assert!(!repo.git(&["rev-parse", "HEAD"]).trim().is_empty());
        } else {
            assert_eq!(repo.git(&["rev-parse", "refs/tags/selected"]).trim(), blob);
        }
    }
}

fn run_git_test(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn tags_symbolic_selection_cannot_delete_the_destination() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.git(&["symbolic-ref", "refs/tags/symbolic", "refs/heads/main"]);
    let tags = prepare_tags(&repo.0).unwrap();
    assert!(delete_tag(&repo.0, &tags, &tags.entries[0]).unwrap_err().contains("Symbolic"));
    assert_eq!(repo.git(&["symbolic-ref", "refs/tags/symbolic"]).trim(), "refs/heads/main");
}

#[test]
fn tags_delete_holds_ref_lock_and_handles_packed_refs_and_lock_errors() {
    let repo = TempRepo::new();
    repo.baseline();
    repo.git(&["-c", "tag.gpgSign=false", "tag", "selected"]);
    repo.git(&["pack-refs", "--all"]);
    let tags = prepare_tags(&repo.0).unwrap();
    let lock = repo.0.join(".git/refs/tags/selected.lock");
    std::fs::write(&lock, "other writer").unwrap();
    assert!(delete_tag(&repo.0, &tags, &tags.entries[0]).unwrap_err().contains("lock"));
    assert_eq!(prepare_tags(&repo.0).unwrap().entries, tags.entries);
    std::fs::remove_file(lock).unwrap();
    TAG_DELETE_BOUNDARY.with(|hook| {
        *hook.borrow_mut() = Some((
            true,
            Box::new(|root| {
                let result = Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["update-ref", "refs/tags/selected", "HEAD:file.txt"])
                    .output()
                    .unwrap();
                assert!(!result.status.success(), "the selected tag lock must exclude external writers");
                assert!(String::from_utf8_lossy(&result.stderr).contains("lock"));
            }),
        ));
    });
    delete_tag(&repo.0, &tags, &tags.entries[0]).unwrap();
    assert!(prepare_tags(&repo.0).unwrap().entries.is_empty());
}

#[test]
fn partial_staging_rechecks_a_rename_with_both_of_its_names() {
    let repo = TempRepo::new();
    repo.write("file.txt", "a\nb\nc\nd\ne\n");
    repo.write("other.txt", "other\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "base"]);
    repo.git(&["mv", "file.txt", "moved.txt"]);
    repo.write("moved.txt", "a\nB\nc\nD\ne\n");
    repo.write("other.txt", "changed elsewhere\n");
    let file = repo.snapshot().files.into_iter().find(|f| f.path == Path::new("moved.txt")).unwrap();
    assert_eq!((file.index, file.worktree), ('R', 'M'));
    let shown = versions(&repo.0, &file, false, 1 << 20).unwrap();
    apply_selection(&repo.0, &file, false, &shown, &selected(false, 1, true)).unwrap();
    assert_eq!(index_text(&repo.0, Path::new("moved.txt")).unwrap().unwrap(), "a\nB\nc\nd\ne\n");
}

#[test]
fn a_folder_at_head_is_not_called_a_submodule() {
    let repo = TempRepo::new();
    std::fs::create_dir(repo.0.join("dir")).unwrap();
    repo.write("dir/inner.txt", "inner\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "base"]);
    let error = head_text(&repo.0, Path::new("dir")).unwrap_err();
    assert!(error.contains("folder"), "{error}");
    assert_eq!(head_text(&repo.0, Path::new("missing.txt")), Ok(None));
}

/// A repository whose own settings name `program` (a marker writer) as `key`.
fn repo_running(key: &str, attributes: &str) -> (TempRepo, PathBuf) {
    let repo = TempRepo::new();
    let marker = repo.0.with_extension("ran");
    let _ = std::fs::remove_file(&marker);
    let script = repo.0.with_extension("sh");
    std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(repo.0.join("a.txt"), "one\n").unwrap();
    std::fs::write(repo.0.join(".gitattributes"), attributes).unwrap();
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "init"]);
    repo.git(&["config", key, script.to_str().unwrap()]);
    // the same size, a new time: status must read the file again
    std::fs::write(repo.0.join("a.txt"), "two\n").unwrap();
    (repo, marker)
}

#[test]
fn a_repository_whose_own_filter_runs_a_program_waits_for_trust() {
    orlo_utils::trust::TRUST_ALL.set(false);
    let (repo, marker) = repo_running("filter.evil.process", "* filter=evil\n");
    let error = discover(&repo.0).unwrap_err();
    assert!(paused(&error) && error.contains("filter.evil.process"), "{error}");
    assert!(blame(&repo.0, Path::new("a.txt")).is_err_and(|e| paused(&e)));
    assert!(!marker.exists(), "nothing ran");
    orlo_utils::trust::TRUST_ALL.set(true);
    assert!(discover(&repo.0).unwrap().is_some(), "trusted, Git runs as configured");
}

#[test]
fn a_repositorys_fsmonitor_and_textconv_never_run() {
    for (key, attributes) in [("core.fsmonitor", ""), ("diff.evil.textconv", "* diff=evil\n")] {
        let (repo, marker) = repo_running(key, attributes);
        assert!(discover(&repo.0).unwrap().is_some());
        blame(&repo.0, Path::new("a.txt")).unwrap();
        assert!(!marker.exists(), "{key} ran");
    }
}
