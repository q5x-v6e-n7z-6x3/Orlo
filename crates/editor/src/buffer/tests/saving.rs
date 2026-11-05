use super::*;
use orlo_project::encoding::Encoding;

fn file(name: &str, text: &str) -> (PathBuf, EditorState) {
    let dir = fresh_dir(name);
    let p = dir.join("f.txt");
    std::fs::write(&p, text).unwrap();
    let s = EditorState::load(&p).unwrap();
    (p, s)
}

fn at_end(s: &mut EditorState) {
    let end = s.len();
    s.set_cursor(end);
}

#[test]
fn a_request_can_be_written_on_another_thread() {
    fn send<T: Send + 'static>(_: &T) {}
    let (p, mut s) = file("orlo-save-send", "a");
    let req = s.doc.capture_save(p.clone(), None).unwrap();
    send(&req);
    let written = std::thread::spawn(move || req.write()).join().unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a");
    assert_eq!(written.bytes, ByteOffset(1));
}

#[test]
fn typing_after_the_capture_is_unsaved_and_undoing_it_comes_back_clean() {
    let (p, mut s) = file("orlo-save-typing-model", "a\n");
    at_end(&mut s);
    s.type_text("b");
    let req = s.doc.capture_save(p.clone(), None).unwrap();
    assert_eq!(req.chars, CharOffset(3));
    // typing while the write is under way: a new undo step, not the captured one's
    s.type_text("c");
    let written = req.write().unwrap();
    s.doc.finish_save(&req, &written);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\nb");
    assert!(s.doc.dirty, "the c is unsaved");
    assert_eq!(s.doc.disk, written.disk);
    s.undo();
    assert_eq!(s.text(), "a\nb");
    assert!(!s.doc.dirty, "back at what the file holds");
    s.redo();
    assert!(s.doc.dirty);
    s.undo();
    s.undo();
    assert!(s.doc.dirty, "before the save is unsaved too");
    assert_eq!(s.doc.saved_text().unwrap().to_string(), "a\nb");
}

#[test]
fn an_edit_after_undoing_past_the_capture_leaves_no_saved_point() {
    let (p, mut s) = file("orlo-save-branch-model", "a");
    at_end(&mut s);
    s.insert("b");
    let req = s.doc.capture_save(p.clone(), None).unwrap();
    s.undo();
    s.insert("z");
    let written = req.write().unwrap();
    s.doc.finish_save(&req, &written);
    assert!(s.doc.dirty);
    // the text written was on the branch the edit discarded
    for _ in 0..3 {
        s.undo();
        assert!(s.doc.dirty);
    }
    for _ in 0..3 {
        s.redo();
        assert!(s.doc.dirty);
    }
    assert!(s.doc.saved_text().is_none());
}

#[test]
fn the_same_text_written_another_way_is_not_the_saved_point() {
    for change in ["encoding", "bom", "line ending"] {
        let (p, mut s) = file(&format!("orlo-save-format-model-{}", change.len()), "a\n");
        s.insert("b");
        let req = s.doc.capture_save(p.clone(), None).unwrap();
        match change {
            "encoding" => s.set_encoding(Encoding::Utf16Le, false),
            "bom" => s.set_encoding(Encoding::Utf8, true),
            _ => s.set_line_ending("\r\n"),
        }
        let written = req.write().unwrap();
        s.doc.finish_save(&req, &written);
        assert_eq!(std::fs::read(&p).unwrap(), b"ba\n", "{change}: as captured");
        assert!(s.doc.dirty, "{change}: not yet written that way");
        assert_eq!(s.doc.disk_encoding, Encoding::Utf8, "{change}: what is on disk");
    }
}

#[test]
fn the_saved_point_is_the_text_state_not_the_undo_depth() {
    let (p, mut s) = file("orlo-save-depth", "");
    s.insert("a");
    s.insert("\n");
    s.save().unwrap();
    // the same depth on another branch is another text
    s.undo();
    s.insert("x\n");
    assert!(s.doc.dirty);
    s.undo();
    s.redo();
    assert!(s.doc.dirty, "x\\n is not what was saved");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\n");
}

#[test]
fn a_write_that_fails_leaves_the_document_unsaved_with_the_stamp_it_left() {
    let (p, mut s) = file("orlo-save-fails-model", "old");
    let dir = p.parent().unwrap().to_path_buf();
    s.insert("new ");
    let before = s.doc.disk;
    let req = s.doc.capture_save(p.clone(), None).unwrap();
    set_mode(&p, 0o444).unwrap();
    set_mode(&dir, 0o555).unwrap();
    let failure = req.write().unwrap_err();
    set_mode(&dir, 0o755).unwrap();
    s.doc.fail_save(&req, &failure);
    assert!(s.doc.dirty);
    assert_eq!(failure.disk, None, "untouched");
    assert_eq!(s.doc.disk, before, "a change from outside is still asked about");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
}

