use super::*;
use orlo_project::encoding::{encode, Encoding};

fn mixed(raw: &str) -> EditorState {
    let (rope, line_ending, endings) = decoded_file(raw);
    EditorState::from_loaded(
        Path::new("/n25-fixture"),
        Loaded {
            rope,
            line_ending,
            endings: Some(endings),
            bom: false,
            encoding: Encoding::Utf8,
            read_only: false,
            disk: None,
        },
    )
}
fn bytes(s: &EditorState) -> Vec<u8> {
    s.doc.bytes_as_saved(None).unwrap()
}

#[test]
fn n25_all_codecs_bom_and_no_final_newline_roundtrip() {
    let dir = fresh_dir("orlo-n25-codecs");
    let mut cases = 0;
    for encoding in Encoding::ALL {
        for bom in [false, true].into_iter().filter(|bom| !*bom || !encoding.bom_bytes().is_empty()) {
            let raw = if matches!(
                encoding,
                Encoding::ShiftJis
                    | Encoding::EucJp
                    | Encoding::Iso2022Jp
                    | Encoding::Gbk
                    | Encoding::Gb18030
                    | Encoding::Big5
                    | Encoding::EucKr
            ) {
                "中\r\n文\n中\r文"
            } else {
                "a\r\nb\nc\rd"
            };
            {
                let original = encode(raw, encoding, bom).unwrap();
                cases += 1;
                let p = dir.join(format!("{encoding:?}-{bom}"));
                std::fs::write(&p, &original).unwrap();
                let mut s = EditorState::from_loaded(&p, Loaded::read_encoding(&p, Some(encoding)).unwrap());
                assert_eq!(s.doc.ending_counts(), [1, 1, 1]);
                assert_eq!(bytes(&s), original, "{encoding:?}/{bom}");
                s.save().unwrap();
                assert_eq!(std::fs::read(&p).unwrap(), original);
            }
        }
    }
    assert_eq!(cases, 31, "all 26 codecs and five supported Unicode BOM variants");
}

#[test]
fn n25_split_join_delete_multicaret_and_shared_undo_preserve_provenance() {
    let original = b"a\r\nb\nc\rd";
    let mut v = two_views(mixed("a\r\nb\nc\rd"));
    {
        let mut a = v.a();
        a.set_selections(vec![Selection::caret(1), Selection::caret(5)]);
        a.insert("x\ny");
    }
    assert_eq!(v.doc.bytes_as_saved(None).unwrap(), b"ax\ny\r\nb\ncx\ny\rd");
    v.b().undo();
    assert_eq!(v.doc.bytes_as_saved(None).unwrap(), original);
    v.b().redo();
    v.a().undo();
    {
        let mut a = v.a();
        a.replace_ranges(vec![(1..2, String::new())]);
    }
    assert_eq!(v.doc.bytes_as_saved(None).unwrap(), b"ab\nc\rd");
    v.b().undo();
    assert_eq!(v.doc.bytes_as_saved(None).unwrap(), original);
    v.a().replace_ranges(vec![(2..3, "B".into()), (6..7, "D".into())]);
    assert_eq!(v.doc.bytes_as_saved(None).unwrap(), b"a\r\nB\nc\rD");
}

#[test]
fn n25_conversion_is_metadata_history_and_preserves_saved_point_capture() {
    let mut s = mixed("a\r\nb\nc\rd");
    let original = bytes(&s);
    let saved = s.doc.save_point();
    let version = s.doc.version;
    s.convert_line_endings("\r\n");
    assert_eq!(s.text(), "a\nb\nc\nd");
    assert!(s.doc.version > version && s.doc.dirty);
    assert_eq!(bytes(&s), b"a\r\nb\r\nc\r\nd");
    assert!(s.take_change().unwrap().changes.is_empty());
    s.undo();
    assert_eq!(bytes(&s), original);
    assert_eq!(s.doc.save_point(), saved);
    assert!(!s.doc.dirty);
    assert!(!s.doc.line_ending_override);
    s.redo();
    assert_eq!(bytes(&s), b"a\r\nb\r\nc\r\nd");
}

