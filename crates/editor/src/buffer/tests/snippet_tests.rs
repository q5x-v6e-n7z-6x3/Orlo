use super::*;
use crate::editing::snippet::parse;

#[test]
fn n32_timestamp_formats_are_atomic_multicaret_and_independent_undo_steps() {
    use orlo_utils::date_time::{DateTime, Snapshot, DEFAULT_FORMAT};
    let time = Snapshot::capture(
        || std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_709_251_645),
        |s| DateTime::with_offset(s, -18_000),
    )
    .unwrap();
    let mut s = at("x|\ny|");
    let before = (s.text(), s.view.selections.clone(), s.doc.version);
    for invalid in ["%", "%Q", "%F", "%::z"] {
        assert!(s.insert_date_time(invalid, false, &time).is_err());
        assert_eq!((s.text(), s.view.selections.clone(), s.doc.version), before);
    }
    assert!(s.insert_date_time(DEFAULT_FORMAT, false, &time).unwrap());
    let local = "x2024-02-29T19:07:25-05:00\ny2024-02-29T19:07:25-05:00";
    assert_eq!(s.text(), local);
    s.insert("!");
    s.undo();
    assert_eq!(s.text(), local);
    s.undo();
    assert_eq!(s.text(), "x\ny");
    s.redo();
    assert_eq!(s.text(), local);
    s.undo();
    s.insert_date_time("%% %Y\n%H:%M", true, &time).unwrap();
    assert_eq!(s.text(), "x% 2024\n00:07\ny% 2024\n00:07");
    for mode in 0..3 {
        let mut s = at("|");
        match mode {
            0 => s.doc.read_only = true,
            1 => {
                s.doc.set_viewer_mode(true);
            }
            _ => {
                s.doc.set_following(true);
            }
        }
        assert!(s.insert_date_time(DEFAULT_FORMAT, false, &time).is_err());
        assert_eq!(s.text(), "");
    }
}

#[test]
fn n32_server_snapshot_variables_keep_builtins_defaults_transforms_and_choices() {
    let mut s = at("pri|");
    s.doc.path = Some("/work/x.rs".into());
    let resolver = |name: &str| match name {
        "CURRENT_YEAR" => Some("2024".into()),
        "CLIPBOARD" => Some("".into()),
        "TM_FILENAME" => Some("must not replace builtin".into()),
        _ => None,
    };
    s.complete_snippet_ranges_with(
        0..3,
        "${CURRENT_YEAR/(.*)/[$1]/}:${CLIPBOARD:empty}:$TM_FILENAME:${1|one,two|}=$1$0",
        Vec::new(),
        &resolver,
    )
    .unwrap();
    assert_eq!(s.text(), "[2024]:empty:x.rs:one=one");
    assert!(s.choose_snippet(1));
    assert_eq!(s.text(), "[2024]:empty:x.rs:two=two");
    s.undo();
    s.undo();
    assert_eq!(s.text(), "pri");
}

#[test]
fn n31_local_choices_decode_escaped_delimiters_and_keep_nested_fields() {
    let mut s = expanded(r"${1:outer ${2|a\,b,c\|d,e\\f,界,|}} $2 ${2/(.*)/<$1>/}$0");
    assert_eq!(s.text(), "outer a,b a,b <a,b>");
    assert_eq!(sel_texts(&s), ["outer a,b"]);
    assert!(s.snippet_next(true));
    assert_eq!(sel_texts(&s), ["a,b", "a,b"]);
    assert_eq!(s.snippet_choices().unwrap().as_ref(), ["a,b", "c|d", r"e\f", "界", ""]);
    assert!(s.choose_snippet(1));
    assert_eq!(s.text(), "outer c|d c|d <c|d>");
    assert_eq!(sel_texts(&s), ["c|d", "c|d"]);
    assert!(s.snippet_choices().is_none());
    assert!(s.snippet_next(false));
    assert_eq!(sel_texts(&s), ["outer c|d"]);
    assert!(s.snippet_next(true));
    assert!(s.choose_snippet(4));
    assert_eq!(s.text(), "outer   <>");
    typed(&mut s, "é");
    assert_eq!(s.text(), "outer é é <é>");
    s.undo();
    assert_eq!(s.text(), "outer   <>");
    s.redo();
    assert_eq!(s.text(), "outer é é <é>");
}

#[test]
fn n31_choice_confirmation_updates_all_insertion_carets_and_one_undo() {
    let mut s = at("|\n|");
    assert!(s.expand_snippet(&snip(r"${1|one,界|}:$1:${1/(.)/[$1]/g}${2:next}$0"), "", &none));
    assert!(s.choose_snippet(1));
    assert_eq!(s.text(), "界:界:[界]next\n界:界:[界]next");
    assert_eq!(sel_texts(&s), ["界", "界", "界", "界"]);
    typed(&mut s, "é");
    assert_eq!(s.text(), "é:é:[é]next\né:é:[é]next");
    s.undo();
    assert_eq!(s.text(), "界:界:[界]next\n界:界:[界]next");
    s.undo();
    assert_eq!(s.text(), "one:one:[o][n][e]next\none:one:[o][n][e]next");
    s.redo();
    assert_eq!(s.text(), "界:界:[界]next\n界:界:[界]next");
}

