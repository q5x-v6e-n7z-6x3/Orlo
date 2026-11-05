use super::*;
use crate::worktrees::*;

fn entry(context: &Worktrees, path: &Path) -> Worktree {
    context.entries.iter().find(|e| e.path == path).unwrap().clone()
}

#[test]
fn worktrees_create_list_linked_open_remove_preserves_literal_paths() {
    let r = TempRepo::new();
    r.baseline();
    let path = r.0.join("linked 界 space\nline");
    let context = prepare_worktrees(&r.0).unwrap();
    create_worktree(&r.0, &context, &path, "topic/界", true).unwrap();
    let linked = prepare_worktrees(&path).unwrap();
    assert_eq!(linked.entries.len(), 2);
    let selected = entry(&linked, &path);
    assert_eq!(selected.branch.as_deref(), Some("topic/界"));
    assert!(selected.current && !selected.main);
    assert_eq!(open_worktree(&path, &linked, &selected).unwrap(), path);
    assert!(remove_worktree(&path, &linked, &selected).unwrap_err().contains("current"));
    let main = entry(&linked, &r.0);
    assert!(remove_worktree(&path, &linked, &main).unwrap_err().contains("main"));
    let context = prepare_worktrees(&r.0).unwrap();
    remove_worktree(&r.0, &context, &entry(&context, &path)).unwrap();
    assert!(!path.exists());
    assert!(branches(&r.0).unwrap().contains(&"topic/界".into()), "removing a worktree keeps its branch");
    let context = prepare_worktrees(&r.0).unwrap();
    create_worktree(&r.0, &context, &path, "topic/界", false).unwrap();
    assert_eq!(std::fs::read_to_string(path.join("file.txt")).unwrap(), "base\n");
}

#[test]
fn worktrees_remove_refuses_dirty_untracked_locked_and_stale_selection() {
    let r = TempRepo::new();
    r.baseline();
    let path = r.0.join("linked");
    create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, "topic", true).unwrap();
    for file in ["file.txt", "new 界.txt"] {
        std::fs::write(path.join(file), "do not lose me").unwrap();
        let context = prepare_worktrees(&r.0).unwrap();
        assert!(remove_worktree(&r.0, &context, &entry(&context, &path)).is_err());
        assert_eq!(std::fs::read_to_string(path.join(file)).unwrap(), "do not lose me");
        if file == "file.txt" {
            std::fs::write(path.join(file), "base\n").unwrap();
        } else {
            std::fs::remove_file(path.join(file)).unwrap();
        }
    }
    r.git(&["worktree", "lock", path.to_str().unwrap(), "--reason", "keep 界"]);
    let locked = prepare_worktrees(&r.0).unwrap();
    assert!(entry(&locked, &path).locked.is_some());
    assert!(remove_worktree(&r.0, &locked, &entry(&locked, &path)).unwrap_err().contains("locked"));
    r.git(&["worktree", "unlock", path.to_str().unwrap()]);
    assert!(remove_worktree(&r.0, &locked, &entry(&locked, &path)).unwrap_err().contains("changed"));
    assert!(path.exists());
}

#[test]
fn worktrees_context_rejects_replaced_directory_repository_and_registrations() {
    let r = TempRepo::new();
    r.baseline();
    let path = r.0.join("linked");
    create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, "topic", true).unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    let selected = entry(&context, &path);
    std::fs::rename(&path, r.0.join("moved")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(open_worktree(&r.0, &context, &selected).unwrap_err().contains("changed"));
    assert!(remove_worktree(&r.0, &context, &selected).is_err());
    assert!(r.0.join("moved/file.txt").exists());
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(r.0.join("moved"), &path).unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    let admin = r.0.join(".git/worktrees/linked");
    std::fs::rename(&admin, r.0.join("old-registration")).unwrap();
    std::fs::create_dir(&admin).unwrap();
    assert!(prune_worktrees(&r.0, &context).is_err());
    std::fs::rename(r.0.join(".git"), r.0.join("old-git")).unwrap();
    r.git(&["init", "-q"]);
    assert!(create_worktree(&r.0, &context, &r.0.join("other"), "other", true).is_err());
    assert!(!r.0.join("other").exists());
}