#[test]
fn n25_literal_cr_source_paste_transform_and_explicit_conversion_are_distinct() {
    let mut s = mixed("a\r\nb\nc");
    s.replace_ranges(vec![(1..1, "X\rY\r\nZ".into())]);
    assert_eq!(s.text(), "aX\rY\r\nZ\nb\nc");
    assert_eq!(bytes(&s), b"aX\rY\r\nZ\r\nb\nc");
    assert_eq!(s.doc.ending_counts(), [1, 2, 1]);
    let prior = bytes(&s);
    let source = s.text();
    s.convert_line_endings("\r\n");
    assert_eq!(s.text(), "aX\nY\nZ\nb\nc");
    assert_eq!(bytes(&s), b"aX\r\nY\r\nZ\r\nb\r\nc");
    s.undo();
    assert_eq!(s.text(), source);
    assert_eq!(bytes(&s), prior);
    s.paste("P\r\nQ", false);
    assert!(!s.text().contains("P\r\nQ"));
    s.transform_document(|t| Ok(t.replace('b', "B"))).unwrap();
    assert_eq!(s.doc.endings.as_ref().unwrap().compact().len(), s.doc.rope.len_lines() - 1);
}

#[test]
fn n25_save_and_copy_capture_immutable_map_and_preserve_conversion_races() {
    let dir = fresh_dir("orlo-n25-save-race");
    let mut s = mixed("a\r\nb\nc\rd");
    s.doc.path = None;
    let original = bytes(&s);
    let req = s.doc.capture_save(dir.join("saved"), None).unwrap();
    let copy = s.doc.capture_copy(dir.join("copy")).unwrap();
    s.convert_line_endings("\r");
    let written = req.write().unwrap();
    copy.write().unwrap();
    s.doc.finish_save(&req, &written);
    assert!(s.doc.dirty);
    assert_eq!(std::fs::read(dir.join("saved")).unwrap(), original);
    assert_eq!(std::fs::read(dir.join("copy")).unwrap(), original);
    s.undo();
    assert!(!s.doc.dirty);
    s.set_encoding(Encoding::Utf16Be, true);
    assert_eq!(bytes(&s), encode("a\r\nb\nc\rd", Encoding::Utf16Be, true).unwrap());
}

#[test]
fn n25_preferred_policy_statistics_selection_bytes_and_protection() {
    let mut s = mixed("a\r\nb\nc\rd");
    let original = bytes(&s);
    s.doc.set_line_ending("\r\n");
    assert_eq!(bytes(&s), original);
    assert_eq!(s.doc.bytes_as_saved(Some(1..6)).unwrap(), b"\r\nb\nc\r");
    assert_eq!(
        crate::stats::saved_len_with_endings(
            &s.doc.rope,
            s.doc.endings.as_ref(),
            s.doc.line_ending,
            false,
            Encoding::Utf8
        )
        .unwrap(),
        original.len()
    );
    for following in [false, true] {
        s.doc.set_viewer_mode(!following);
        s.doc.set_following(following);
        let point = s.doc.save_point();
        s.convert_line_endings("\r");
        s.insert("bad\n");
        assert_eq!(s.doc.save_point(), point);
        assert_eq!(s.doc.next_ending("\r", 0), Some(5));
        assert_eq!(bytes(&s), original);
    }
}

#[test]
fn n25_closed_pipeline_search_and_follow_keep_physical_endings() {
    use crate::search::pipeline::{Op, Outcome, Pipeline};
    let dir = fresh_dir("orlo-n25-closed");
    let p = dir.join("file");
    std::fs::write(&p, "a\r\nb\nc\rd").unwrap();
    let pipeline = Pipeline { name: "case".into(), steps: vec![Op::ConvertCase { case: Case::Upper }] };
    assert_eq!(crate::search::pipeline::run_file(&p, &pipeline).unwrap(), Outcome::Changed);
    assert_eq!(std::fs::read(&p).unwrap(), b"A\r\nB\nC\rD");
    let read = crate::search::files::read_file(&p).unwrap();
    assert_eq!(read.text, "A\nB\nC\nD");
    assert_eq!(read.endings.counts(), [1, 1, 1]);
    let orlo_project::watch::follow::Read::Snapshot { loaded, changes, .. } =
        orlo_project::watch::follow::read(&p, None, Encoding::Utf8, &Rope::new()).unwrap()
    else {
        panic!("snapshot")
    };
    let mut s = EditorState::new();
    s.doc.set_following(true);
    s.follow_from(loaded, &changes);
    assert_eq!(bytes(&s), b"A\r\nB\nC\rD");
    s.doc.set_following(false);
    s.undo();
    assert_eq!(bytes(&s), b"A\r\nB\nC\rD");
}