#[test]
fn n31_choices_dismiss_without_edit_and_reject_stale_protected_or_invalid_confirmation() {
    let mut s = expanded("${1|a,b|} $1 ${2:end}$0");
    let before = (s.text(), s.view.selections.clone(), s.doc.version);
    assert!(!s.choose_snippet(20));
    assert_eq!((s.text(), s.view.selections.clone(), s.doc.version), before);
    s.close_snippet_choices();
    assert!(!s.choose_snippet(1));
    assert!(s.in_snippet());
    assert!(s.snippet_next(true));
    assert!(s.snippet_next(false));
    assert!(s.snippet_choices().is_some());
    s.doc.read_only = true;
    assert!(!s.choose_snippet(1));
    s.doc.read_only = false;
    s.set_selections(vec![Selection::caret(s.len())]);
    assert!(!s.choose_snippet(1));
    assert_eq!(s.text(), "a a end");
    for protected in [true, false] {
        let mut s = expanded("${1|a,b|} $1$0");
        if protected {
            s.doc.set_viewer_mode(true);
        } else {
            s.doc.set_following(true);
        }
        assert!(!s.choose_snippet(1));
        assert_eq!(s.text(), "a a");
    }
}

#[test]
fn n31_choice_session_ends_on_shared_edits_and_parent_replacement() {
    let mut v = views("");
    v.a().expand_snippet(&snip("${1|a,b|} $1$0"), "", &none);
    assert!(v.a().snippet_choices().is_some());
    v.b().insert("!");
    assert!(v.a().snippet_choices().is_none());
    assert!(!v.a().choose_snippet(1));
    let mut s = expanded("${1:outer ${2|a,b|}} $2$0");
    typed(&mut s, "gone");
    assert!(s.snippet_next(true));
    assert!(!s.in_snippet());
    assert_eq!(s.text(), "gone a");
}

#[test]
fn server_snippet_and_additional_edits_share_one_undo_with_unicode_fields() {
    let mut s = at("😀pri|");
    s.complete_snippet_ranges(1..4, "f(${1:中}, $1, ${2|one,two|})$0", vec![(0..0, "use x;\n".into())]).unwrap();
    assert_eq!(s.text(), "use x;\n😀f(中, 中, one)");
    assert_eq!(s.doc.rope.slice(s.view.selections[0].range()).to_string(), "中");
    assert!(s.in_snippet());
    s.snippet_next(true);
    assert_eq!(s.doc.rope.slice(s.view.selections[0].range()).to_string(), "one");
    s.undo();
    assert_eq!(s.text(), "😀pri");
    s.redo();
    assert_eq!(s.text(), "use x;\n😀f(中, 中, one)");
}

#[test]
fn invalid_or_read_only_server_snippets_refuse_all_edits() {
    let mut s = at("pri|");
    assert!(s.complete_snippet_ranges(0..3, "${1:bad", vec![(3..3, "extra".into())]).is_err());
    assert!(s.complete_snippet_ranges(0..3, "$1", vec![(1..2, "overlap".into())]).is_err());
    s.doc.read_only = true;
    assert!(s.complete_snippet_ranges(0..3, "${1:ok}", Vec::new()).is_err());
    assert_eq!(s.text(), "pri");
    s.undo();
    assert_eq!(s.text(), "pri");
}

fn snip(body: &str) -> Snippet {
    Snippet {
        trigger: String::new(),
        scope: String::new(),
        description: String::new(),
        body: parse(body).unwrap(),
        path: PathBuf::from("/x.sublime-snippet"),
    }
}

fn none(_: &str, _: usize) -> Option<String> {
    None
}

/// `text` with a caret at each `|`.
fn at(text: &str) -> EditorState {
    let mut s = EditorState::from_text(&text.replace('|', ""));
    let carets = (text.match_indices('|').enumerate())
        .map(|(i, (b, _))| Selection::caret(text[..b].chars().count() - i))
        .collect();
    s.set_selections(carets);
    s
}

/// A caret's expansion of `body`, without a trigger.
fn expanded(body: &str) -> EditorState {
    let mut s = at("|");
    s.expand_snippet(&snip(body), "", &none);
    s
}

/// A command as the editor runs it: the session catches up after it.
fn act(s: &mut EditorState, f: impl FnOnce(&mut EditorState)) {
    f(s);
    s.sync_snippet();
}

/// Keyboard typing, a char a command.
fn typed(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        act(s, |s| s.type_text(&ch.to_string()));
    }
}

#[test]
fn a_trigger_ends_the_line_before_the_caret_and_is_not_glued_to_a_word() {
    let s = EditorState::from_text("x fn\nfoo$$\nfn\n$$ FN");
    let end = |l: usize| s.line_end(l);
    assert!(s.trigger_before(end(0), "fn"));
    assert!(!s.trigger_before(end(0), "n"), "glued to the word before");
    assert!(!s.trigger_before(end(0), "x fn\n"));
    // a trigger that starts with punctuation may follow a word
    assert!(s.trigger_before(end(1), "$$"));
    assert!(!s.trigger_before(end(1), "o$$"));
    assert!(s.trigger_before(end(1), "foo$$"), "at the line's start");
    assert!(s.trigger_before(end(2), "fn"));
    // never across a line break, exact case, never empty
    assert!(!s.trigger_before(s.line_start(3), "fn"));
    assert!(!s.trigger_before(end(3), "fn"));
    assert!(!s.trigger_before(end(2), ""));
    assert!(!s.trigger_before(end(2), "longer than the line"));
}

