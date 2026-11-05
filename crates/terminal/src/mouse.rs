//! Mouse reporting requested by terminal applications.

use crate::{Modifiers, Screen};
use vt100::{MouseProtocolEncoding as Encoding, MouseProtocolMode as Mode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Press,
    Release,
    Motion,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug)]
pub struct Mouse {
    pub kind: Kind,
    /// Left = 0, middle = 1, right = 2; None for unpressed motion.
    pub button: Option<u8>,
    pub row: u16,
    pub col: u16,
    pub modifiers: Modifiers,
}

pub fn requested(screen: &Screen) -> bool {
    screen.mouse_protocol_mode() != Mode::None
}

pub fn mouse_bytes(screen: &Screen, event: Mouse) -> Option<Vec<u8>> {
    let mode = screen.mouse_protocol_mode();
    let encoding = screen.mouse_protocol_encoding();
    let Mouse { kind, button, row, col, modifiers: m } = event;
    let (rows, cols) = screen.size();
    if mode == Mode::None || m.shift || m.platform || row >= rows || col >= cols || button.is_some_and(|b| b > 2) {
        return None;
    }
    let allowed = match kind {
        Kind::Press | Kind::WheelUp | Kind::WheelDown => true,
        Kind::Release => mode != Mode::Press,
        Kind::Motion => mode == Mode::AnyMotion || mode == Mode::ButtonMotion && button.is_some(),
    };
    if !allowed {
        return None;
    }
    let button = button.unwrap_or(3);
    let code = match kind {
        Kind::Press => button,
        Kind::Release => {
            if encoding == Encoding::Sgr {
                button
            } else {
                3
            }
        }
        Kind::Motion => 32 + button,
        Kind::WheelUp => 64,
        Kind::WheelDown => 65,
    } + 8 * u8::from(m.alt)
        + 16 * u8::from(m.control);
    if encoding == Encoding::Sgr {
        let end = if kind == Kind::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes());
    }
    let values = [code as u32 + 32, col as u32 + 33, row as u32 + 33];
    let mut bytes = b"\x1b[M".to_vec();
    for value in values {
        if encoding == Encoding::Default {
            bytes.push(u8::try_from(value).ok()?);
        } else {
            bytes.extend_from_slice(char::from_u32(value)?.encode_utf8(&mut [0; 4]).as_bytes());
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Emulator, Modifiers};

    #[test]
    fn mouse_modes_encode_only_requested_events_and_never_shift_selection() {
        let mut e = Emulator::with_size(256, 512, 0);
        let mut event = Mouse { kind: Kind::Press, button: Some(0), row: 2, col: 4, modifiers: Modifiers::default() };
        assert_eq!(mouse_bytes(e.screen(), event), None);
        e.process(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<0;5;3M".to_vec()));
        event.kind = Kind::Release;
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<0;5;3m".to_vec()));
        event.kind = Kind::Motion;
        assert_eq!(mouse_bytes(e.screen(), event), None);
        e.process(b"\x1b[?1002h");
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<32;5;3M".to_vec()));
        event.button = None;
        assert_eq!(mouse_bytes(e.screen(), event), None);
        e.process(b"\x1b[?1003h");
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<35;5;3M".to_vec()));
        event.modifiers.shift = true;
        assert_eq!(mouse_bytes(e.screen(), event), None);
        event.modifiers = Modifiers { alt: true, control: true, ..Modifiers::default() };
        event.kind = Kind::WheelUp;
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<88;5;3M".to_vec()));
        event.col = 512;
        assert_eq!(mouse_bytes(e.screen(), event), None);
    }

    #[test]
    fn legacy_and_utf8_coordinates_do_not_wrap_and_x10_omits_releases() {
        let mut e = Emulator::with_size(256, 512, 0);
        e.process(b"\x1b[?9h");
        let mut event = Mouse { kind: Kind::Press, button: Some(2), row: 0, col: 0, modifiers: Modifiers::default() };
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[M\x22!!".to_vec()));
        event.kind = Kind::Release;
        assert_eq!(mouse_bytes(e.screen(), event), None);
        event.kind = Kind::Press;
        event.col = 300;
        assert_eq!(mouse_bytes(e.screen(), event), None);
        e.process(b"\x1b[?1005h");
        assert_eq!(mouse_bytes(e.screen(), event), Some("\x1b[M\x22ō!".as_bytes().to_vec()));
    }

    #[test]
    fn exit_clears_mouse_modes_without_resetting_final_output_or_other_input_modes() {
        for mode in [9, 1000, 1002, 1003] {
            for encoding in [1005, 1006] {
                let mut e = Emulator::with_size(3, 40, 20);
                e.set_directory("/old".into());
                e.process(b"old\r\nline2\r\nline3\r\nlast\x1b[?1049hfinal.rs:2:3");
                e.process(format!("\x1b[?{mode}h\x1b[?{encoding}h\x1b[?1h\x1b=\x1b[?2004h").as_bytes());
                let before = e.screen().clone();
                // A partial escape at EOF must not swallow lifecycle cleanup.
                e.process(b"\x1b[");
                e.exited(7);
                assert!(!requested(e.screen()), "mode {mode}, encoding {encoding}");
                assert_eq!(e.screen().mouse_protocol_encoding(), Encoding::Default);
                assert_eq!(e.screen().contents(), before.contents());
                assert_eq!(e.screen().contents_formatted(), before.contents_formatted());
                assert_eq!(e.screen().cursor_position(), before.cursor_position());
                assert!(e.screen().same_layout(&before));
                assert_eq!(e.screen().directory_at(0, 0), Some(std::path::Path::new("/old")));
                assert_eq!(e.working_directory(), Some(std::path::Path::new("/old")));
                assert!(e.screen().alternate_screen());
                assert!(e.screen().application_cursor());
                assert!(e.screen().application_keypad());
                assert!(e.screen().bracketed_paste());
                assert!(e.take_replies().is_empty());
                e.process(b"m\x1b[?1049l");
                assert!(e.screen().contents().contains("last"));
                e.screen_mut().set_scrollback(20);
                assert!(e.screen().contents().contains("old"));
            }
        }
    }

    #[test]
    fn alternate_screen_keeps_reporting_until_explicit_disable_or_terminal_reset() {
        let mut e = Emulator::with_size(3, 40, 0);
        let event = Mouse { kind: Kind::WheelDown, button: None, row: 2, col: 39, modifiers: Modifiers::default() };
        e.process(b"\x1b[?1003h\x1b[?1006h\x1b[?1049h");
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<65;40;3M".to_vec()));
        e.process(b"\x1b[?1049l");
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<65;40;3M".to_vec()));
        e.process(b"\x1b[?1003l");
        assert_eq!(mouse_bytes(e.screen(), event), None);
        e.process(b"\x1b[?1003h\x1bc");
        assert_eq!(mouse_bytes(e.screen(), event), None);
        assert_eq!(e.screen().mouse_protocol_encoding(), Encoding::Default);
    }

    #[test]
    fn each_mode_admits_only_its_event_classes_and_coordinate_boundaries() {
        for (mode, release, drag, motion) in
            [(9, false, false, false), (1000, true, false, false), (1002, true, true, false), (1003, true, true, true)]
        {
            let mut e = Emulator::with_size(3, 512, 0);
            e.process(format!("\x1b[?{mode}h\x1b[?1006h").as_bytes());
            for (kind, button, allowed) in [
                (Kind::Press, Some(1), true),
                (Kind::Release, Some(1), release),
                (Kind::Motion, Some(1), drag),
                (Kind::Motion, None, motion),
                (Kind::WheelUp, None, true),
                (Kind::WheelDown, None, true),
            ] {
                let event = Mouse { kind, button, row: 2, col: 511, modifiers: Modifiers::default() };
                assert_eq!(mouse_bytes(e.screen(), event).is_some(), allowed, "mode {mode}, {kind:?}, {button:?}");
                for modifiers in [
                    Modifiers { shift: true, ..Default::default() },
                    Modifiers { platform: true, ..Default::default() },
                ] {
                    assert_eq!(mouse_bytes(e.screen(), Mouse { modifiers, ..event }), None);
                }
                for event in
                    [Mouse { row: 3, ..event }, Mouse { col: 512, ..event }, Mouse { button: Some(3), ..event }]
                {
                    assert_eq!(mouse_bytes(e.screen(), event), None);
                }
            }
        }
        let mut e = Emulator::with_size(3, 512, 0);
        e.process(b"\x1b[?1000h");
        let event = Mouse {
            kind: Kind::Release,
            button: Some(2),
            row: 0,
            col: 222,
            modifiers: Modifiers { alt: true, control: true, ..Default::default() },
        };
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[M;\xff!".to_vec()));
        assert_eq!(mouse_bytes(e.screen(), Mouse { col: 223, ..event }), None);
        e.process(b"\x1b[?1005h");
        assert_eq!(mouse_bytes(e.screen(), event), Some("\x1b[M;ÿ!".as_bytes().to_vec()));
        e.process(b"\x1b[?1006h");
        assert_eq!(mouse_bytes(e.screen(), event), Some(b"\x1b[<26;223;1m".to_vec()));
    }
}
