use super::*;

#[test]
fn each_unit_converts_through_the_text_of_its_version() {
    // é is 2 bytes and 1 UTF-16 unit, 🦀 4 bytes and 2 units
    let s = EditorState::from_text("aé🦀\r\nx");
    let c = s.doc.coords();
    assert_eq!(c.version(), s.doc.version);
    let crab = CharOffset(2);
    assert_eq!(c.char_to_byte(crab), ByteOffset(3));
    assert_eq!(c.char_to_utf16(crab), Utf16Offset(2));
    assert_eq!(c.char_to_utf16(CharOffset(3)), Utf16Offset(4));
    assert_eq!(c.byte_to_char(ByteOffset(3)), crab);
    assert_eq!(c.utf16_to_char(Utf16Offset(2)), crab);
    // inside a char: the char it is in
    assert_eq!(c.byte_to_char(ByteOffset(4)), crab);
    assert_eq!(c.utf16_to_char(Utf16Offset(3)), crab);
    // the same answers as the buffer's own conversions
    for o in 0..=s.len() {
        let p = c.char_to_point(CharOffset(o));
        assert_eq!((p.line, p.column), s.point(o));
        assert_eq!(c.point_to_char(p), CharOffset(s.offset(p.line, p.column)));
        assert_eq!(c.char_to_utf16(CharOffset(o)).0, s.char_to_utf16(o));
    }
}

#[test]
fn positions_out_of_range_clamp_as_carets_do() {
    let s = EditorState::from_text("ab\r\ncd");
    let c = s.doc.coords();
    assert_eq!(c.char_to_byte(CharOffset(99)), ByteOffset(6));
    assert_eq!(c.byte_to_char(ByteOffset(99)), CharOffset(6));
    assert_eq!(c.utf16_to_char(Utf16Offset(99)), CharOffset(6));
    // a column past the line's end stops before its CRLF; a line past the last is the end
    assert_eq!(c.point_to_char(Point::new(0, 9)), CharOffset(2));
    assert_eq!(c.point_to_char(Point::new(1, 9)), CharOffset(6));
    assert_eq!(c.point_to_char(Point::new(5, 0)), CharOffset(6));
}

#[test]
fn extreme_columns_clamp_before_offset_arithmetic_can_overflow() {
    let s = EditorState::from_text("é\r\n猫x");
    for line in 0..s.line_count() {
        assert_eq!(s.offset(line, usize::MAX), s.line_end(line));
        assert_eq!(s.doc.coords().point_to_char(Point::new(line, usize::MAX)), CharOffset(s.offset(line, usize::MAX)));
    }
}

#[test]
fn coords_name_the_version_they_were_taken_from() {
    let mut s = EditorState::from_text("ab");
    let before = s.doc.coords().version();
    s.set_cursor(2);
    s.insert("c");
    assert!(!s.doc.coords().is(before), "an edit makes a new version");
    assert!(s.doc.coords().is(s.doc.version));
}

#[test]
fn legacy_coordinates_keep_unicode_line_breaks_and_extreme_clamping() {
    for (text, ends) in [("", &[0][..]), ("a\n", &[1, 2]), ("a\r\n", &[1, 3]), ("x\ry", &[3]), ("é🦀\r\n猫", &[2, 5])]
    {
        let d = Document::from_text(text);
        for (line, &end) in ends.iter().enumerate() {
            assert_eq!(d.line_end(line), end, "{text:?}, line {line}");
            assert_eq!(d.offset(line, usize::MAX), end);
        }
        assert_eq!(d.line_end(usize::MAX), d.len());
        assert_eq!(d.offset(usize::MAX, 0), d.len());
        for at in 0..=d.len() {
            let prefix: String = text.chars().take(at).collect();
            let line = prefix.matches('\n').count();
            let column = prefix.rsplit('\n').next().unwrap().chars().count();
            let utf16 = prefix.encode_utf16().count();
            assert_eq!(d.point(at), (line, column));
            assert_eq!(d.char_to_utf16(at), utf16);
            assert_eq!(d.utf16_to_char(utf16), at);
        }
    }
}