#[test]
fn an_expansion_replaces_the_trigger_and_selects_the_first_field_as_one_step() {
    let mut s = at("x fn|");
    assert!(s.expand_snippet(&snip("fn ${1:name}($2) {\n\t$0\n}"), "fn", &none));
    assert_eq!(s.text(), "x fn name() {\n    \n}");
    assert_eq!(sel_texts(&s), ["name"]);
    assert!(s.in_snippet());
    s.undo();
    assert_eq!((s.text(), carets_of(&s)), ("x fn".into(), vec![4]));
    assert!(!s.in_snippet());
    // a caret without the trigger before it is left alone
    let mut s = at("fn| xfn|");
    assert!(s.expand_snippet(&snip("<$1>"), "fn", &none));
    assert_eq!(s.text(), "<> xfn");
    assert!(!at("xfn|").expand_snippet(&snip("<$1>"), "fn", &none));
    // no fields: the caret ends at $0, or at the end, and no session starts
    let mut s = at("|;");
    s.expand_snippet(&snip("a$0b"), "", &none);
    assert_eq!((s.text(), carets_of(&s), s.in_snippet()), ("ab;".into(), vec![1], false));
    let mut s = at("|;");
    s.expand_snippet(&snip("ab"), "", &none);
    assert_eq!((carets_of(&s), s.in_snippet()), (vec![2], false));
}

#[test]
fn tab_visits_nested_fields_unless_typing_over_the_parent_dropped_them() {
    let body = snip("<${1:a ${2:b}}>$0");
    let mut s = at("|");
    s.expand_snippet(&body, "", &none);
    assert_eq!((s.text(), sel_texts(&s)), ("<a b>".into(), vec!["a b".into()]));
    assert!(s.snippet_next(true));
    assert_eq!(sel_texts(&s), ["b"]);
    assert!(s.snippet_next(false));
    assert_eq!(sel_texts(&s), ["a b"]);
    assert!(s.snippet_next(false), "at the first field it stays there");
    assert_eq!(sel_texts(&s), ["a b"]);
    s.snippet_next(true);
    assert!(s.snippet_next(true));
    assert_eq!((carets_of(&s), s.in_snippet()), (vec![5], false), "on to $0");
    assert!(!s.snippet_next(true), "no session");
    // typing over the parent drops the field in it: Tab goes on to $0
    let mut s = at("|");
    s.expand_snippet(&body, "", &none);
    typed(&mut s, "x");
    assert_eq!(s.text(), "<x>");
    assert!(s.snippet_next(true));
    assert_eq!((carets_of(&s), s.in_snippet()), (vec![3], false));
    // an empty field at the end of its parent goes with it; one after the parent stays
    let mut s = expanded("${1:a$2}${3}|");
    typed(&mut s, "x");
    s.snippet_next(true);
    assert_eq!((carets_of(&s), s.in_snippet()), (vec![1], true), "field 3, after the x");
    s.snippet_next(true);
    assert_eq!((carets_of(&s), s.in_snippet()), (vec![2], false), "the end");
}

#[test]
fn mirrors_and_transformed_mirrors_follow_the_typing() {
    let mut s = expanded("${1:foo} $1 ${1/(.)/\\u$1/}");
    assert_eq!(sel_texts(&s), ["foo", "foo"]);
    typed(&mut s, "bar");
    assert_eq!(s.text(), "bar bar Bar");
    act(&mut s, |s| s.backspace());
    assert_eq!(s.text(), "ba ba Ba");
    assert_eq!(carets_of(&s), [2, 5]);
    // a mirror in a later field follows the earlier one, until that field is typed over
    let mut s = expanded("${1:key}: ${2:$1};");
    typed(&mut s, "id");
    assert_eq!(s.text(), "id: id;");
    s.snippet_next(true);
    assert_eq!(sel_texts(&s), ["id"]);
    typed(&mut s, "7");
    s.snippet_next(false);
    typed(&mut s, "n");
    assert_eq!(s.text(), "n: 7;", "the mirror went with what was typed over it");
    // an inactive field's mirrors follow it when an edit of the active one changes it
    let mut s = expanded("${1:a ${2:bc}} $2");
    act(&mut s, |s| s.set_cursor(3));
    typed(&mut s, "x");
    assert_eq!(s.text(), "a bxc bxc");
    assert!(s.in_snippet());
    s.snippet_next(true);
    assert_eq!(sel_texts(&s), ["bxc", "bxc"]);
    // a mirror before its primary, and empty fields side by side
    let mut s = expanded("$1$1=${1:v}");
    assert_eq!(s.text(), "vv=v");
    typed(&mut s, "ab");
    assert_eq!(s.text(), "abab=ab");
}

