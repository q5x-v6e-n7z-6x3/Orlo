use super::*;
use orlo_project::encoding::{encode, Encoding};
use orlo_project::watch::{check, Disk};

#[test]
fn loading_rejects_devices_and_directories() {
    for path in ["/dev/null", "/dev"] {
        assert!(Loaded::read(Path::new(path)).is_err(), "{path}");
    }
}

#[test]
fn a_file_replaced_by_a_pipe_during_load_does_not_wait_for_a_writer() {
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Duration;

    let path = fresh_dir("orlo-buffer-files").join(format!("orlo-loaded-pipe-{}", std::process::id()));
    std::fs::write(&path, "regular file").unwrap();
    let pipe = path.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        MID_READ.set(Some(|p| {
            std::fs::remove_file(p).unwrap();
            assert!(std::process::Command::new("mkfifo").arg(p).status().unwrap().success());
        }));
        let _ = tx.send(Loaded::read(&pipe).map(|_| ()));
    });
    let result = rx.recv_timeout(Duration::from_secs(5));
    if result.is_err() {
        // a read stuck opening the pipe is let go (a writer opens it) before the test fails
        drop(std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(&path));
    }
    assert!(matches!(result, Ok(Err(e)) if e.kind() == io::ErrorKind::InvalidInput));
    reader.join().unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn failed_temp_creation_keeps_the_existing_file() {
    let dir = fresh_dir("orlo-create-collision");
    let target = dir.join("document.txt");
    std::fs::write(&target, "original").unwrap();
    BEFORE_CREATE.set(Some(|tmp| std::fs::write(tmp, "someone else's data").unwrap()));
    let error = write_atomic(&target, "replacement").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    let temps: Vec<_> =
        std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path != &target).collect();
    assert_eq!(temps.len(), 1, "a failed create must not delete another file");
    assert_eq!(std::fs::read_to_string(&temps[0]).unwrap(), "someone else's data");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_private_write_is_owner_only_before_any_text_is_written() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = fresh_dir("orlo-private-write-mode");
    let path = dir.join("workspace");
    std::fs::write(&path, "old").unwrap();
    set_mode(&path, 0o644).unwrap();
    write_safely_with(
        &path,
        &|file| {
            assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
            file.write_all(b"private text")
        },
        true,
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "private text");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_private_write_that_cannot_stage_keeps_the_existing_file() {
    let dir = fresh_dir("orlo-private-write-staging");
    let path = dir.join("workspace");
    std::fs::write(&path, "old private text").unwrap();
    FAULT.set(Some(("copy", io::ErrorKind::PermissionDenied)));
    let result = write_atomic_private(&path, "new private text");
    FAULT.set(None);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old private text");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(target_os = "macos")]
fn a_private_write_has_no_inherited_acl_before_writing_text() {
    use std::io::Write;
    let dir = fresh_dir("orlo-private-write-acl");
    let path = dir.join("workspace");
    std::fs::write(&path, "old").unwrap();
    sh("chmod", &["+a", "everyone allow read,file_inherit,directory_inherit"], &dir);
    let result = write_safely_with(
        &path,
        &|file| {
            let stage = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).find(|p| p != &path).unwrap();
            let acl = sh("ls", &["-le"], &stage);
            assert_eq!(acl.lines().count(), 1, "{acl}");
            file.write_all(b"private text")
        },
        true,
    );
    sh("chmod", &["-RN"], &dir);
    result.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "private text");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(target_os = "macos")]