#[test]
fn a_lossy_documents_original_is_refused_before_anything_is_captured() {
    let dir = fresh_dir("orlo-save-lossy-model");
    let p = dir.join("f.txt");
    std::fs::write(&p, b"a\xff").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    assert!(s.doc.read_only);
    let typing = s.doc.typing.get();
    assert!(s.doc.capture_save(p.clone(), None).is_err());
    assert_eq!(s.doc.typing.get(), typing, "nothing taken");
    assert!(s.doc.capture_save(dir.join("copy.txt"), None).is_ok());
}

#[test]
fn a_capture_closes_the_typing_group() {
    let (p, mut s) = file("orlo-save-typing-group", "");
    s.type_text("a");
    let req = s.doc.capture_save(p, None).unwrap();
    s.type_text("b");
    s.undo();
    assert_eq!(s.text(), "a", "b is a step of its own");
    let written = req.write().unwrap();
    s.doc.finish_save(&req, &written);
    assert!(!s.doc.dirty);
}

#[test]
fn saves_with_file_history_on_written_at_once_both_land() {
    use orlo_project::file_history::{list_in, with_root};
    let dir = fresh_dir("orlo-save-history-together").canonicalize().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writers: Vec<_> = (0..2)
        .map(|n| {
            let (p, mut s) = file(&format!("orlo-save-history-together-{n}"), "a\n");
            s.doc.history_enabled = true;
            let requests: Vec<_> = (0..20)
                .map(|round| {
                    s.insert(&format!("{round}"));
                    s.doc.capture_save(p.clone(), None).unwrap()
                })
                .collect();
            let (dir, barrier) = (dir.clone(), barrier.clone());
            std::thread::spawn(move || {
                // each round into a store of its own, which the first version kept creates
                let rounds = requests.into_iter().enumerate();
                let failed: Vec<_> = rounds
                    .filter_map(|(round, req)| {
                        let history = dir.join(format!("history-{round}"));
                        barrier.wait();
                        let written = with_root(history, || req.write());
                        written.err().map(|f| format!("{round}: {}", f.error))
                    })
                    .collect();
                (p, failed)
            })
        })
        .collect();
    for w in writers {
        let (p, failed) = w.join().unwrap();
        assert!(failed.is_empty(), "{p:?}: {failed:?}");
        for round in 0..20 {
            let history = dir.join(format!("history-{round}"));
            assert_eq!(list_in(&history, &p).unwrap().len(), 1, "{p:?} {round}");
        }
    }
}

#[test]
fn save_copy_captures_all_supported_codecs_endings_and_boms_without_mutation() {
    let dir = fresh_dir("n22-codecs");
    assert_eq!(Encoding::ALL.len(), 26);
    for encoding in Encoding::ALL {
        for ending in ["\n", "\r\n", "\r"] {
            for bom in [false, true].into_iter().filter(|bom| !bom || !encoding.bom_bytes().is_empty()) {
                let value = match encoding {
                    Encoding::Windows1250 | Encoding::Iso8859_2 => "Łódź\nlast",
                    Encoding::Windows1251 | Encoding::Iso8859_5 => "Привет\nlast",
                    Encoding::Windows1252 | Encoding::Iso8859_1 => "café\nlast",
                    Encoding::Windows1253 | Encoding::Iso8859_7 => "Ελλάδα\nlast",
                    Encoding::Windows1254 => "İıŞğ\nlast",
                    Encoding::Windows1255 => "שלום\nlast",
                    Encoding::Windows1256 => "مرحبا\nlast",
                    Encoding::Windows1257 => "Žą\nlast",
                    Encoding::Windows1258 => "ă\nlast",
                    Encoding::Iso8859_15 => "€\nlast",
                    Encoding::ShiftJis | Encoding::EucJp | Encoding::Iso2022Jp => "日本\nlast",
                    Encoding::Gbk | Encoding::Big5 => "中文\nlast",
                    Encoding::EucKr => "한국\nlast",
                    _ => "café 🦀 e\u{301}\nlast",
                };
                let mut s = EditorState::from_text(value);
                s.set_encoding(encoding, bom);
                s.set_line_ending(ending);
                let point = s.doc.save_point();
                let path = dir.join("copy.txt");
                let req = s.doc.capture_copy(path.clone()).unwrap();
                s.insert("later");
                s.set_encoding(Encoding::Utf8, false);
                s.set_line_ending("\n");
                req.write().unwrap();
                let expected = orlo_project::encoding::encode(&value.replace('\n', ending), encoding, bom).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), expected, "{encoding:?}, {ending:?}, BOM {bom}");
                assert_eq!(req.save.point, point);
                assert!(s.doc.path.is_none());
                assert!(s.doc.dirty);
            }
        }
    }
}