#[test]
fn a_conditional_on_a_group_that_matched_nothing_takes_its_then_part() {
    // probed in Sublime 4215: `xx` as inserted, `bT`, `abT` and `cc` as `b`, `ab` and `c` are typed
    for (typed_text, want) in [("", "xx"), ("b", "bT"), ("ab", "abT"), ("c", "cc")] {
        let mut s = expanded("${1:x}${1/(a*)b/(?1:T:E)/}");
        typed(&mut s, typed_text);
        assert_eq!(s.text(), want, "typed {typed_text:?}");
    }
}

#[test]
fn a_caret_stays_in_its_field_when_a_mirror_beside_it_changes() {
    // a mirror right after the field (Go's `meth`): the caret stays before it
    let mut s = expanded("f() $1${1/.+/ /}{");
    typed(&mut s, "int");
    assert_eq!((s.text(), carets_of(&s)), ("f() int {".into(), vec![7]));
    assert!(s.in_snippet());
    // LaTeX's Tabular (less its line with a lookbehind): `t` + `abular`, `a` + `rray`
    let mirror = "${1/(t)$|(a)$|(.*)/(?1:abular)(?2:rray)/}";
    let body = format!("\\begin{{${{1:t}}{mirror}}}{{${{2:c}}}}\n$0\n\\end{{${{1:t}}{mirror}}}");
    let mut s = at("|");
    s.expand_snippet(&snip(&body), "", &none);
    assert_eq!(s.text(), "\\begin{tabular}{c}\n\n\\end{tabular}");
    typed(&mut s, "a");
    assert_eq!(s.text(), "\\begin{array}{c}\n\n\\end{array}");
    assert_eq!(carets_of(&s), [8, 24], "after the a, before rray");
    assert!(s.snippet_next(true));
    assert_eq!(sel_texts(&s), ["c"]);
    // a mirror right before the field: the caret stays after it, at the field's end or start
    let mut s = expanded("${1/.+/x/}$1;");
    typed(&mut s, "ab");
    assert_eq!((s.text(), carets_of(&s), s.in_snippet()), ("xab;".into(), vec![3], true));
    let mut s = expanded("${1/^a.*//}${1:ab};");
    assert_eq!(s.text(), "ab;");
    act(&mut s, |s| s.move_cursors(Motion::Left, false));
    act(&mut s, |s| s.delete());
    assert_eq!((s.text(), carets_of(&s), s.in_snippet()), ("bb;".into(), vec![1], true));
    typed(&mut s, "c");
    assert_eq!((s.text(), carets_of(&s)), ("cbcb;".into(), vec![3]));
}

#[test]
fn typing_in_a_mirror_alone_ends_the_session_and_the_text_stays_as_typed() {
    // a click into the mirror, then a key
    let mut s = expanded("${1:foo} $1;");
    act(&mut s, |s| s.set_cursor(6));
    assert!(s.in_snippet());
    typed(&mut s, "x");
    assert_eq!((s.text(), carets_of(&s), s.in_snippet()), ("foo foxo;".into(), vec![7], false));
    // a mirror right before its primary takes what is typed at the primary's start
    let mut s = expanded("$1${1:ab};");
    act(&mut s, |s| s.move_cursors(Motion::Left, false));
    assert_eq!(carets_of(&s), [0, 2]);
    typed(&mut s, "z");
    assert_eq!((s.text(), carets_of(&s), s.in_snippet()), ("zabzab;".into(), vec![1, 4], false));
    // typing in the primary alone still brings the mirror along
    let mut s = expanded("${1:foo} $1;");
    act(&mut s, |s| s.set_cursor(1));
    typed(&mut s, "x");
    assert_eq!((s.text(), carets_of(&s), s.in_snippet()), ("fxoo fxoo;".into(), vec![2], true));
}

#[test]
fn every_caret_expands_with_its_own_selection_line_and_word() {
    let mut s = EditorState::from_text("one two");
    s.set_selections(sels(&[(0, 3), (4, 7)]));
    assert!(s.expand_snippet(&snip("[${1:$SELECTION}]$0"), "", &none));
    assert_eq!(s.text(), "[one] [two]");
    assert_eq!(sel_texts(&s), ["one", "two"]);
    assert_eq!(s.view.selections[s.primary()].range(), 1..4, "the first caret's field");
    typed(&mut s, "x");
    assert_eq!(s.text(), "[x] [x]");
    s.snippet_next(true);
    assert_eq!(carets_of(&s), [3, 7]);
    // with an empty selection SELECTION is empty
    let mut s = at("ab|\ncd|");
    s.expand_snippet(&snip("<$SELECTION|$TM_LINE_NUMBER:$TM_LINE_INDEX:$TM_CURRENT_WORD>"), "", &none);
    assert_eq!(s.text(), "ab<|1:2:ab>\ncd<|2:2:cd>");
    // the rest of the list, and the caller's for the others
    let mut s = at("    a|");
    s.doc.path = Some(PathBuf::from("/src/lib.rs"));
    (s.doc.tab_size, s.doc.translate_tabs) = (2, false);
    let vars = |name: &str, i: usize| (name == "MINE").then(|| format!("caret {i}"));
    let body = "$TM_FILENAME $TM_FILEPATH $TM_DIRECTORY $TM_SOFT_TABS $TM_TAB_SIZE \
                    [$TM_CURRENT_LINE] $MINE ${NOPE:none}";
    s.expand_snippet(&snip(body), "", &vars);
    assert_eq!(s.text(), "    alib.rs /src/lib.rs /src NO 2 [    a] caret 0 none");
}