fn a_private_stage_inherits_no_acl_at_creation() {
    let dir = fresh_dir("orlo-private-stage-acl");
    let path = dir.join("workspace");
    sh("chmod", &["+a", "everyone allow read,file_inherit,directory_inherit"], &dir);
    AFTER_CREATE.set(Some(|stage| {
        let acl = sh("ls", &["-le"], stage);
        assert_eq!(acl.lines().count(), 1, "{acl}");
    }));
    let result = write_atomic_private(&path, "private text");
    sh("chmod", &["-RN"], &dir);
    result.unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_private_write_that_cannot_secure_permissions_keeps_the_existing_file() {
    let dir = fresh_dir("orlo-private-write-permissions");
    let path = dir.join("workspace");
    std::fs::write(&path, "old private text").unwrap();
    FAULT.set(Some(("chmod", io::ErrorKind::PermissionDenied)));
    let result = write_atomic_private(&path, "new private text");
    FAULT.set(None);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old private text");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_private_recovery_copy_refuses_existing_files_and_symlinks() {
    let dir = fresh_dir("orlo-private-recovery-collision");
    let source = dir.join("source");
    let target = dir.join("backup");
    let outside = dir.join("outside");
    std::fs::write(&source, b"raw private\xff\0bytes").unwrap();
    std::fs::write(&outside, "outside text").unwrap();
    for link in [false, true] {
        if link {
            std::os::unix::fs::symlink(&outside, &target).unwrap();
        } else {
            std::fs::write(&target, "previous backup").unwrap();
        }
        assert_eq!(copy_private(&source, &target).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&source).unwrap(), b"raw private\xff\0bytes");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside text");
        assert_eq!(std::fs::symlink_metadata(&target).unwrap().is_symlink(), link);
        if !link {
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "previous backup");
        }
        std::fs::remove_file(&target).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_private_recovery_copy_refuses_a_pipe_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let dir = fresh_dir("orlo-private-recovery-pipe");
    let source = dir.join("source");
    let target = dir.join("backup");
    let name = std::ffi::CString::new(source.as_os_str().as_bytes()).unwrap();
    // SAFETY: a live NUL-terminated path in this test's directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(copy_private(&source, &target).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn securing_a_private_recovery_file_does_not_follow_a_symlink() {
    let dir = fresh_dir("orlo-private-recovery-symlink");
    let outside = dir.join("outside");
    let link = dir.join("link");
    std::fs::write(&outside, "outside text").unwrap();
    set_mode(&outside, 0o640).unwrap();
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert_eq!(make_private_file(&link).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(mode_of(&outside), 0o640);
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside text");
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn write_atomic_replaces_the_target_of_a_symlink_and_leaves_no_temp_file() {
    let dir = fresh_dir("orlo-write-atomic");
    let (real, link) = (dir.join("real"), dir.join("link"));
    std::fs::write(&real, "old").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    write_atomic(&link, "new").unwrap();
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
    assert!(link.symlink_metadata().unwrap().is_symlink());
    write_atomic(&dir.join("fresh"), "x").unwrap();
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    assert_eq!(names, ["fresh", "link", "real"]);
}

#[test]
fn resolve_spells_a_path_through_a_linked_folder_the_way_opening_it_does() {
    let dir = fresh_dir("orlo-resolve");
    std::fs::create_dir(dir.join("real")).unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
    let real = dir.join("real").canonicalize().unwrap();
    assert_eq!(resolve(&dir.join("link/new.txt")), real.join("new.txt"));
    std::fs::write(real.join("old.txt"), "").unwrap();
    assert_eq!(resolve(&dir.join("link/old.txt")), real.join("old.txt"));
    assert_eq!(resolve(Path::new("/nope/at/all")), Path::new("/nope/at/all"));
}

#[test]
fn a_hard_linked_file_is_written_in_place_so_both_names_see_it() {
    use std::os::unix::fs::MetadataExt;
    let dir = fresh_dir("orlo-write-hardlink");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, "old").unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    let ino = std::fs::metadata(&a).unwrap().ino();
    write_atomic(&a, "new").unwrap();
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "new");
    assert_eq!(std::fs::metadata(&a).unwrap().ino(), ino);
}

#[test]
fn a_save_failing_midway_or_on_a_full_disk_leaves_the_file_whole() {
    let dir = fresh_dir("orlo-save-midway");
    let p = dir.join("f.txt");
    for step in ["write", "copy"] {
        std::fs::write(&p, "old").unwrap();
        let mut s = EditorState::load(&p).unwrap();
        s.insert("new ");
        FAULT.set(Some((step, io::ErrorKind::StorageFull)));
        let r = s.save();
        FAULT.set(None);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::StorageFull, "{step}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "old", "{step}: not in place");
        assert!(s.doc.dirty);
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "{step}: no temp file left");
    }
}

