//! Explicit file codecs; malformed reads and non-round-tripping writes are protected.

use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    Windows1250,
    Windows1251,
    Windows1252,
    Windows1253,
    Windows1254,
    Windows1255,
    Windows1256,
    Windows1257,
    Windows1258,
    Iso8859_1,
    Iso8859_2,
    Iso8859_5,
    Iso8859_7,
    Iso8859_15,
    ShiftJis,
    EucJp,
    Iso2022Jp,
    Gbk,
    Gb18030,
    Big5,
    EucKr,
}

impl Encoding {
    pub const ALL: [Self; 26] = [
        Self::Utf8,
        Self::Utf16Le,
        Self::Utf16Be,
        Self::Utf32Le,
        Self::Utf32Be,
        Self::Windows1250,
        Self::Windows1251,
        Self::Windows1252,
        Self::Windows1253,
        Self::Windows1254,
        Self::Windows1255,
        Self::Windows1256,
        Self::Windows1257,
        Self::Windows1258,
        Self::Iso8859_1,
        Self::Iso8859_2,
        Self::Iso8859_5,
        Self::Iso8859_7,
        Self::Iso8859_15,
        Self::ShiftJis,
        Self::EucJp,
        Self::Iso2022Jp,
        Self::Gbk,
        Self::Gb18030,
        Self::Big5,
        Self::EucKr,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Utf32Le => "UTF-32 LE",
            Self::Utf32Be => "UTF-32 BE",
            Self::Windows1250 => "Windows-1250",
            Self::Windows1251 => "Windows-1251",
            Self::Windows1252 => "Windows-1252",
            Self::Windows1253 => "Windows-1253",
            Self::Windows1254 => "Windows-1254",
            Self::Windows1255 => "Windows-1255",
            Self::Windows1256 => "Windows-1256",
            Self::Windows1257 => "Windows-1257",
            Self::Windows1258 => "Windows-1258",
            Self::Iso8859_1 => "ISO-8859-1",
            Self::Iso8859_2 => "ISO-8859-2",
            Self::Iso8859_5 => "ISO-8859-5",
            Self::Iso8859_7 => "ISO-8859-7",
            Self::Iso8859_15 => "ISO-8859-15",
            Self::ShiftJis => "Shift JIS",
            Self::EucJp => "EUC-JP",
            Self::Iso2022Jp => "ISO-2022-JP",
            Self::Gbk => "GBK",
            Self::Gb18030 => "GB18030",
            Self::Big5 => "Big5",
            Self::EucKr => "EUC-KR",
        }
    }

    pub fn bom_bytes(self) -> &'static [u8] {
        match self {
            Self::Utf8 => &[0xef, 0xbb, 0xbf],
            Self::Utf16Le => &[0xff, 0xfe],
            Self::Utf16Be => &[0xfe, 0xff],
            Self::Utf32Le => &[0xff, 0xfe, 0, 0],
            Self::Utf32Be => &[0, 0, 0xfe, 0xff],
            _ => &[],
        }
    }

    /// Exact constants, never WHATWG label aliases (notably Latin-1 → Windows-1252).
    pub(crate) fn legacy(self) -> Option<&'static encoding_rs::Encoding> {
        match self {
            Self::Windows1250 => Some(encoding_rs::WINDOWS_1250),
            Self::Windows1251 => Some(encoding_rs::WINDOWS_1251),
            Self::Windows1252 => Some(encoding_rs::WINDOWS_1252),
            Self::Windows1253 => Some(encoding_rs::WINDOWS_1253),
            Self::Windows1254 => Some(encoding_rs::WINDOWS_1254),
            Self::Windows1255 => Some(encoding_rs::WINDOWS_1255),
            Self::Windows1256 => Some(encoding_rs::WINDOWS_1256),
            Self::Windows1257 => Some(encoding_rs::WINDOWS_1257),
            Self::Windows1258 => Some(encoding_rs::WINDOWS_1258),
            Self::Iso8859_2 => Some(encoding_rs::ISO_8859_2),
            Self::Iso8859_5 => Some(encoding_rs::ISO_8859_5),
            Self::Iso8859_7 => Some(encoding_rs::ISO_8859_7),
            Self::Iso8859_15 => Some(encoding_rs::ISO_8859_15),
            Self::ShiftJis => Some(encoding_rs::SHIFT_JIS),
            Self::EucJp => Some(encoding_rs::EUC_JP),
            Self::Iso2022Jp => Some(encoding_rs::ISO_2022_JP),
            Self::Gbk => Some(encoding_rs::GBK),
            Self::Gb18030 => Some(encoding_rs::GB18030),
            Self::Big5 => Some(encoding_rs::BIG5),
            Self::EucKr => Some(encoding_rs::EUC_KR),
            _ => None,
        }
    }
}

pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    pub bom: bool,
    pub lossy: bool,
}

pub fn bom_encoding(bytes: &[u8]) -> Option<Encoding> {
    // UTF-32 LE starts with the complete UTF-16 LE signature.
    [Encoding::Utf32Le, Encoding::Utf32Be, Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be]
        .into_iter()
        .find(|e| bytes.starts_with(e.bom_bytes()))
}

pub fn decode(bytes: &[u8], explicit: Option<Encoding>) -> Decoded {
    let marked = bom_encoding(bytes);
    let encoding = explicit.or(marked).unwrap_or_default();
    let bom = marked == Some(encoding);
    let mut lossy = marked.is_some_and(|m| m != encoding);
    let body = if bom { &bytes[encoding.bom_bytes().len()..] } else { bytes };
    let text = match encoding {
        Encoding::Utf8 => match std::str::from_utf8(body) {
            Ok(s) => s.to_string(),
            Err(_) => {
                lossy = true;
                String::from_utf8_lossy(body).into_owned()
            }
        },
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let mut text = String::with_capacity(body.len());
            let (pairs, remainder) = body.as_chunks::<2>();
            for c in char::decode_utf16(pairs.iter().map(|b| match encoding {
                Encoding::Utf16Le => u16::from_le_bytes(*b),
                _ => u16::from_be_bytes(*b),
            })) {
                text.push(c.unwrap_or_else(|_| {
                    lossy = true;
                    char::REPLACEMENT_CHARACTER
                }));
            }
            if !remainder.is_empty() {
                lossy = true;
                text.push(char::REPLACEMENT_CHARACTER);
            }
            text
        }
        Encoding::Utf32Le | Encoding::Utf32Be => {
            let mut text = String::with_capacity(body.len());
            let (units, remainder) = body.as_chunks::<4>();
            for unit in units {
                let scalar =
                    if encoding == Encoding::Utf32Le { u32::from_le_bytes(*unit) } else { u32::from_be_bytes(*unit) };
                text.push(char::from_u32(scalar).unwrap_or_else(|| {
                    lossy = true;
                    char::REPLACEMENT_CHARACTER
                }));
            }
            if !remainder.is_empty() {
                lossy = true;
                text.push(char::REPLACEMENT_CHARACTER);
            }
            text
        }
        Encoding::Iso8859_1 => body.iter().map(|&b| char::from(b)).collect(),
        _ => {
            let (text, errors) = encoding.legacy().unwrap().decode_without_bom_handling(body);
            lossy |= errors;
            text.into_owned()
        }
    };
    Decoded { text, encoding, bom, lossy }
}

/// Keep the common UTF-8 load zero-copy while other codecs decode into Unicode text.
pub fn decode_owned(bytes: Vec<u8>, explicit: Option<Encoding>) -> Decoded {
    let marked = bom_encoding(&bytes);
    let encoding = explicit.or(marked).unwrap_or_default();
    if encoding != Encoding::Utf8 {
        return decode(&bytes, explicit);
    }
    let bom = marked == Some(Encoding::Utf8);
    let (mut text, lossy) = match String::from_utf8(bytes) {
        Ok(s) => (s, marked.is_some_and(|m| m != encoding)),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    };
    if bom {
        text.drain(..3);
    }
    Decoded { text, encoding, bom, lossy }
}