#[test]
fn escape_a_selection_outside_or_an_edit_outside_ends_the_session() {
    let body = snip("f(${1:a}, ${2:b})$0");
    let start = || {
        let mut s = at("x\n|");
        s.expand_snippet(&body, "", &none);
        s
    };
    // Escape: the selections stay
    let mut s = start();
    s.end_snippet();
    assert_eq!((s.in_snippet(), sel_texts(&s)), (false, vec!["a".into()]));
    // moving inside the field keeps it, leaving it ends it
    let mut s = start();
    act(&mut s, |s| s.move_cursors(Motion::Right, false));
    assert!(s.in_snippet());
    act(&mut s, |s| s.move_cursors(Motion::Right, false));
    assert!(!s.in_snippet());
    // a caret outside, while another stays in the field, then an edit there
    let mut s = start();
    act(&mut s, |s| s.add_cursor(0));
    assert!(s.in_snippet());
    act(&mut s, |s| s.insert("!"));
    assert!(!s.in_snippet());
    assert_eq!(s.text(), "!x\nf(!, b)");
    // a reload, and another expansion, which takes over
    let mut s = start();
    reload(&mut s, loaded("y\n"));
    assert!(!s.in_snippet());
    let mut s = start();
    s.expand_snippet(&snip("<${1:in}>"), "", &none);
    assert_eq!(sel_texts(&s), ["in"]);
    s.snippet_next(true);
    assert_eq!((s.in_snippet(), s.text()), (false, "x\nf(<in>, b)".into()));
}

#[test]
fn undo_and_redo_end_the_session_and_take_the_expansion_and_typing_back_a_step_each() {
    let mut s = at("fn|");
    assert!(s.expand_snippet(&snip("${1:name}($2) $1"), "fn", &none));
    assert_eq!(s.text(), "name() name");
    typed(&mut s, "go");
    assert_eq!(s.text(), "go() go");
    s.undo();
    assert!(!s.in_snippet());
    assert_eq!((s.text(), sel_texts(&s)), ("name() name".into(), vec!["name".into(), "name".into()]));
    s.undo();
    assert_eq!((s.text(), carets_of(&s)), ("fn".into(), vec![2]));
    s.redo();
    assert_eq!(s.text(), "name() name");
    assert_eq!(sel_texts(&s), ["name", "name"]);
    assert!(!s.in_snippet());
    s.redo();
    assert_eq!(s.text(), "go() go");
    // a transformed mirror's sync is part of the typing's step
    let mut s = expanded("${1:a} ${1/(.*)/\\U$1/}");
    typed(&mut s, "xy");
    assert_eq!(s.text(), "xy XY");
    s.undo();
    assert_eq!(s.text(), "a A");
    s.redo();
    assert_eq!(s.text(), "xy XY");
}

#[test]
fn the_expansion_takes_the_lines_indent_and_the_views_indentation() {
    let mut s = at("    x|");
    (s.doc.tab_size, s.doc.translate_tabs) = (2, true);
    s.expand_snippet(&snip("{\n\t$0\n}"), "", &none);
    assert_eq!((s.text(), carets_of(&s)), ("    x{\n      \n    }".into(), vec![13]));
    let mut s = at("\tx|");
    s.doc.translate_tabs = false;
    s.expand_snippet(&snip("{\n\t$0\n}"), "", &none);
    assert_eq!(s.text(), "\tx{\n\t\t\n\t}");
}

#[test]
fn another_view_follows_the_expansion_and_its_mirrors_and_its_own_edit_ends_the_session() {
    let mut v = views("");
    v.a().expand_snippet(&snip("${1:foo} ${1/(.)/\\u$1/}"), "", &none);
    assert_eq!(v.b().text(), "foo Foo");
    for ch in ["x", "y"] {
        v.a().type_text(ch);
        v.b();
    }
    assert_eq!((v.a().text(), v.b().text()), ("xy Xy".into(), "xy Xy".into()));
    assert!(v.a().in_snippet() && !v.b().in_snippet());
    // an edit from the other view lies in none of this view's fields: the session ends
    let mut b = v.b();
    let end = b.len();
    b.set_cursor(end);
    b.insert("!");
    assert_eq!(v.a().text(), "xy Xy!");
    assert!(!v.a().in_snippet());
}

#[test]
fn subword_delete_preserves_local_snippet_mirrors_and_overtype_mode() {
    let mut s = expanded("${1:camelCase} = $1; $0");
    s.view.overtype = true;
    act(&mut s, |s| s.set_cursor(5));
    act(&mut s, |s| s.delete_subword(true));
    assert_eq!(s.text(), "camel = camel; ");
    assert!(s.in_snippet());
    assert!(s.view.overtype);
    act(&mut s, |s| s.undo());
    assert_eq!(s.text(), "camelCase = camelCase; ");
    act(&mut s, |s| s.redo());
    assert_eq!(s.text(), "camel = camel; ");
}