#[test]
fn a_failed_save_that_never_touched_the_file_still_sees_an_outside_change() {
    let dir = fresh_dir("orlo-save-untouched");
    let p = dir.join("f.txt");
    std::fs::write(&p, "old").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    std::fs::write(&p, "from another app").unwrap();
    s.insert("mine ");
    set_mode(&p, 0o444).unwrap();
    set_mode(&dir, 0o555).unwrap();
    let r = s.save();
    set_mode(&dir, 0o755).unwrap();
    assert!(r.is_err());
    assert!(matches!(check(&p, s.doc.disk, s.doc.deleted), Disk::Changed(_)));
}

#[test]
fn a_write_while_a_file_is_read_is_seen_at_the_next_poll() {
    // the stamp comes before the read; stamped after, the text from before
    // the write would take the stamp from after it, and the change would be lost
    let dir = fresh_dir("orlo-mid-read");
    let p = dir.join("f.txt");
    std::fs::write(&p, "old").unwrap();
    MID_READ.set(Some(|p| std::fs::write(p, "new, and longer").unwrap()));
    let l = Loaded::read(&p).unwrap();
    // read after the write, with the stamp from before it: a needless reload at worst
    let polled = check(&p, l.disk, false);
    assert!(matches!(polled, Disk::Changed(_)), "{polled:?}");
}

#[test]
fn temp_files_a_dead_orlo_left_are_swept_and_a_live_ones_kept() {
    let dir = fresh_dir("orlo-temp-sweep");
    let f = dir.join("a.txt");
    // a PID no process has (macOS stops below 100000), this process, and another file's
    let dead = dir.join("a.txt.orlo-99999-3.tmp");
    let live = dir.join(format!("a.txt.orlo-{}-4.tmp", std::process::id()));
    let other = dir.join("b.txt.orlo-99999-1.tmp");
    for p in [&f, &dead, &live, &other] {
        std::fs::write(p, "").unwrap();
    }
    assert_eq!(temp_pid("a.txt.orlo-99999-3.tmp"), Some(99999));
    assert_eq!(temp_pid("notes.orlo-old.tmp"), None);
    sweep_temp_files(std::slice::from_ref(&f));
    assert!(!dead.exists() && live.exists() && other.exists() && f.exists());
}

#[test]
fn a_temp_file_another_users_process_made_is_kept() {
    let dir = fresh_dir("orlo-temp-sweep-eperm");
    let f = dir.join("a.txt");
    // PID 1 is launchd's, root's: `kill -0` is refused, not told it is gone
    let theirs = dir.join("a.txt.orlo-1-2.tmp");
    for p in [&f, &theirs] {
        std::fs::write(p, "").unwrap();
    }
    sweep_temp_files(std::slice::from_ref(&f));
    assert!(theirs.exists());
}

#[test]
fn write_atomic_keeps_extended_attributes() {
    let dir = fresh_dir("orlo-write-xattr");
    let f = dir.join("tagged.txt");
    std::fs::write(&f, "old").unwrap();
    sh("xattr", &["-w", "dev.orlo.test", "kept"], &f);
    write_atomic(&f, "new").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
    assert_eq!(sh("xattr", &["-p", "dev.orlo.test"], &f), "kept");
}

