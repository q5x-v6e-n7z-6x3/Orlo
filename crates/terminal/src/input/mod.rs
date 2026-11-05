//! Platform-independent terminal keystrokes, paste encoding, and PTY commands.

#[derive(Debug)]
pub enum PtyCommand {
    Input(Vec<u8>),
    Resize(u16, u16),
    SyncFolder(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Key<'a> {
    pub name: &'a str,
    pub modifiers: Modifiers,
}

pub fn key_bytes(key: Key<'_>, application_cursor: bool) -> Option<Vec<u8>> {
    let m = key.modifiers;
    if m.platform {
        return None;
    }
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let name = key.name;
    // F1-F4 are always SS3 sequences; the cursor keys only in application cursor mode
    let final_char = match name {
        "up" => Some(('A', application_cursor)),
        "down" => Some(('B', application_cursor)),
        "right" => Some(('C', application_cursor)),
        "left" => Some(('D', application_cursor)),
        "home" => Some(('H', application_cursor)),
        "end" => Some(('F', application_cursor)),
        "f1" => Some(('P', true)),
        "f2" => Some(('Q', true)),
        "f3" => Some(('R', true)),
        "f4" => Some(('S', true)),
        _ => None,
    };
    if let Some((final_char, ss3)) = final_char {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_char}")
            } else if ss3 {
                format!("\x1bO{final_char}")
            } else {
                format!("\x1b[{final_char}")
            }
            .into_bytes(),
        );
    }
    let tilde = match name {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    };
    if let Some(code) = tilde {
        return Some(
            if modifier > 1 { format!("\x1b[{code};{modifier}~") } else { format!("\x1b[{code}~") }.into_bytes(),
        );
    }
    let mut bytes = match name {
        "enter" => vec![b'\r'],
        "backspace" => vec![if m.control { 8 } else { 127 }],
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }
        }
        "escape" => vec![27],
        "space" | "@" | "2" if m.control => vec![0],
        "6" if m.control => vec![30],
        "-" if m.control => vec![31],
        _ if m.control && name.len() == 1 => {
            let b = name.as_bytes()[0].to_ascii_uppercase();
            if (b'@'..=b'_').contains(&b) {
                vec![b & 0x1f]
            } else {
                return None;
            }
        }
        _ => return None,
    };
    if m.alt {
        bytes.insert(0, 27);
    }
    Some(bytes)
}

/// Normalize clipboard line endings and apply the shell's paste protocol.
pub fn paste_bytes(text: &str, bracketed_paste: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if bracketed_paste {
        // only tabs and line breaks stay: ESC would end the bracket early, and ^C, ^Z or ^\
        // reach the line discipline as signals, aborting the paste so the rest runs as typed
        let text: String = text.chars().filter(|&c| !c.is_control() || matches!(c, '\t' | '\n')).collect();
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace('\n', "\r").into_bytes()
    }
}

pub fn selection_bytes(text: &str, bracketed: bool, execute: bool) -> Result<Vec<u8>, String> {
    let mut bytes = paste_bytes(text, bracketed);
    if execute {
        bytes.push(b'\r');
    }
    if bytes.len() > crate::configuration::MAX_INPUT {
        return Err("Terminal input is limited to 1 MiB per send".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t07_unbracketed_multiline_send_uses_the_existing_paste_protocol_without_extra_enter() {
        assert_eq!(selection_bytes("é\r\n中\n", false, false).unwrap(), "é\r中\r".as_bytes());
        assert_eq!(selection_bytes("é\r\n中\n", false, true).unwrap(), "é\r中\r\r".as_bytes());
    }

    #[test]
    fn editor_selection_paste_and_execution_are_distinct_atomic_inputs() {
        assert_eq!(selection_bytes("echo 中", false, false).unwrap(), "echo 中".as_bytes());
        assert_eq!(selection_bytes("echo 中", false, true).unwrap(), "echo 中\r".as_bytes());
        assert_eq!(selection_bytes("one\ntwo\n", false, false).unwrap(), b"one\rtwo\r");
        assert_eq!(selection_bytes("one\r\ntwo\n", true, false).unwrap(), b"\x1b[200~one\ntwo\n\x1b[201~");
        assert_eq!(selection_bytes("one\ntwo", true, true).unwrap(), b"\x1b[200~one\ntwo\x1b[201~\r");
    }

    #[test]
    fn bracketed_paste_drops_signal_and_other_controls() {
        assert_eq!(paste_bytes("echo hi\x03\nrm -rf ~\n", true), b"\x1b[200~echo hi\nrm -rf ~\n\x1b[201~");
        assert_eq!(paste_bytes("a\x1a\x1c\x7f\u{9b}\x1b[201~b\tc", true), b"\x1b[200~a[201~b\tc\x1b[201~");
        assert!(selection_bytes(&"x".repeat(crate::configuration::MAX_INPUT), true, true).is_err());
    }

    #[test]
    fn key_codes_use_terminal_control_and_application_cursor_sequences() {
        let control = Modifiers { control: true, ..Modifiers::default() };
        let shift = Modifiers { shift: true, ..Modifiers::default() };
        for (name, modifiers, application, expected) in [
            ("c", control, false, "\x03"),
            ("d", control, false, "\x04"),
            ("space", control, false, "\0"),
            ("up", Modifiers::default(), false, "\x1b[A"),
            ("up", Modifiers::default(), true, "\x1bOA"),
            ("left", control, false, "\x1b[1;5D"),
            ("tab", shift, false, "\x1b[Z"),
            ("delete", Modifiers::default(), false, "\x1b[3~"),
            ("f5", Modifiers::default(), false, "\x1b[15~"),
        ] {
            assert_eq!(key_bytes(Key { name, modifiers }, application), Some(expected.as_bytes().to_vec()), "{name}");
        }
        assert_eq!(
            key_bytes(Key { name: "a", modifiers: Modifiers::default() }, false),
            None,
            "ordinary input belongs to the IME"
        );
        assert_eq!(
            key_bytes(Key { name: "c", modifiers: Modifiers { platform: true, ..Modifiers::default() } }, false),
            None
        );
    }

    #[test]
    fn paste_normalizes_line_endings_and_strips_escape_from_bracketed_input() {
        assert_eq!(paste_bytes("one\r\ntwo\rthree\n", false), b"one\rtwo\rthree\r");
        assert_eq!(paste_bytes("one\r\n\x1btwo", true), b"\x1b[200~one\ntwo\x1b[201~");
    }
}