#[test]
fn virtual_space_fix1_explicit_snippet_final_caret_continues_and_undoes_padding() {
    let mut s = EditorState::from_text("a");
    s.set_virtual_space(VirtualSpace::Rectangular);
    let original = vec![Selection::from_points((1, 2), (1, 2)).in_rectangle()];
    s.set_selections(original.clone());
    assert!(s.expand_snippet(&snip("X$0"), "", &none));
    assert_eq!(s.text(), "a  X");
    assert!(s.view.selections[0].rectangular);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.view.selections[0].head_point(), (4, 1));
    let after = s.view.selections.clone();
    s.undo();
    assert_eq!(s.text(), "a");
    assert_eq!(s.view.selections, original);
    s.redo();
    assert_eq!(s.text(), "a  X");
    assert_eq!(s.view.selections, after);
}

#[test]
fn virtual_space_fix1_snippet_slots_retain_mixed_origins_when_field_counts_and_order_differ() {
    let mut s = EditorState::from_text("a\nb\nc");
    s.set_virtual_space(VirtualSpace::Rectangular);
    s.set_selections(vec![
        Selection::from_points((1, 2), (1, 2)).in_rectangle(),
        Selection::caret(3),
        Selection::from_points((5, 1), (5, 1)).in_rectangle(),
    ]);
    let origins = |s: &EditorState| s.view.selections.iter().map(|sel| sel.rectangular).collect::<Vec<_>>();
    let body = snip(r"${SKIP:${1:${2:x}}-$1-${1/(.)/\U$1/}}$0");
    assert!(s.expand_snippet(&body, "", &|name, i| (name == "SKIP" && i == 0).then(|| "Z".into())));
    let expanded = "a  Z\nbx-x-X\nc x-x-X";
    assert_eq!(s.text(), expanded);
    assert_eq!(sel_texts(&s), ["x", "x", "x", "x"]);
    assert_eq!(origins(&s), [false, false, true, true]);
    assert_eq!(s.primary(), 0, "the first present primary field remains primary");
    assert!(s.snippet_next(true));
    assert_eq!(sel_texts(&s), ["x", "x"]);
    assert_eq!(origins(&s), [false, true]);
    typed(&mut s, "é");
    let edited = "a  Z\nbé-é-É\nc é-é-É";
    assert_eq!(s.text(), edited);
    assert!(s.snippet_next(false));
    assert_eq!(sel_texts(&s), ["é", "é", "é", "é"]);
    assert_eq!(origins(&s), [false, false, true, true]);
    assert!(s.snippet_next(true));
    assert_eq!(origins(&s), [false, true]);
    assert!(s.snippet_next(true));
    assert!(!s.in_snippet());
    assert_eq!(origins(&s), [true, false, true]);
    assert_eq!(s.primary(), 0);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.view.selections.iter().map(|sel| sel.head_virtual).collect::<Vec<_>>(), [1, 0, 1]);
    let after = s.view.selections.clone();
    s.undo();
    assert_eq!(s.text(), expanded, "field typing and all mirrors share one undo");
    assert_eq!(origins(&s), [false, true]);
    s.redo();
    assert_eq!(s.text(), edited);
    assert_eq!(s.view.selections, after);
}

#[test]
fn virtual_space_fix1_filtered_snippet_triggers_keep_only_the_matching_origins() {
    let mut s = at("fn|\nno|\nfn|");
    s.set_virtual_space(VirtualSpace::Rectangular);
    s.view.selections[2].rectangular = true;
    let original = s.view.selections.clone();
    assert!(s.expand_snippet(&snip("${1:X}-$1$0"), "fn", &none));
    assert_eq!(s.text(), "X-X\nno\nX-X");
    assert_eq!(s.view.selections.iter().map(|sel| sel.rectangular).collect::<Vec<_>>(), [false, false, true, true]);
    assert!(s.snippet_next(true));
    assert_eq!(s.view.selections.iter().map(|sel| sel.rectangular).collect::<Vec<_>>(), [false, true]);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.view.selections.iter().map(Selection::head_point).collect::<Vec<_>>(), [(4, 0), (10, 1)]);
    let after = s.view.selections.clone();
    s.undo();
    assert_eq!(s.text(), "fn\nno\nfn");
    assert_eq!(s.view.selections, original);
    s.redo();
    assert_eq!(s.text(), "X-X\nno\nX-X");
    assert_eq!(s.view.selections, after);
}

#[test]
fn virtual_space_fix1_server_snippet_uses_primary_origin_through_fields_and_additional_edits() {
    for rectangular in [false, true] {
        let mut s = EditorState::from_text("n\npri");
        s.set_virtual_space(VirtualSpace::Rectangular);
        let original = vec![
            Selection { rectangular: !rectangular, ..Selection::caret(1) },
            Selection { rectangular, ..Selection::caret(5) },
        ];
        s.set_selections(original.clone());
        assert!(s.complete_snippet_ranges(2..5, "${1:x}-$1-${2:界}$0", vec![(0..0, "use;\n".into())]).unwrap());
        assert_eq!(s.text(), "use;\nn\nx-x-界");
        assert_eq!(s.view.selections.len(), 2);
        assert!(s.view.selections.iter().all(|sel| sel.rectangular == rectangular));
        assert_eq!(s.primary(), 0);
        assert!(s.snippet_next(true));
        assert_eq!(sel_texts(&s), ["界"]);
        assert_eq!(s.view.selections[0].rectangular, rectangular);
        assert!(s.snippet_next(false));
        assert_eq!(sel_texts(&s), ["x", "x"]);
        assert!(s.view.selections.iter().all(|sel| sel.rectangular == rectangular));
        assert!(s.snippet_next(true));
        assert!(s.snippet_next(true));
        s.move_cursors(Motion::Right, false);
        assert_eq!(s.view.selections[0].head_point(), (12, usize::from(rectangular)));
        let after = s.view.selections.clone();
        s.undo();
        assert_eq!(s.text(), "n\npri");
        assert_eq!(s.view.selections, original);
        s.redo();
        assert_eq!(s.text(), "use;\nn\nx-x-界");
        assert_eq!(s.view.selections, after);
    }
}