#[test]
fn a_save_keeps_the_mode_an_xattr_and_an_acl_without_reading_the_old_text() {
    use std::os::unix::fs::MetadataExt;
    let dir = fresh_dir("orlo-write-metadata");
    let acl = "user:nobody allow read";
    // -n prints nobody's UUID even when directory-service name lookup is busy.
    let numeric_acl = "FFFFEEEE-DDDD-CCCC-BBBB-AAAAFFFFFFFE allow read";
    // an ACL entry, where the volume takes them: else that check alone is skipped
    let set_acl = |f: &Path| {
        let out = std::process::Command::new("chmod").args(["+a", acl]).arg(f).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
            eprintln!("skipping the ACL check: chmod +a failed: {err}");
        }
        out.status.success()
    };
    // an odd mode, an extended attribute and an ACL: all on the new file renamed in
    let f = dir.join("f.txt");
    std::fs::write(&f, "old").unwrap();
    sh("xattr", &["-w", "dev.orlo.test", "kept"], &f);
    let acls = set_acl(&f);
    set_mode(&f, 0o640).unwrap();
    let ino = std::fs::metadata(&f).unwrap().ino();
    write_atomic(&f, "new").unwrap();
    let m = std::fs::metadata(&f).unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
    assert_ne!(m.ino(), ino, "renamed in, not written in place");
    assert_eq!(m.mode() & 0o777, 0o640);
    assert_eq!(sh("xattr", &["-p", "dev.orlo.test"], &f), "kept");
    if acls {
        assert!(sh("ls", &["-len"], &f).contains(numeric_acl), "{}", sh("ls", &["-len"], &f));
    }
    // write-only: neither the old text nor the extended attributes can be read; the mode
    // and the ACL are still kept
    let w = dir.join("w.txt");
    std::fs::write(&w, "old").unwrap();
    let acls = set_acl(&w);
    set_mode(&w, 0o220).unwrap();
    let ino = std::fs::metadata(&w).unwrap().ino();
    write_atomic(&w, "new").unwrap();
    let m = std::fs::metadata(&w).unwrap();
    assert_ne!(m.ino(), ino, "renamed in, not written in place");
    assert_eq!(m.mode() & 0o777, 0o220);
    if acls {
        assert!(sh("ls", &["-len"], &w).contains(numeric_acl), "{}", sh("ls", &["-len"], &w));
    }
    set_mode(&w, 0o644).unwrap();
    assert_eq!(std::fs::read_to_string(&w).unwrap(), "new");
    for p in [&f, &w] {
        sh("chmod", &["-N"], p);
    }
}

#[test]
fn a_save_of_a_finder_locked_file_fails_and_leaves_it_whole() {
    let dir = fresh_dir("orlo-write-locked");
    let f = dir.join("locked.txt");
    std::fs::write(&f, "old").unwrap();
    sh("chflags", &["uchg"], &f);
    let r = write_atomic(&f, "new");
    // unlocked before anything is checked, so the folder can go; a temp file left too
    sh("chflags", &["-R", "nouchg"], &dir);
    let e = r.unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "old");
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names, ["locked.txt"]);
}

#[test]
fn a_save_of_a_compressed_file_reads_back_as_written() {
    let dir = fresh_dir("orlo-write-compressed");
    let plain = dir.join("plain.txt");
    std::fs::write(&plain, "old line\n".repeat(10_000)).unwrap();
    let f = dir.join("f.txt");
    let ditto = std::process::Command::new("ditto").arg("--hfsCompression").arg(&plain).arg(&f).output().unwrap();
    let err = String::from_utf8_lossy(&ditto.stderr);
    assert!(ditto.status.success(), "ditto --hfsCompression: {err}");
    assert!(sh("ls", &["-lO"], &f).contains("compressed"), "{}", sh("ls", &["-lO"], &f));
    let text = "new line\n".repeat(20_000);
    write_atomic(&f, &text).unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), text);
    // and one cut down to nothing
    write_atomic(&f, "").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "");
}

#[test]
fn a_save_that_empties_a_file_moves_its_modified_time() {
    use std::time::Duration;
    let dir = fresh_dir("orlo-write-emptied");
    let f = dir.join("f.txt");
    std::fs::write(&f, "old").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::File::options().write(true).open(&f).unwrap().set_modified(old).unwrap();
    write_atomic(&f, "").unwrap();
    let now = std::fs::metadata(&f).unwrap().modified().unwrap();
    assert!(now > old + Duration::from_secs(86_400), "{now:?}");
}