/// Decode a growing file without publishing replacement characters for its unfinished tail.
/// `None` means the leading bytes are still an incomplete BOM.
pub fn decode_follow(bytes: &[u8], hint: Encoding) -> io::Result<Option<(Decoded, bool)>> {
    use encoding_rs::DecoderResult;
    let unicode = [Encoding::Utf32Le, Encoding::Utf32Be, Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be];
    if !bytes.is_empty()
        && unicode.iter().any(|e| bytes.len() < e.bom_bytes().len() && e.bom_bytes().starts_with(bytes))
    {
        return Ok(None);
    }
    let marked = bom_encoding(bytes);
    let encoding = marked.unwrap_or(hint);
    let body = if marked.is_some() { &bytes[encoding.bom_bytes().len()..] } else { bytes };
    let malformed = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Malformed {} log data; stop following to reload or choose a codec", encoding.name()),
        )
    };
    let (text, partial) = match encoding {
        Encoding::Utf32Le | Encoding::Utf32Be => {
            let (units, tail) = body.as_chunks::<4>();
            let mut text = String::with_capacity(body.len());
            for unit in units {
                let scalar =
                    if encoding == Encoding::Utf32Le { u32::from_le_bytes(*unit) } else { u32::from_be_bytes(*unit) };
                text.push(char::from_u32(scalar).ok_or_else(malformed)?);
            }
            (text, !tail.is_empty())
        }
        Encoding::Iso8859_1 => (body.iter().map(|&b| char::from(b)).collect(), false),
        _ => {
            let codec = match encoding {
                Encoding::Utf8 => encoding_rs::UTF_8,
                Encoding::Utf16Le => encoding_rs::UTF_16LE,
                Encoding::Utf16Be => encoding_rs::UTF_16BE,
                _ => encoding.legacy().unwrap(),
            };
            let mut decoder = codec.new_decoder_without_bom_handling();
            let capacity = decoder.max_utf8_buffer_length_without_replacement(body.len()).ok_or_else(malformed)?;
            let mut text = String::with_capacity(capacity);
            let (result, read) = decoder.decode_to_string_without_replacement(body, &mut text, false);
            if result != DecoderResult::InputEmpty || read != body.len() {
                return Err(malformed());
            }
            // Flush only to learn whether a final code unit/escape is pending; never publish
            // substitution. Re-decode the bounded snapshot on the next changed stamp so a
            // stateful codec always starts with the file's actual shift state.
            let end = text.len();
            text.reserve(decoder.max_utf8_buffer_length_without_replacement(0).ok_or_else(malformed)?);
            let (result, _) = decoder.decode_to_string_without_replacement(b"", &mut text, true);
            let partial = matches!(result, DecoderResult::Malformed(_, _));
            text.truncate(end);
            (text, partial)
        }
    };
    Ok(Some((Decoded { text, encoding, bom: marked.is_some(), lossy: false }, partial)))
}

fn check_bom(bytes: &[u8], encoding: Encoding, bom: bool) -> io::Result<()> {
    if bom_encoding(bytes) != bom.then_some(encoding) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The leading text bytes would change the encoding or BOM on reopen; choose another encoding or BOM option",
        ));
    }
    Ok(())
}

/// Locations describe source Unicode scalars, with CRLF treated as one line break.
fn conversion_failure(text: &str, encoding: Encoding, at: usize) -> io::Error {
    let (mut line, mut column, mut cr) = (1, 1, false);
    for c in text[..at].chars() {
        match c {
            '\r' => (line, column) = (line + 1, 1),
            '\n' if !cr => (line, column) = (line + 1, 1),
            '\n' => {}
            _ => column += 1,
        }
        cr = c == '\r';
    }
    let character = text[at..]
        .chars()
        .next()
        .map_or_else(|| "end of text".to_string(), |c| format!("{c:?} (U+{:04X})", u32::from(c)));
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Text cannot be saved losslessly as {}: {character} at line {line}, column {column}; \
             choose a Unicode encoding",
            encoding.name()
        ),
    )
}

/// Encode without substitution or Unicode changes. Legacy aliases that encode successfully
/// but decode to different scalars (for example Shift JIS yen) are conversion errors too.
pub fn encode(text: &str, encoding: Encoding, bom: bool) -> io::Result<Vec<u8>> {
    if bom && encoding.bom_bytes().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} does not support a BOM", encoding.name())));
    }
    if let Some(codec) = encoding.legacy() {
        let (bytes, _, errors) = codec.encode(text);
        let (decoded, malformed) = codec.decode_without_bom_handling(&bytes);
        if errors || malformed || decoded != text {
            // The whole-string round trip already identifies both unmappable scalars and
            // aliases; inspect its first difference only on failure, never per-scalar encode.
            let mut converted = decoded.chars();
            let at = text.char_indices().find(|(_, c)| Some(*c) != converted.next()).map_or(text.len(), |(at, _)| at);
            return Err(conversion_failure(text, encoding, at));
        }
        check_bom(&bytes, encoding, bom)?;
        return Ok(bytes.into_owned());
    }
    let mut bytes = Vec::with_capacity(text.len().saturating_add(4));
    if bom {
        bytes.extend_from_slice(encoding.bom_bytes());
    }
    match encoding {
        Encoding::Utf8 => bytes.extend_from_slice(text.as_bytes()),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&if encoding == Encoding::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
        }
        Encoding::Utf32Le | Encoding::Utf32Be => {
            for scalar in text.chars().map(u32::from) {
                bytes.extend_from_slice(&if encoding == Encoding::Utf32Le {
                    scalar.to_le_bytes()
                } else {
                    scalar.to_be_bytes()
                });
            }
        }
        Encoding::Iso8859_1 => {
            for c in text.chars() {
                bytes.push(u8::try_from(u32::from(c)).map_err(|_| {
                    let at = text.find(|c| u32::from(c) > 0xff).unwrap_or(text.len());
                    conversion_failure(text, encoding, at)
                })?);
            }
        }
        _ => unreachable!("legacy codecs handled above"),
    }
    check_bom(&bytes, encoding, bom)?;
    Ok(bytes)
}