#[test]
fn virtual_space_fix1_plain_completion_retains_primary_and_secondary_origins() {
    for rectangular in [false, true] {
        let mut s = EditorState::from_text("n\npri");
        s.set_virtual_space(VirtualSpace::Rectangular);
        let original = vec![
            Selection { rectangular: !rectangular, ..Selection::caret(1) },
            Selection { rectangular, ..Selection::caret(5) },
        ];
        s.set_selections(original.clone());
        assert!(s.complete_ranges(vec![(2..5, "界X".into()), (0..0, "use;\n".into())]));
        assert_eq!(s.text(), "use;\nn\n界X");
        assert_eq!(
            s.view.selections.iter().map(|sel| sel.rectangular).collect::<Vec<_>>(),
            [!rectangular, rectangular]
        );
        assert_eq!(s.primary(), 1);
        s.move_cursors(Motion::Right, false);
        assert_eq!(
            s.view.selections.iter().map(|sel| sel.head_virtual).collect::<Vec<_>>(),
            [usize::from(!rectangular), usize::from(rectangular)]
        );
        let after = s.view.selections.clone();
        s.undo();
        assert_eq!(s.text(), "n\npri");
        assert_eq!(s.view.selections, original);
        s.redo();
        assert_eq!(s.text(), "use;\nn\n界X");
        assert_eq!(s.view.selections, after);
    }
}

#[test]
fn virtual_space_fix1_ordinary_source_range_selection_does_not_inherit_snippet_origin() {
    let mut s = EditorState::from_text("a");
    s.set_virtual_space(VirtualSpace::Rectangular);
    s.set_selections(vec![Selection::caret(1).in_rectangle()]);
    s.select_ranges(std::slice::from_ref(&(0..1)));
    assert_eq!(s.view.selections, [Selection::new(0, 1)]);
    assert!(!s.doc.unsaved());
}

#[test]
fn virtual_space_fix1_plain_completion_does_not_promote_a_mapped_mixed_primary_group() {
    for prepend in [false, true] {
        let mut s = EditorState::from_text("n\npri");
        s.set_virtual_space(VirtualSpace::Rectangular);
        let original =
            vec![Selection::caret(0).in_rectangle(), Selection::caret(3), Selection::caret(5).in_rectangle()];
        s.set_selections(original.clone());
        let mut edits = vec![(2..5, String::new())];
        if prepend {
            edits.push((0..0, "use;\n".into()));
        }
        assert!(s.complete_ranges(edits));
        let expected = if prepend { "use;\nn\n" } else { "n\n" };
        assert_eq!(s.text(), expected);
        assert_eq!(s.view.selections.len(), 2);
        assert_eq!(s.primary(), 1);
        assert_eq!(s.view.selections.iter().map(|sel| sel.rectangular).collect::<Vec<_>>(), [true, false]);
        s.move_cursors(Motion::Right, false);
        assert_eq!(s.view.selections[1].head_point(), (expected.chars().count(), 0));
        let after = s.view.selections.clone();
        s.undo();
        assert_eq!(s.text(), "n\npri");
        assert_eq!(s.view.selections, original);
        s.redo();
        assert_eq!(s.text(), expected);
        assert_eq!(s.view.selections, after);
    }
}

#[test]
fn virtual_space_fix1_server_snippet_keeps_primary_only_origin_after_mapped_peer_merge() {
    let mut s = EditorState::from_text("n\npri");
    s.set_virtual_space(VirtualSpace::Rectangular);
    let original = vec![Selection::caret(0).in_rectangle(), Selection::caret(3), Selection::caret(5).in_rectangle()];
    s.set_selections(original.clone());
    assert!(s.complete_snippet_ranges(2..5, "", vec![(0..0, "use;\n".into())]).unwrap());
    assert_eq!(s.text(), "use;\nn\n");
    assert_eq!(s.view.selections, [Selection::caret(7).in_rectangle()]);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.view.selections[0].head_point(), (7, 1));
    let after = s.view.selections.clone();
    s.undo();
    assert_eq!(s.text(), "n\npri");
    assert_eq!(s.view.selections, original);
    s.redo();
    assert_eq!(s.text(), "use;\nn\n");
    assert_eq!(s.view.selections, after);
}