#[test]
fn a_file_in_a_read_only_folder_is_written_in_place() {
    let dir = fresh_dir("orlo-write-ro-dir");
    let f = dir.join("f.txt");
    std::fs::write(&f, "old").unwrap();
    set_mode(&dir, 0o555).unwrap();
    let r = write_atomic(&f, "new");
    set_mode(&dir, 0o755).unwrap();
    r.unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn write_atomic_keeps_the_mode_and_leaves_other_temp_files_alone() {
    let dir = fresh_dir("orlo-write-atomic-mode");
    let script = dir.join("run.sh");
    std::fs::write(&script, "old").unwrap();
    set_mode(&script, 0o755).unwrap();
    std::fs::write(dir.join("run.sh.tmp"), "someone else's").unwrap();
    write_atomic(&script, "new").unwrap();
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "new");
    assert_eq!(mode_of(&script), 0o755);
    assert_eq!(std::fs::read_to_string(dir.join("run.sh.tmp")).unwrap(), "someone else's");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
}

#[test]
fn scratch_buffers_never_get_dirty() {
    let mut s = EditorState::from_text("");
    s.doc.scratch = true;
    s.insert("x");
    assert!(!s.doc.dirty);
    s.undo();
    assert!(!s.doc.dirty, "undo neither");
}

#[test]
fn appending_to_a_saved_scratch_buffer_asks_to_save_it_again() {
    let p = fresh_dir("orlo-buffer-files").join("orlo-scratch-appended.txt");
    let mut s = EditorState::from_text("log\n");
    s.doc.scratch = true;
    s.append("streamed\n");
    assert!(!s.doc.dirty, "results stay clean while scratch");
    s.save_to(p.clone()).unwrap();
    assert!(!s.doc.dirty);
    s.append("more\n");
    assert!(s.doc.dirty, "the appended text is not on disk");
    let _ = std::fs::remove_file(p);
}

#[test]
fn a_scratch_buffer_saved_as_a_file_is_an_ordinary_buffer() {
    let p = fresh_dir("orlo-buffer-files").join("orlo-scratch-saved.txt");
    let mut s = EditorState::from_text("results");
    s.doc.scratch = true;
    s.save_to(p.clone()).unwrap();
    s.insert("x");
    assert!(s.doc.dirty, "its edits now ask to be saved");
    let _ = std::fs::remove_file(p);
}

#[test]
fn append_is_not_an_undo_step() {
    let mut v = views("");
    let mut a = v.a();
    a.doc.scratch = true;
    a.set_cursor(0);
    a.append("a");
    a.append("b\nc");
    assert_eq!(a.text(), "ab\nc");
    assert_eq!(a.cursor(), 0, "the caret stays");
    assert_eq!(v.b().text(), "ab\nc", "a second view follows");
    let mut a = v.a();
    a.undo();
    assert_eq!(a.text(), "ab\nc");
    a.set_cursor(5);
    a.insert("!");
    a.append("d");
    a.undo();
    assert_eq!(a.text(), "ab\nc!d", "undo never drops appended results");
}

#[test]
fn own_save_is_not_an_external_change() {
    let p = fresh_dir("orlo-buffer-files").join("orlo-own-save.txt");
    std::fs::write(&p, "abc").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    assert_eq!(check(&p, s.doc.disk, false), Disk::Same);
    s.insert("more ");
    s.save_to(p.clone()).unwrap();
    assert_eq!(check(&p, s.doc.disk, false), Disk::Same);
    let _ = std::fs::remove_file(p);
}

#[test]
fn reload_keeps_the_caret_on_the_same_line() {
    let mut s = EditorState::from_text("l1\nl2\nl3\nl4");
    s.set_cursor(7); // l3
    s.doc.deleted = true;
    let mut disk = loaded("l1\nl2\nl3\nl4\nl5");
    disk.disk = orlo_project::watch::stamp(&std::env::current_exe().unwrap());
    reload(&mut s, disk);
    assert_eq!(s.point(s.cursor()), (2, 1));
    assert!(!s.doc.deleted, "a reloaded file is back");
    assert!(s.doc.disk.is_some(), "the reload records the disk stamp");
}