pub fn encode_owned(text: String, encoding: Encoding, bom: bool) -> io::Result<Vec<u8>> {
    if encoding == Encoding::Utf8 {
        let mut text = text;
        if bom {
            text.insert(0, '\u{feff}');
        }
        check_bom(text.as_bytes(), encoding, bom)?;
        Ok(text.into_bytes())
    } else {
        encode(&text, encoding, bom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_f01_conversion_errors_identify_first_scalar_and_source_location() {
        for ending in ["\n", "\r\n", "\r"] {
            for encoding in [Encoding::Windows1252, Encoding::Iso8859_1] {
                let text = format!("é\tX{ending}é\u{a0}😀🚀");
                let error = encode(&text, encoding, false).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert_eq!(error.to_string(), format!(
                    "Text cannot be saved losslessly as {}: '😀' (U+1F600) at line 2, column 3; choose a Unicode encoding",
                    encoding.name()
                ));
                assert_eq!(encode_owned(text, encoding, false).unwrap_err().to_string(), error.to_string());
            }
            for encoding in [Encoding::ShiftJis, Encoding::EucJp] {
                // Yen encodes without an encoder error but decodes as backslash. It must be
                // reported before the later emoji, rather than only finding unmappable input.
                let text = format!("日本{ending}本¥😀");
                let error = encode(&text, encoding, false).unwrap_err().to_string();
                assert!(error.contains("'¥' (U+00A5) at line 2, column 2"), "{encoding:?}: {error}");
            }
        }
        let error = encode("日本\r\n本😀", Encoding::Iso2022Jp, false).unwrap_err().to_string();
        assert!(error.contains("'😀' (U+1F600) at line 2, column 2"), "{error}");
        let error = encode("é\t\u{301}😀", Encoding::Windows1258, false).unwrap_err().to_string();
        assert!(error.contains("'😀' (U+1F600) at line 1, column 4"), "{error}");
        let error = encode("é\r\nø\rñ\nü😀", Encoding::Windows1252, false).unwrap_err().to_string();
        assert!(error.contains("line 4, column 2"), "{error}");
    }

    #[test]
    fn follow_decodes_every_partial_unicode_and_cjk_prefix_without_substitution() {
        for (encoding, text) in [
            (Encoding::Utf8, "a😀中"),
            (Encoding::Utf16Le, "a😀中"),
            (Encoding::Utf16Be, "a😀中"),
            (Encoding::Utf32Le, "a😀中"),
            (Encoding::Utf32Be, "a😀中"),
            (Encoding::ShiftJis, "a日本"),
            (Encoding::EucJp, "a日本"),
            (Encoding::Iso2022Jp, "a日本"),
            (Encoding::Gbk, "a中文"),
            (Encoding::Gb18030, "a😀中"),
            (Encoding::Big5, "a中文"),
            (Encoding::EucKr, "a한글"),
        ] {
            for bom in [false, true].into_iter().filter(|b| !b || !encoding.bom_bytes().is_empty()) {
                let bytes = encode(text, encoding, bom).unwrap();
                for end in 0..=bytes.len() {
                    if let Some((decoded, _)) = decode_follow(&bytes[..end], encoding).unwrap() {
                        assert!(!decoded.lossy, "{encoding:?} prefix {end}");
                        assert!(text.starts_with(&decoded.text), "{encoding:?} prefix {end}: {:?}", decoded.text);
                    }
                }
                let (decoded, partial) = decode_follow(&bytes, encoding).unwrap().unwrap();
                assert_eq!(decoded.text, text);
                assert!(!partial);
            }
        }
    }

    #[test]
    fn follow_rejects_malformed_bytes_and_honors_external_bom() {
        for (encoding, bytes) in [
            (Encoding::Utf8, &b"a\xffb"[..]),
            (Encoding::Utf16Le, &[0, 0xdc, 65, 0][..]),
            (Encoding::Utf32Be, &[0, 0x11, 0, 0][..]),
            (Encoding::ShiftJis, &[0x81, 0x20][..]),
            (Encoding::Iso2022Jp, &[0x1b, b'X'][..]),
        ] {
            assert!(decode_follow(bytes, encoding).is_err(), "{encoding:?}");
        }
        let bytes = encode("中😀", Encoding::Utf32Be, true).unwrap();
        let (decoded, partial) = decode_follow(&bytes, Encoding::Windows1252).unwrap().unwrap();
        assert_eq!((decoded.encoding, decoded.text.as_str(), partial), (Encoding::Utf32Be, "中😀", false));
        for encoding in Encoding::ALL {
            let bytes = encode("ABC", encoding, false).unwrap();
            assert_eq!(decode_follow(&bytes, encoding).unwrap().unwrap().0.text, "ABC");
        }
    }

    #[test]
    fn output_cannot_accidentally_change_bom_interpretation() {
        for (text, encoding, bom) in [
            ("\u{feff}text", Encoding::Utf8, false),
            ("\u{feff}text", Encoding::Utf32Be, false),
            ("\0text", Encoding::Utf16Le, true),
            ("ï»¿text", Encoding::Windows1252, false),
            ("ÿþtext", Encoding::Iso8859_1, false),
        ] {
            assert!(encode(text, encoding, bom).is_err(), "{encoding:?}: {text:?}");
            assert!(encode_owned(text.into(), encoding, bom).is_err());
        }
        assert_eq!(decode(&encode("\u{feff}text", Encoding::Utf8, true).unwrap(), None).text, "\u{feff}text");
    }

    #[test]
    fn every_codec_roundtrips_and_rejects_silent_substitution() {
        for encoding in Encoding::ALL {
            let text = "ASCII text\r\n";
            let bytes = encode(text, encoding, false).unwrap();
            assert_eq!(decode(&bytes, Some(encoding)).text, text, "{encoding:?}");
            if encoding.bom_bytes().is_empty() {
                assert!(encode(text, encoding, true).is_err(), "{encoding:?}");
            }
        }
        for encoding in
            [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp, Encoding::Windows1252, Encoding::Iso8859_1]
        {
            assert!(encode("🦀", encoding, false).is_err(), "{encoding:?}");
        }
        for encoding in [Encoding::ShiftJis, Encoding::EucJp] {
            for text in ["¥", "‾", "−"] {
                assert!(encode(text, encoding, false).is_err(), "{encoding:?}: {text}");
            }
        }
        let c1: Vec<_> = (0x80..=0x9f).collect();
        let latin = decode(&c1, Some(Encoding::Iso8859_1));
        assert_eq!(latin.text.chars().map(u32::from).collect::<Vec<_>>(), (0x80..=0x9f).collect::<Vec<_>>());
        assert_eq!(encode(&latin.text, Encoding::Iso8859_1, false).unwrap(), c1);
        assert_ne!(latin.text, decode(&c1, Some(Encoding::Windows1252)).text);
    }

    #[test]
    fn malformed_legacy_and_utf32_reads_remain_protected() {
        for (bytes, encoding) in [
            (&b"\x93"[..], Encoding::ShiftJis),
            (&b"\x81"[..], Encoding::Gbk),
            (&b"\x1b$"[..], Encoding::Iso2022Jp),
            (&[0, 0xd8, 0, 0][..], Encoding::Utf32Le),
            (&[0, 0x11, 0, 0][..], Encoding::Utf32Be),
            (&[0x61, 0, 0][..], Encoding::Utf32Le),
        ] {
            assert!(decode(bytes, Some(encoding)).lossy, "{encoding:?}: {bytes:?}");
        }
        assert!(decode(b"\xef\xbb\xbfabc", Some(Encoding::Windows1252)).lossy);
        assert!(decode(&[0xff, 0xfe, 0, 0], Some(Encoding::Utf16Le)).lossy);
    }

    #[test]
    fn utf32_bom_precedes_utf16_and_validates_scalars() {
        for (bytes, name) in [
            (&[0xff, 0xfe, 0, 0, 0x41, 0, 0, 0, 0, 0xf6, 1, 0][..], "UTF-32 LE"),
            (&[0, 0, 0xfe, 0xff, 0, 0, 0, 0x41, 0, 1, 0xf6, 0][..], "UTF-32 BE"),
        ] {
            let d = decode(bytes, None);
            assert_eq!(d.text, "A😀");
            assert_eq!(d.encoding.name(), name);
            assert!(d.bom && !d.lossy);
        }
    }

    #[test]
    fn legacy_fixtures_use_exact_mappings() {
        for (name, bytes, expected) in [
            ("Windows1250", &[0xa3][..], "Ł"),
            ("Windows1251", &[0xdf][..], "Я"),
            ("Windows1253", &[0xd9][..], "Ω"),
            ("Windows1254", &[0xd0][..], "Ğ"),
            ("Windows1255", &[0xf9][..], "ש"),
            ("Windows1256", &[0xd4][..], "ش"),
            ("Windows1257", &[0xc2][..], "Ā"),
            ("Windows1258", &[0xf5][..], "ơ"),
            ("Iso8859_2", &[0xa3][..], "Ł"),
            ("Iso8859_5", &[0xcf][..], "Я"),
            ("Iso8859_7", &[0xd9][..], "Ω"),
            ("Iso8859_15", &[0xa4][..], "€"),
            ("Iso2022Jp", &[0x1b, 0x24, 0x42, 0x46, 0x7c, 0x4b, 0x5c, 0x1b, 0x28, 0x42][..], "日本"),
            ("Gb18030", &[0x94, 0x39, 0xfc, 0x36][..], "😀"),
            ("Windows1252", &b"\x80\x93caf\xe9\x94"[..], "€“café”"),
            ("Iso8859_1", &b"\x80\x93caf\xe9\x94"[..], "\u{80}\u{93}café\u{94}"),
            ("ShiftJis", &b"\x93\xfa\x96\x7b"[..], "日本"),
            ("EucJp", &b"\xc6\xfc\xcb\xdc"[..], "日本"),
            ("Gbk", &b"\xd6\xd0\xce\xc4"[..], "中文"),
            ("Big5", &b"\xa4\xa4\xa4\xe5"[..], "中文"),
            ("EucKr", &b"\xc7\xd1\xb1\xdb"[..], "한글"),
        ] {
            let encoding: Encoding = serde_json::from_value(serde_json::json!(name)).expect("supported codec");
            let decoded = decode(bytes, Some(encoding));
            assert_eq!(decoded.text, expected, "{name}");
            assert!(!decoded.lossy && !decoded.bom, "{name}");
            assert_eq!(encode(expected, encoding, false).unwrap(), bytes, "{name}");
        }
    }

    #[test]
    fn utf8_serialization_reuses_owned_text() {
        let text = "plain UTF-8 café 🦀".to_string();
        let allocation = text.as_ptr();
        let bytes = encode_owned(text, Encoding::Utf8, false).unwrap();
        assert_eq!(bytes, "plain UTF-8 café 🦀".as_bytes());
        assert_eq!(bytes.as_ptr(), allocation);
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            for bom in [false, true] {
                assert_eq!(
                    encode_owned("a🦀é\r\n".into(), encoding, bom).unwrap(),
                    encode("a🦀é\r\n", encoding, bom).unwrap()
                );
            }
        }
    }

    #[test]
    fn unicode_roundtrip_and_explicit_bomless_decode() {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            for bom in [false, true] {
                let bytes = encode("a🦀é\r\n", encoding, bom).unwrap();
                let decoded = decode(&bytes, (!bom).then_some(encoding));
                let owned = decode_owned(bytes, (!bom).then_some(encoding));
                assert_eq!(
                    (&owned.text, owned.encoding, owned.bom, owned.lossy),
                    (&decoded.text, decoded.encoding, decoded.bom, decoded.lossy)
                );
                assert_eq!(decoded.text, "a🦀é\r\n");
                assert_eq!((decoded.encoding, decoded.bom, decoded.lossy), (encoding, bom, false));
            }
        }
    }

    #[test]
    fn malformed_utf16_and_conflicting_bom_are_lossy() {
        for bytes in [&[0xff, 0xfe, 0][..], &[0xff, 0xfe, 0, 0xd8][..]] {
            assert!(decode(bytes, None).lossy);
        }
        assert!(decode(&[0xff, 0xfe, b'a', 0], Some(Encoding::Utf8)).lossy);
        assert!(!decode("�".as_bytes(), None).lossy);
    }
}