#[test]
fn n25_inconsistent_map_refuses_writes_before_history_or_destination_changes() {
    let dir = fresh_dir("orlo-n25-inconsistent");
    let p = dir.join("source");
    std::fs::write(&p, b"protected original").unwrap();
    let mut s = mixed("a\r\nb\nc");
    s.doc.rope = Rope::from_str("inconsistent\n");
    let req = s.doc.capture_save(p.clone(), None).unwrap();
    assert!(req.write().is_err());
    assert_eq!(std::fs::read(&p).unwrap(), b"protected original");
    assert!(s.doc.bytes_as_saved(None).is_err());
    let mut valid = mixed("a\r\nb\nc");
    let mut req = valid.doc.capture_save(p.clone(), None).unwrap();
    req.point.endings = 0;
    assert!(req.write().is_err());
    assert_eq!(std::fs::read(&p).unwrap(), b"protected original");
}

#[test]
fn n25_ordinary_typing_reuses_boundary_identity_and_snapshot_rope() {
    let mut s = mixed(&"line\r\n".repeat(100_000));
    let map = s.doc.endings.clone().unwrap();
    let id = map.identity();
    s.set_cursor(2);
    for _ in 0..100 {
        s.type_text("x");
    }
    assert_eq!(s.doc.endings.as_ref().unwrap().identity(), id);
    assert_eq!(s.doc.endings.as_ref().unwrap(), &map);
    s.undo();
    assert_eq!(s.doc.endings.as_ref().unwrap().identity(), id);
}

#[test]
fn n25_read_only_original_allows_buffer_conversion_and_protected_copy() {
    let dir = fresh_dir("orlo-n25-readonly");
    let p = dir.join("original");
    std::fs::write(&p, b"a\r\nb\nc\rd").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    s.doc.read_only = true;
    s.convert_line_endings("\r");
    assert_eq!(bytes(&s), b"a\rb\rc\rd");
    assert!(s.doc.capture_save(p.clone(), None).is_err());
    s.doc.capture_copy(dir.join("copy")).unwrap().write().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nb\nc\rd");
    assert_eq!(std::fs::read(dir.join("copy")).unwrap(), b"a\rb\rc\rd");
}

#[test]
fn n25_whole_document_case_sort_and_insert_transform_preserve_separator_bytes() {
    let p = fresh_dir("orlo-n25-broad-transform").join("source");
    std::fs::write(&p, b"c\r\na\nb\r").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    s.transform_document(|text| Ok(text.to_uppercase())).unwrap();
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"C\r\nA\nB\r");
    s.permute_lines(Permute::Sort { case_sensitive: true });
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"A\r\nB\nC\r");
    assert_eq!(s.doc.endings.as_ref().unwrap().compact(), "wlr");
    s.doc.set_line_ending("\r\n");
    s.transform_document(|text| Ok(text.replace('B', "B\nx"))).unwrap();
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"A\r\nB\nx\r\nC\r");
}

#[test]
fn n25_same_ending_explicit_conversion_records_preference_provenance_and_undo() {
    let mut s = mixed("a\nb");
    let point = s.doc.save_point();
    let version = s.doc.version;
    assert!(!s.doc.line_ending_override);
    s.convert_line_endings("\n");
    assert!(s.doc.line_ending_override);
    assert!(!s.doc.dirty);
    assert_eq!(s.doc.save_point(), point);
    assert_eq!(bytes(&s), b"a\nb");
    assert_eq!(s.doc.history.undo.len(), 1);
    assert!(s.doc.history.redo.is_empty());
    assert!(s.doc.version > version);
    assert!(s.take_change().unwrap().changes.is_empty());
    s.undo();
    assert!(!s.doc.line_ending_override);
    assert_eq!(s.doc.save_point(), point);
    assert!(!s.doc.dirty);
    assert!(s.doc.history.undo.is_empty());
    assert_eq!(s.doc.history.redo.len(), 1);
    s.redo();
    assert!(s.doc.line_ending_override);
    assert_eq!(bytes(&s), b"a\nb");
    assert_eq!(s.doc.save_point(), point);
    assert!(!s.doc.dirty);
    let version = s.doc.version;
    s.convert_line_endings("\n");
    assert_eq!(s.doc.save_point(), point);
    assert_eq!(s.doc.version, version, "an already explicit uniform choice is a no-op");
    assert_eq!(s.doc.history.undo.len(), 1);
    assert!(s.doc.history.redo.is_empty());
}