#[test]
fn reload_from_is_one_clean_undo_step() {
    let mut s = EditorState::from_text("old text");
    s.set_cursor(4);
    reload(&mut s, loaded("new text"));
    assert_eq!(s.text(), "new text");
    assert!(!s.doc.dirty);
    assert_eq!(s.cursor(), 4, "carets map through the reload");
    s.undo();
    assert_eq!(s.text(), "old text");
    assert!(s.doc.dirty);
}

#[test]
fn enabled_save_captures_original_bytes_and_aborts_if_history_fails() {
    let dir = fresh_dir("orlo-backup-save");
    let p = dir.join("source.txt");
    let old = encode("old 🦀\r\n", Encoding::Utf16Le, false).unwrap();
    std::fs::write(&p, &old).unwrap();
    let mut s = EditorState::from_loaded(&p, Loaded::read_encoding(&p, Some(Encoding::Utf16Le)).unwrap());
    s.doc.history_enabled = true;
    s.set_encoding(Encoding::Utf16Be, true);
    s.insert("new ");
    let history = dir.canonicalize().unwrap().join("history");
    orlo_project::file_history::with_root(history.clone(), || s.save().unwrap());
    let versions = orlo_project::file_history::list_in(&history, &p).unwrap();
    assert_eq!(versions.len(), 1);
    let saved = orlo_project::file_history::read_in(&history, &p, &versions[0].id).unwrap();
    assert_eq!((saved.bytes, saved.version.encoding), (old, Encoding::Utf16Le));
    let current = std::fs::read(&p).unwrap();
    let bad = dir.join("not-a-directory");
    std::fs::write(&bad, b"blocked").unwrap();
    s.insert("do not publish ");
    assert!(orlo_project::file_history::with_root(bad, || s.save()).is_err());
    assert_eq!(std::fs::read(&p).unwrap(), current);
    assert!(s.unsaved());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn closed_pipeline_keeps_target_encoding() {
    let dir = fresh_dir("orlo-encoding-tools");
    let p = dir.join("sort.txt");
    for encoding in [Encoding::Utf16Le, Encoding::Utf32Le, Encoding::Utf32Be] {
        std::fs::write(&p, encode("b🦀\r\na\r\n", encoding, true).unwrap()).unwrap();
        let pipeline = crate::search::pipeline::parse(r#"{"steps":[{"op":"sort_lines"}]}"#).unwrap();
        crate::search::pipeline::run_file(&p, &pipeline).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), encode("a\r\nb🦀\r\n", encoding, true).unwrap());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stored_encoding_is_only_a_bomless_fallback() {
    let p = fresh_dir("orlo-buffer-files").join(format!("orlo-encoding-hint-{}", std::process::id()));
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
        std::fs::write(&p, encode("café 🦀\n", encoding, true).unwrap()).unwrap();
        let loaded = Loaded::read_with_hint(&p, Some(Encoding::Utf16Le)).unwrap();
        assert_eq!(loaded.rope.to_string(), "café 🦀\n");
        assert_eq!(loaded.encoding, encoding);
        assert!(!loaded.read_only);
    }
    std::fs::write(&p, encode("café 🦀\n", Encoding::Utf16Le, false).unwrap()).unwrap();
    assert_eq!(Loaded::read_with_hint(&p, Some(Encoding::Utf16Le)).unwrap().rope.to_string(), "café 🦀\n");
    std::fs::remove_file(p).unwrap();
}

#[test]
fn save_as_cannot_overwrite_a_malformed_original_or_alias() {
    use std::os::unix::fs::symlink;
    let dir = fresh_dir("orlo-encoding-protect");
    let original = dir.join("source.txt");
    let symbolic = dir.join("symbolic.txt");
    let hard = dir.join("hard.txt");
    let bytes = [0xff, 0xfe, 0, 0xd8];
    std::fs::write(&original, bytes).unwrap();
    symlink(&original, &symbolic).unwrap();
    std::fs::hard_link(&original, &hard).unwrap();
    for target in [&original, &symbolic, &hard] {
        let mut state = EditorState::load(&original).unwrap();
        assert!(state.save_to(target.clone()).is_err(), "must protect {target:?}");
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert!(state.doc.read_only);
    }
    let mut state = EditorState::load(&original).unwrap();
    state.save_to(dir.join("copy.txt")).unwrap();
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn search_decodes_utf16_without_treating_its_code_units_as_binary() {
    let p = fresh_dir("orlo-buffer-files").join(format!("orlo-encoding-search-{}", std::process::id()));
    std::fs::write(&p, encode("a🦀\r\nb", Encoding::Utf16Be, true).unwrap()).unwrap();
    let file = crate::search::search::read_file(&p).unwrap();
    assert!(!file.binary && !file.lossy);
    assert_eq!((file.text.as_str(), file.line_ending, file.bom), ("a🦀\nb", "\r\n", true));
    std::fs::remove_file(p).unwrap();
}

#[test]
fn utf16_bom_roundtrips_unicode_and_crlf() {
    let orders = [("le", u16::to_le_bytes as fn(_) -> _), ("be", u16::to_be_bytes)];
    for (suffix, unit_bytes) in orders {
        let p = fresh_dir("orlo-buffer-files").join(format!("orlo-encoding-{}-{suffix}.txt", std::process::id()));
        // the byte order mark, then the text
        let mut bytes = unit_bytes(0xfeff).to_vec();
        bytes.extend("a🦀\r\nb\r\n".encode_utf16().flat_map(unit_bytes));
        std::fs::write(&p, &bytes).unwrap();
        let mut s = EditorState::load(&p).unwrap();
        assert_eq!(s.text(), "a🦀\nb\n");
        assert!(!s.doc.read_only);
        assert_eq!(s.doc.line_ending, "\r\n");
        s.save().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        std::fs::remove_file(p).unwrap();
    }
}

#[test]
fn crlf_is_normalised_on_load_and_restored_on_save() {
    let p = fresh_dir("orlo-buffer-files").join("orlo_crlf.txt");
    std::fs::write(&p, "a\r\nb\r\n").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    assert_eq!(s.text(), "a\nb\n");
    assert_eq!(s.doc.line_ending, "\r\n");
    s.view.selections = vec![Selection::caret(1)];
    s.insert("x");
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"ax\r\nb\r\n");
    s.doc.line_ending = "\n";
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"ax\r\nb\r\n");
    s.convert_line_endings("\n");
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"ax\nb\n");
    s.undo();
    assert_eq!(s.doc.bytes_as_saved(None).unwrap(), b"ax\r\nb\r\n");
}