#[test]
fn save_copy_refuses_source_aliases_and_changed_overwrite_without_touching_source() {
    let (p, mut s) = file("n22-alias-model", "source");
    s.insert("dirty");
    let hard = p.with_file_name("hard");
    let link = p.with_file_name("link");
    std::fs::hard_link(&p, &hard).unwrap();
    std::os::unix::fs::symlink(&p, &link).unwrap();
    for target in [&p, &hard, &link] {
        assert!(s.doc.capture_copy(target.clone()).is_err());
    }
    for change in ["appearance", "replacement", "content", "source alias"] {
        let target = p.with_file_name(format!("target-{change}"));
        if change != "appearance" {
            std::fs::write(&target, "old").unwrap();
        }
        let point = s.doc.save_point();
        let disk = s.doc.disk;
        let req = s.doc.capture_copy(target.clone()).unwrap();
        if change == "replacement" || change == "source alias" {
            std::fs::remove_file(&target).unwrap();
        }
        if change == "source alias" {
            std::fs::hard_link(&p, &target).unwrap();
        } else {
            std::fs::write(&target, "external").unwrap();
        }
        assert!(req.write().is_err(), "{change}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "source");
        assert_eq!((s.doc.save_point(), s.doc.disk), (point, disk));
    }
}

#[test]
fn save_copy_rejects_lossy_output_and_preserves_redo() {
    let dir = fresh_dir("n22-lossy-model");
    let mut s = EditorState::from_text("é\ncafé🦀");
    s.insert("x");
    s.undo();
    s.set_encoding(Encoding::Windows1252, false);
    let path = dir.join("copy.txt");
    std::fs::write(&path, "keep").unwrap();
    let req = s.doc.capture_copy(path.clone()).unwrap();
    let before = (s.doc.save_point(), s.doc.version, s.doc.path.clone(), s.doc.disk, s.unsaved());
    let error = req.write().unwrap_err();
    assert!(error.error.to_string().contains("'🦀' (U+1F980) at line 2, column 5"));
    assert_eq!((s.doc.save_point(), s.doc.version, s.doc.path.clone(), s.doc.disk, s.unsaved()), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "keep");
    s.redo();
    assert_eq!(s.text(), "xé\ncafé🦀");
}

#[test]
fn save_copy_preserves_history_permissions_and_catches_retargeted_symlink() {
    use orlo_project::file_history::{list_in, with_root};
    use std::os::unix::fs::PermissionsExt;
    let dir = fresh_dir("n22-history-copy").canonicalize().unwrap();
    let target = dir.join("target");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut s = EditorState::from_text("new");
    s.doc.history_enabled = true;
    let history = dir.join("history");
    let req = s.doc.capture_copy(target.clone()).unwrap();
    with_root(history.clone(), || req.write().unwrap());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
    assert_eq!(list_in(&history, &target).unwrap().len(), 1);
    let alias = dir.join("alias");
    let other = dir.join("other");
    std::fs::write(&other, "other").unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let req = s.doc.capture_copy(alias.clone()).unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&other, &alias).unwrap();
    assert!(req.write().is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(other).unwrap(), "other");
}

#[test]
fn expanded_f01_guarded_save_reports_conversion_before_history_or_disk_changes() {
    let dir = fresh_dir("f01-guarded-conversion");
    let path = dir.join("source.txt");
    std::fs::write(&path, "é\n").unwrap();
    let mut s = EditorState::from_loaded(&path, Loaded::read(&path).unwrap());
    s.insert("café🦀");
    s.set_encoding(Encoding::Windows1252, false);
    s.doc.history_enabled = true;
    let before = (s.doc.save_point(), s.doc.version, s.doc.path.clone(), s.doc.disk, s.unsaved());
    let req = s.doc.capture_save(path.clone(), None).unwrap();
    let history = dir.canonicalize().unwrap().join("history");
    let error = orlo_project::file_history::with_root(history.clone(), || req.write_if_unchanged(s.doc.disk.unwrap()))
        .unwrap_err();
    assert!(error.error.to_string().contains("'🦀' (U+1F980) at line 1, column 5"));
    assert!(error.disk.is_none());
    assert_eq!((s.doc.save_point(), s.doc.version, s.doc.path.clone(), s.doc.disk, s.unsaved()), before);
    assert_eq!(std::fs::read(&path).unwrap(), "é\n".as_bytes());
    assert!(!history.exists());
}