#[test]
fn prose_sentence_delete_reuses_snippet_mirrors_and_preserves_overtype() {
    let mut s = expanded("${1:One. Two!} = $1; $0");
    s.view.overtype = true;
    act(&mut s, |s| s.set_cursor(1));
    act(&mut s, |s| s.delete_sentence());
    assert_eq!(s.text(), "Two! = Two!; ");
    assert!(s.in_snippet());
    assert!(s.view.overtype);
    act(&mut s, |s| s.undo());
    assert_eq!(s.text(), "One. Two! = One. Two!; ");
    act(&mut s, |s| s.redo());
    assert_eq!(s.text(), "Two! = Two!; ");
}

#[test]
fn n06_transpose_words_reuses_snippet_mirrors_and_keeps_overtype() {
    let mut s = expanded("${1:alpha beta} = $1; $0");
    s.view.overtype = true;
    act(&mut s, |s| s.set_cursor(1));
    act(&mut s, |s| {
        assert!(s.transpose_words());
    });
    assert_eq!(s.text(), "beta alpha = beta alpha; ");
    assert!(s.in_snippet());
    assert!(s.view.overtype);
    act(&mut s, |s| s.undo());
    assert_eq!(s.text(), "alpha beta = alpha beta; ");
    act(&mut s, |s| s.redo());
    assert_eq!(s.text(), "beta alpha = beta alpha; ");
}

#[test]
fn n07_surrounds_reuse_snippet_mirrors_and_keep_overtype() {
    let mut s = expanded("${1:(value)} = $1; $0");
    s.view.overtype = true;
    act(&mut s, |s| s.set_cursor(3));
    act(&mut s, |s| {
        assert!(s.replace_ranges(s.surround_edits(
            Surround::Round,
            Some(Surround::Square),
            std::num::NonZeroUsize::MIN
        )));
    });
    assert_eq!(s.text(), "[value] = [value]; ");
    assert!(s.in_snippet());
    assert!(s.view.overtype);
    act(&mut s, |s| s.undo());
    assert_eq!(s.text(), "(value) = (value); ");
    act(&mut s, |s| s.redo());
    assert_eq!(s.text(), "[value] = [value]; ");
}

#[test]
fn n09_adjusting_a_snippet_field_updates_its_mirrors_in_the_same_undo() {
    let mut s = expanded("${1:009} $1 ${1/(.*)/<$1>/}$0");
    act(&mut s, |s| {
        let edits = s.adjust_value_edits(crate::columns::ValueStep(1)).unwrap();
        s.replace_ranges(edits);
    });
    assert_eq!(s.text(), "010 010 <010>");
    assert!(s.in_snippet());
    s.undo();
    assert_eq!(s.text(), "009 009 <009>");
    s.redo();
    assert_eq!(s.text(), "010 010 <010>");
}

#[test]
fn n10_aligning_a_snippet_field_reuses_its_mirrors_and_batch_undo() {
    let mut s = expanded("${1:a}=x\nlong=y\n$1$0");
    s.view.overtype = true;
    act(&mut s, |s| s.set_cursor(0));
    act(&mut s, |s| {
        s.replace_ranges(s.align_pattern_edits("=", 0, false).unwrap());
    });
    assert_eq!(s.text(), "a   =x\nlong=y\na   ");
    assert!(s.in_snippet());
    assert!(s.view.overtype);
    s.undo();
    assert_eq!(s.text(), "a=x\nlong=y\na");
    s.redo();
    assert_eq!(s.text(), "a   =x\nlong=y\na   ");
}

#[test]
fn n10_review_snippet_mirrors_refuse_unstable_alignment_without_losing_redo() {
    let source = "a=x\nprefix a=zz";
    for published in [false, true] {
        let mut s = EditorState::from_text(source);
        s.set_cursor(s.len());
        s.insert("!");
        s.undo();
        // Expanding over identical source enters a real snippet without a new source edit.
        s.set_selections(vec![Selection { anchor: 0, head: s.len(), ..Selection::caret(0) }]);
        assert!(s.expand_snippet(&snip("${1:a}=x\nprefix $1=zz$0"), "", &none));
        act(&mut s, |s| s.set_cursor(0));
        if published {
            s.take_change();
        }
        assert_eq!(s.text(), source);
        assert!(s.in_snippet());
        let before = (
            s.text(),
            s.selection_state(),
            s.selection_generation(),
            s.doc.version,
            s.doc.save_point(),
            s.doc.unsaved(),
        );
        assert!(s.align_pattern_edits("=", 0, false).unwrap_err().contains("field columns"));
        assert!(
            (
                s.text(),
                s.selection_state(),
                s.selection_generation(),
                s.doc.version,
                s.doc.save_point(),
                s.doc.unsaved()
            ) == before
        );
        assert!(s.in_snippet());
        s.redo();
        assert_eq!(s.text(), "a=x\nprefix a=zz!");
    }
}

#[test]
fn n10_review_refused_alignment_keeps_live_snippet_mirrors_working() {
    let mut s = expanded("${1:a}=x\nprefix $1=zz$0");
    act(&mut s, |s| s.set_cursor(0));
    assert!(s.align_pattern_edits("=", 0, false).is_err());
    assert!(s.in_snippet());
    typed(&mut s, "Q");
    assert_eq!(s.text(), "Qa=x\nprefix Qa=zz");
    s.undo();
    assert_eq!(s.text(), "a=x\nprefix a=zz");
    s.redo();
    assert_eq!(s.text(), "Qa=x\nprefix Qa=zz");
}