#[test]
fn load_non_utf8_is_lossy_and_read_only() {
    let p = fresh_dir("orlo-buffer-files").join("orlo_bad.txt");
    std::fs::write(&p, [0x66, 0xff, 0x6f]).unwrap();
    let s = EditorState::load(&p).unwrap();
    assert!(s.doc.read_only);
    assert_eq!(s.text(), "f\u{fffd}o");
}

#[test]
fn save_without_path_errors_and_save_to_clears_dirty() {
    let mut s = st("x", 1);
    s.insert("y");
    assert!(s.save().is_err());
    let p = fresh_dir("orlo-buffer-files").join("orlo_save.txt");
    s.save_to(p.clone()).unwrap();
    assert!(!s.doc.dirty);
    assert_eq!(std::fs::read_to_string(p).unwrap(), "xy");
}

#[test]
fn legacy_conversion_failure_preserves_source_destination_and_history() {
    let dir = fresh_dir("f01-save-lossless");
    let path = dir.join("source.txt");
    let copy = dir.join("copy.txt");
    let original = b"caf\xe9\r\n";
    std::fs::write(&path, original).unwrap();
    std::fs::write(&copy, b"keep destination").unwrap();
    let mut state = EditorState::from_loaded(&path, Loaded::read_encoding(&path, Some(Encoding::Windows1252)).unwrap());
    state.doc.history_enabled = true;
    state.view.selections = vec![Selection::caret(state.doc.rope.len_chars())];
    state.insert("é🦀");
    let before = (state.doc.save_point(), state.doc.version, state.doc.path.clone(), state.doc.disk);
    let history = dir.canonicalize().unwrap().join("history");
    for destination in [&path, &copy] {
        let error =
            orlo_project::file_history::with_root(history.clone(), || state.save_to(destination.clone())).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("'🦀' (U+1F980) at line 2, column 2"), "{error}");
        assert_eq!((state.doc.save_point(), state.doc.version, state.doc.path.clone(), state.doc.disk), before);
        assert!(state.unsaved());
        assert_eq!(state.doc.disk_encoding, Encoding::Windows1252);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(&copy).unwrap(), b"keep destination");
        assert!(!history.exists(), "conversion must precede history capture");
    }
    state.undo();
    assert_eq!(state.text(), "café\n");
    state.redo();
    assert_eq!(state.text(), "café\né🦀");
    state.set_encoding(Encoding::Utf32Le, true);
    orlo_project::file_history::with_root(history.clone(), || state.save()).unwrap();
    let versions = orlo_project::file_history::list_in(&history, &path).unwrap();
    assert_eq!(versions.len(), 1);
    let old = orlo_project::file_history::read_in(&history, &path, &versions[0].id).unwrap();
    assert_eq!(old.bytes, original);
    assert_eq!(old.version.encoding, Encoding::Windows1252);
    assert_eq!(Loaded::read(&path).unwrap().rope.to_string(), "café\né🦀");
    assert_eq!(state.doc.disk_encoding, Encoding::Utf32Le);
    assert!(!state.unsaved());
}