#[test]
fn worktrees_prune_only_missing_unlocked_registrations() {
    let r = TempRepo::new();
    r.baseline();
    for name in ["missing", "locked", "live"] {
        let path = r.0.join(name);
        create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, name, true).unwrap();
    }
    r.git(&["worktree", "lock", r.0.join("locked").to_str().unwrap()]);
    std::fs::remove_dir_all(r.0.join("missing")).unwrap();
    std::fs::remove_dir_all(r.0.join("locked")).unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    assert!(entry(&context, &r.0.join("missing")).missing);
    prune_worktrees(&r.0, &context).unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    assert_eq!(context.entries.len(), 3);
    assert!(context.entries.iter().any(|e| e.path == r.0.join("locked")));
    assert!(r.0.join("live/file.txt").exists());
}

#[test]
fn worktrees_create_never_overwrites_existing_branch_or_destination() {
    let r = TempRepo::new();
    r.baseline();
    let context = prepare_worktrees(&r.0).unwrap();
    assert!(create_worktree(&r.0, &context, &r.0.join("new"), "main", true).is_err());
    assert!(create_worktree(&r.0, &context, &r.0.join("new"), "main", false).is_err());
    assert!(create_worktree(&r.0, &context, Path::new("relative"), "topic", true).is_err());
    assert!(create_worktree(&r.0, &context, &r.0.join("new"), "--detach", true).is_err());
    assert!(!r.0.join("new").exists());
    let occupied = r.0.join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("keep"), "saved").unwrap();
    assert!(create_worktree(&r.0, &context, &occupied, "topic", true).is_err());
    assert_eq!(std::fs::read_to_string(occupied.join("keep")).unwrap(), "saved");
}

#[test]
fn worktrees_create_requires_existing_parent_before_creating_a_branch() {
    let r = TempRepo::new();
    r.baseline();
    let path = r.0.join("missing-parent/linked");
    let error = create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, "topic", true).unwrap_err();
    assert!(error.contains("parent"), "{error}");
    assert!(!path.exists());
    assert!(!branches(&r.0).unwrap().contains(&"topic".into()));
}

#[test]
fn worktrees_existing_registration_must_still_name_its_own_checkout() {
    let r = TempRepo::new();
    r.baseline();
    let path = r.0.join("linked");
    create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, "topic", true).unwrap();
    std::fs::remove_file(path.join(".git")).unwrap();
    // An ancestor repository must not make a no-longer-linked folder look valid.
    assert!(prepare_worktrees(&r.0).is_err());
    let out = Command::new("git").arg("-C").arg(&path).args(["init", "-q"]).output().unwrap();
    assert!(out.status.success());
    // Nor may an unrelated repository occupying the registered path be opened/removed.
    assert!(prepare_worktrees(&r.0).is_err());
    assert_eq!(std::fs::read_to_string(path.join("file.txt")).unwrap(), "base\n");
}

#[test]
fn worktrees_reject_crossed_git_pointers_within_the_same_common_repository() {
    let r = TempRepo::new();
    r.baseline();
    let a = r.0.join("linked A 界");
    let b = r.0.join("linked B\nline");
    for (path, branch) in [(&a, "topic-a"), (&b, "topic-b")] {
        create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), path, branch, true).unwrap();
    }
    let context = prepare_worktrees(&r.0).unwrap();
    let selected = entry(&context, &a);
    let a_git = std::fs::read(a.join(".git")).unwrap();
    let b_git = std::fs::read(b.join(".git")).unwrap();
    std::fs::write(a.join(".git"), &b_git).unwrap();
    std::fs::write(b.join(".git"), &a_git).unwrap();

    // Both checkouts still discover their own paths and the same common repository.
    assert!(open_worktree(&r.0, &context, &selected).is_err(), "a pending Open must become stale");
    assert!(prepare_worktrees(&r.0).is_err(), "initial preparation must reject crossed registrations");
    assert!(prepare_worktrees(&a).is_err(), "the linked origin must reject the same corruption");
    assert_eq!(std::fs::read_to_string(a.join("file.txt")).unwrap(), "base\n");
    assert_eq!(std::fs::read_to_string(b.join("file.txt")).unwrap(), "base\n");

    std::fs::write(a.join(".git"), a_git).unwrap();
    std::fs::write(b.join(".git"), b_git).unwrap();
    let restored = prepare_worktrees(&r.0).unwrap();
    assert_eq!(open_worktree(&r.0, &restored, &entry(&restored, &a)).unwrap(), a);
}