#[test]
fn n25_review_shared_provenance_conversion_separates_peer_typing() {
    let mut v = two_views(mixed("a\n"));
    v.a().set_cursor(1);
    v.a().insert("x");
    let point = v.doc.save_point();
    v.b().convert_line_endings("\n");
    assert_eq!(v.doc.save_point(), point);
    assert!(v.doc.line_ending_override);

    v.a().insert("y");
    assert_eq!(v.doc.text(), "axy\n");
    v.a().undo();
    assert_eq!(v.doc.text(), "ax\n");
    assert!(v.doc.line_ending_override, "undo typing must retain the separate conversion");
    v.b().undo();
    assert_eq!(v.doc.text(), "ax\n");
    assert!(!v.doc.line_ending_override);
    v.a().undo();
    assert_eq!(v.doc.text(), "a\n");
    assert!(!v.doc.dirty);

    v.b().redo();
    v.a().redo();
    assert_eq!(v.doc.text(), "ax\n");
    assert!(v.doc.line_ending_override);
    v.b().redo();
    assert_eq!(v.doc.text(), "axy\n");
    assert!(v.doc.line_ending_override);
    assert_eq!(v.doc.bytes_as_saved(None).unwrap(), b"axy\n");
}

#[test]
fn n25_review_automatic_preference_keeps_explicit_conversion_redo() {
    let mut s = mixed("a\nb");
    s.convert_line_endings("\r");
    assert_eq!(bytes(&s), b"a\rb");
    s.undo();
    assert!(!s.doc.line_ending_override);

    s.doc.set_line_ending("\r\n");
    let path = fresh_dir("orlo-n25-review-preference").join("saved.txt");
    s.save_to(path.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"a\nb");
    assert!(!s.doc.dirty);
    s.redo();
    assert_eq!(s.doc.line_ending, "\r", "redo must restore the explicit conversion preference");
    assert!(s.doc.line_ending_override);
    assert_eq!(bytes(&s), b"a\rb");

    s.set_cursor(s.len());
    s.insert("\nend");
    assert_eq!(bytes(&s), b"a\rb\rend");
    s.undo();
    s.undo();
    assert_eq!(s.doc.line_ending, "\r\n", "the automatic preference still applies before conversion");
    assert!(!s.doc.line_ending_override);
    assert_eq!(bytes(&s), b"a\nb");
    assert!(!s.doc.dirty);
    s.redo();
    assert_eq!(s.doc.line_ending, "\r");
    assert!(s.doc.line_ending_override);
}

#[test]
fn n30_disjoint_sort_blocks_keep_headers_boundary_bytes_and_one_undo() {
    use crate::editing::transform::{sort_by_pattern, SortOpts};
    let original = "H1\r\nv2-part10\nv2-part9\rGAP\r\nH2\nv3-part10\rv3-part9";
    let mut s = mixed(original);
    let before = s.text();
    s.view.selections = vec![Selection::new(1, s.line_start(3)), Selection::new(s.line_start(4) + 1, s.len())];
    let selections = s.view.selections.clone();
    let options: SortOpts =
        serde_json::from_value(serde_json::json!({"pattern":".*", "order":"natural", "keep_header":true})).unwrap();
    s.transform_selections(true, |text| sort_by_pattern(text, &options)).unwrap();
    assert_eq!(bytes(&s), b"H1\r\nv2-part9\nv2-part10\rGAP\r\nH2\nv3-part9\rv3-part10");
    assert_eq!(s.doc.endings.as_ref().unwrap().compact(), "wlrwlr");
    s.undo();
    assert_eq!(bytes(&s), original.as_bytes());
    assert_eq!(s.view.selections, selections);
    assert_eq!(s.text(), before);
    s.redo();
    assert_eq!(bytes(&s), b"H1\r\nv2-part9\nv2-part10\rGAP\r\nH2\nv3-part9\rv3-part10");
    s.undo();
    let invalid: SortOpts = serde_json::from_value(serde_json::json!({"pattern":"(a)", "capture":2})).unwrap();
    assert!(s.transform_selections(true, |text| sort_by_pattern(text, &invalid)).is_err());
    assert_eq!(bytes(&s), original.as_bytes());
    assert_eq!(s.view.selections, selections);
    assert!(s.doc.history.undo.is_empty());
}