#[test]
fn named_legacy_search_hint_is_explicit_and_external_bom_wins() {
    let dir = fresh_dir("f01-search-hints");
    let path = dir.join("source.txt");
    std::fs::write(&path, b"caf\xe9\r\n").unwrap();
    let unknown = crate::search::search::read_file(&path).unwrap();
    assert!(unknown.lossy);
    let known = crate::search::search::read_file_with_hint(&path, Some(Encoding::Windows1252)).unwrap();
    assert_eq!(known.text, "café\n");
    assert_eq!(known.line_ending, "\r\n");
    assert!(!known.lossy && !known.binary);
    std::fs::write(&path, [0, 0, 0xfe, 0xff, 0, 0, 0, 0x41]).unwrap();
    let changed = crate::search::search::read_file_with_hint(&path, Some(Encoding::Windows1252)).unwrap();
    assert_eq!(changed.text, "A");
    assert_eq!(changed.encoding, Encoding::Utf32Be);
}

#[test]
fn save_receipt_does_not_bless_external_replacement_after_publication() {
    for linked in [false, true] {
        let dir = fresh_dir(if linked { "orlo-save-receipt-linked" } else { "orlo-save-receipt" });
        let path = dir.join("source");
        std::fs::write(&path, "old").unwrap();
        if linked {
            std::fs::hard_link(&path, dir.join("alias")).unwrap();
        }
        let mut state = EditorState::from_loaded(&path, Loaded::read(&path).unwrap());
        state.insert("mine");
        let request = state.doc.capture_save(path.clone(), None).unwrap();
        orlo_project::io::AFTER_PUBLISH.set(Some(|path| {
            let replacement = path.with_extension("external");
            std::fs::write(&replacement, "external").unwrap();
            std::fs::rename(replacement, path).unwrap();
        }));
        let written = request.write().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(written.disk, None, "an unrelated file must never become the autosave baseline");
    }
}

#[test]
fn n25_mixed_file_roundtrip_and_line_edit_keep_exact_boundaries() {
    let dir = fresh_dir("orlo-n25-roundtrip");
    let p = dir.join("mixed.txt");
    let raw = "\u{feff}a\r\nb\nc\rd";
    std::fs::write(&p, raw).unwrap();
    let mut s = EditorState::load(&p).unwrap();
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), raw.as_bytes());
    assert_eq!(s.text(), "a\nb\nc\nd");
    s.view.selections = vec![Selection::new(2, 3)];
    s.insert("B");
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), "\u{feff}a\r\nB\nc\rd".as_bytes());
}