#[test]
fn worktrees_registration_accepts_native_relative_backlinks() {
    let r = TempRepo::new();
    r.baseline();
    r.git(&["config", "worktree.useRelativePaths", "true"]);
    let path = r.0.join("relative 界\nline");
    create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, "relative-topic", true).unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    let selected = entry(&context, &path);
    assert_eq!(open_worktree(&r.0, &context, &selected).unwrap(), path);
    let linked_context = prepare_worktrees(&path).unwrap();
    assert!(entry(&linked_context, &path).current);
    remove_worktree(&r.0, &context, &selected).unwrap();
    assert!(!path.exists());
}

#[test]
fn worktrees_native_prunable_states_match_immediate_prune_and_locked_protections() {
    let r = TempRepo::new();
    r.baseline();
    for name in ["missing 界\nline", "future-index", "locked", "live"] {
        create_worktree(
            &r.0,
            &prepare_worktrees(&r.0).unwrap(),
            &r.0.join(name),
            name.replace([' ', '\n'], "-").as_str(),
            true,
        )
        .unwrap();
    }
    r.git(&["worktree", "lock", "--reason", "portable 界\nkeep", r.0.join("locked").to_str().unwrap()]);
    for name in ["missing 界\nline", "future-index", "locked"] {
        std::fs::remove_dir_all(r.0.join(name)).unwrap();
    }
    let future_index = r.0.join(".git/worktrees/future-index/index");
    std::fs::File::open(&future_index)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(86_400))
        .unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    assert_eq!(context.entries.len(), 5);
    let native = r.git(&["worktree", "list", "--porcelain", "-z", "--expire", "now"]);
    for selected in &context.entries {
        let prefix = format!("worktree {}\0", selected.path.display());
        let record = native.split("\0\0").find(|line| line.starts_with(&prefix)).unwrap();
        let reason = record.split('\0').find_map(|field| field.strip_prefix("prunable "));
        assert_eq!(selected.prunable.as_deref(), reason);
        if selected.path == r.0.join("missing 界\nline") || selected.path == r.0.join("future-index") {
            assert!(selected.missing && selected.locked.is_none());
            assert_eq!(selected.prunable.as_deref(), Some("gitdir file points to non-existent location"));
            assert!(open_worktree(&r.0, &context, selected).is_err());
            assert!(remove_worktree(&r.0, &context, selected).unwrap_err().contains("missing"));
        } else {
            assert!(selected.prunable.is_none());
        }
    }
    let locked = entry(&context, &r.0.join("locked"));
    assert!(locked.missing);
    assert_eq!(locked.locked.as_deref(), Some("portable 界\nkeep"));
    assert!(remove_worktree(&r.0, &context, &locked).unwrap_err().contains("locked"));
    prune_worktrees(&r.0, &context).unwrap();
    let remaining = prepare_worktrees(&r.0).unwrap();
    assert_eq!(
        remaining.entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
        [r.0.clone(), r.0.join("live"), r.0.join("locked")]
    );
    assert!(remaining.entries.iter().all(|e| e.prunable.is_none()));
    assert!(r.0.join(".git/worktrees/locked").is_dir());
    assert_eq!(std::fs::read_to_string(r.0.join("live/file.txt")).unwrap(), "base\n");
    assert!(branches(&r.0).unwrap().contains(&"future-index".into()));
}

#[test]
fn worktrees_checked_out_branch_creation_switch_and_deletion_keep_linked_content() {
    let r = TempRepo::new();
    r.baseline();
    let path = r.0.join("linked");
    create_worktree(&r.0, &prepare_worktrees(&r.0).unwrap(), &path, "topic", true).unwrap();
    let context = prepare_worktrees(&r.0).unwrap();
    let refused = r.0.join("refused");
    assert!(create_worktree(&r.0, &context, &refused, "topic", false).unwrap_err().contains("already"));
    assert!(switch(&r.0, "topic").unwrap_err().contains("already"));
    assert!(delete_branch(&r.0, "topic").is_err());
    assert!(!refused.exists());
    assert_eq!(r.snapshot().branch, "main");
    assert_eq!(std::fs::read_to_string(path.join("file.txt")).unwrap(), "base\n");
    assert_eq!(prepare_worktrees(&r.0).unwrap(), context);
    remove_worktree(&r.0, &context, &entry(&context, &path)).unwrap();
    delete_branch(&r.0, "topic").unwrap();
    assert_eq!(branches(&r.0).unwrap(), ["main"]);
}
