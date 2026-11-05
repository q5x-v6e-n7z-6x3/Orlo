//! Edit > Convert: named text codecs, literal escaping, and the hashes of text and files.

use base64::alphabet::{STANDARD, URL_SAFE};
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine;
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::io::{self, BufReader, Write};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Base64,
    Base64Url,
    Hex,
    UriComponent,
    HtmlEntities,
    RegexLiteral,
    JsonString,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hash {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Hash {
    pub const ALL: [Hash; 4] = [Hash::Md5, Hash::Sha1, Hash::Sha256, Hash::Sha512];

    pub fn name(self) -> &'static str {
        match self {
            Hash::Md5 => "MD5",
            Hash::Sha1 => "SHA-1",
            Hash::Sha256 => "SHA-256",
            Hash::Sha512 => "SHA-512",
        }
    }
}

const PADDED: GeneralPurpose = GeneralPurpose::new(&STANDARD, GeneralPurposeConfig::new());
// RFC 4648 section 5: written without padding, read with or without
const URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new().with_encode_padding(false).with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[usize::from(b >> 4)] as char);
        s.push(DIGITS[usize::from(b & 15)] as char);
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let mut digits = Vec::with_capacity(s.len());
    for c in s.chars().filter(|c| !c.is_ascii_whitespace()) {
        let d = c.to_digit(16).ok_or_else(|| format!("'{c}' is not a hex digit"))?;
        digits.push(d as u8);
    }
    if digits.len() % 2 != 0 {
        return Err("Hex needs an even number of digits".into());
    }
    Ok(digits.chunks(2).map(|p| p[0] << 4 | p[1]).collect())
}

fn encode_uri_component(s: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(b));
        } else {
            encoded.push('%');
            encoded.push(char::from(DIGITS[usize::from(b >> 4)]));
            encoded.push(char::from(DIGITS[usize::from(b & 15)]));
        }
    }
    encoded
}

fn decode_uri_component(s: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let mut digit = || bytes.next().filter(u8::is_ascii_hexdigit).and_then(|b| char::from(b).to_digit(16));
            let high = digit().ok_or("Invalid URI escape: expected %HH")?;
            let low = digit().ok_or("Invalid URI escape: expected %HH")?;
            decoded.push((high << 4 | low) as u8);
        } else {
            decoded.push(b);
        }
    }
    text_of(decoded)
}

fn encode_html_entities(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => encoded.push_str("&amp;"),
            '<' => encoded.push_str("&lt;"),
            '>' => encoded.push_str("&gt;"),
            '"' => encoded.push_str("&quot;"),
            '\'' => encoded.push_str("&#39;"),
            _ => encoded.push(c),
        }
    }
    encoded
}

fn decode_html_entities(mut source: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(source.len());
    while let Some(c) = source.chars().next() {
        source = &source[c.len_utf8()..];
        if c != '&' {
            decoded.push(c);
        } else if let Some(numeric) = source.strip_prefix('#') {
            let invalid = "Invalid HTML numeric entity: expected a semicolon-terminated Unicode scalar";
            let (digits, tail) = numeric.split_once(';').ok_or(invalid)?;
            let (digits, radix) = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => (hex, 16),
                None => (digits, 10),
            };
            if digits.is_empty()
                || !digits.bytes().all(|b| if radix == 16 { b.is_ascii_hexdigit() } else { b.is_ascii_digit() })
            {
                return Err(invalid.into());
            }
            let scalar = u32::from_str_radix(digits, radix).ok().and_then(char::from_u32).ok_or(invalid)?;
            decoded.push(scalar);
            source = tail;
        } else if let Some((name, value)) = [("amp;", '&'), ("lt;", '<'), ("gt;", '>'), ("quot;", '"')]
            .into_iter()
            .find(|(name, _)| source.starts_with(name))
        {
            decoded.push(value);
            source = &source[name.len()..];
        } else {
            decoded.push('&');
        }
    }
    Ok(decoded)
}

pub fn encode(c: Codec, s: &str) -> String {
    match c {
        Codec::Base64 => PADDED.encode(s),
        Codec::Base64Url => URL.encode(s),
        Codec::Hex => to_hex(s.as_bytes()),
        Codec::UriComponent => encode_uri_component(s),
        Codec::HtmlEntities => encode_html_entities(s),
        Codec::RegexLiteral => regex_syntax::escape(s),
        Codec::JsonString => serde_json::to_string(s).expect("serializing a string is infallible"),
    }
}

/// The text `s` encodes. Base64/hex ignore ASCII whitespace; URI/HTML preserve it and `+`.
/// JSON requires one string value. Regex Literal has no inverse; byte codecs require UTF-8.
pub fn decode(c: Codec, s: &str) -> Result<String, String> {
    match c {
        Codec::UriComponent => decode_uri_component(s),
        Codec::HtmlEntities => decode_html_entities(s),
        Codec::RegexLiteral => Err("Regex Literal supports Escape Regex only".into()),
        Codec::JsonString => serde_json::from_str::<String>(s).map_err(|error| format!("Invalid JSON string: {error}")),
        Codec::Hex => text_of(from_hex(s)?),
        Codec::Base64 | Codec::Base64Url => {
            let compact: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            let engine = if c == Codec::Base64 { PADDED } else { URL };
            text_of(engine.decode(compact).map_err(|e| format!("Invalid Base64: {e}"))?)
        }
    }
}

/// Decode exactly the documented text escapes once; hexadecimal bytes denote U+00HH scalars.
pub fn decode_text_escapes(s: &str) -> Result<String, String> {
    fn hex(chars: &mut std::str::Chars<'_>, count: usize) -> Result<u32, String> {
        (0..count).try_fold(0, |value, _| {
            let digit = chars.next().filter(char::is_ascii_hexdigit).and_then(|c| c.to_digit(16));
            digit.map(|digit| value * 16 + digit).ok_or_else(|| format!("Expected {count} ASCII hex digits"))
        })
    }
    let mut decoded = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            decoded.push(c);
            continue;
        }
        let escape = chars.next().ok_or("Trailing backslash in text escapes")?;
        let c = match escape {
            '\\' | '"' | '\'' => escape,
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'x' | 'u' | 'U' => {
                let count = match escape {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let mut value = hex(&mut chars, count)?;
                if escape == 'u' && (0xD800..=0xDBFF).contains(&value) {
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err("A high surrogate requires an adjacent \\u low surrogate".into());
                    }
                    let low = hex(&mut chars, 4)?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err("Expected a low surrogate after the high surrogate".into());
                    }
                    value = 0x10000 + ((value - 0xD800) << 10) + low - 0xDC00;
                }
                char::from_u32(value).ok_or("Text escape is not a Unicode scalar")?
            }
            _ => return Err(format!("Unknown text escape: \\{escape}")),
        };
        decoded.push(c);
    }
    Ok(decoded)
}

fn text_of(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| "The decoded data is not UTF-8 text".into())
}

/// Lowercase hex of the hash of `bytes`.
pub fn digest(h: Hash, bytes: &[u8]) -> String {
    match h {
        Hash::Md5 => to_hex(&Md5::digest(bytes)),
        Hash::Sha1 => to_hex(&Sha1::digest(bytes)),
        Hash::Sha256 => to_hex(&Sha256::digest(bytes)),
        Hash::Sha512 => to_hex(&Sha512::digest(bytes)),
    }
}

/// All four hashers fed by one pass over a file.
struct Hashers(Md5, Sha1, Sha256, Sha512);

impl Write for Hashers {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.update(b);
        self.1.update(b);
        self.2.update(b);
        self.3.update(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The digests of a file, in `Hash::ALL` order, read once in chunks (any size).
pub fn digest_file(path: &Path) -> io::Result<[String; 4]> {
    let mut h = Hashers(Md5::new(), Sha1::new(), Sha256::new(), Sha512::new());
    let file = std::fs::File::open(path)?;
    io::copy(&mut BufReader::with_capacity(1 << 16, file), &mut h)?;
    Ok([to_hex(&h.0.finalize()), to_hex(&h.1.finalize()), to_hex(&h.2.finalize()), to_hex(&h.3.finalize())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_vectors_in_both_alphabets() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (text, b64) in vectors {
            assert_eq!(encode(Codec::Base64, text), b64);
            assert_eq!(decode(Codec::Base64, b64).as_deref(), Ok(text));
            // the URL-safe form drops the padding and accepts it back
            let url = b64.trim_end_matches('=');
            assert_eq!(encode(Codec::Base64Url, text), url);
            assert_eq!(decode(Codec::Base64Url, url).as_deref(), Ok(text));
            assert_eq!(decode(Codec::Base64Url, b64).as_deref(), Ok(text));
        }
        // sextets 62 and 63 are the only places the alphabets differ
        assert_eq!(encode(Codec::Base64, ">>>???"), "Pj4+Pz8/");
        assert_eq!(encode(Codec::Base64Url, ">>>???"), "Pj4-Pz8_");
        assert_eq!(decode(Codec::Base64, "Pj4+Pz8/").unwrap(), ">>>???");
        assert_eq!(decode(Codec::Base64Url, "Pj4-Pz8_").unwrap(), ">>>???");
        assert!(decode(Codec::Base64, "Pj4-Pz8_").is_err());
        assert!(decode(Codec::Base64Url, "Pj4+Pz8/").is_err());
        // standard Base64 is padded; a short last group is an invalid length
        assert!(decode(Codec::Base64, "Zg").is_err());
        // multi-byte text goes through its UTF-8 bytes
        assert_eq!(encode(Codec::Base64, "\u{fb}\u{ff}"), "w7vDvw==");
        assert_eq!(encode(Codec::Base64Url, "\u{fb}\u{ff}"), "w7vDvw");
    }

    #[test]
    fn base64_decode_ignores_white_space_and_rejects_anything_else() {
        assert_eq!(decode(Codec::Base64, "Zm9v\nYmFy").as_deref(), Ok("foobar"));
        assert_eq!(decode(Codec::Base64, " Zm9v\t\r\n YmFy ").unwrap(), "foobar");
        assert_eq!(decode(Codec::Base64Url, "Zm9v\nYmFy").unwrap(), "foobar");
        for bad in ["Zm9v!", "Zm9", "Zm9vY", "=Zm9v", "Zm=9"] {
            let e = decode(Codec::Base64, bad).unwrap_err();
            assert!(e.starts_with("Invalid Base64"), "{bad}: {e}");
        }
    }

    #[test]
    fn decoded_bytes_must_be_utf8_text() {
        for (codec, data) in [(Codec::Base64, "/w=="), (Codec::Base64Url, "_w"), (Codec::Hex, "ff")] {
            assert_eq!(decode(codec, data), Err("The decoded data is not UTF-8 text".into()));
        }
    }

    #[test]
    fn hex_round_trips_and_rejects_odd_counts_and_non_hex() {
        assert_eq!(encode(Codec::Hex, "abc"), "616263");
        assert_eq!(encode(Codec::Hex, "\u{e9}"), "c3a9");
        assert_eq!(encode(Codec::Hex, ""), "");
        for text in ["", "a", "hello, world\n", "h\u{e9}llo \u{1f600}"] {
            let hex = encode(Codec::Hex, text);
            assert_eq!(decode(Codec::Hex, &hex).as_deref(), Ok(text));
        }
        // upper case and white space are fine
        assert_eq!(decode(Codec::Hex, "C3 A9\n").unwrap(), "\u{e9}");
        assert!(decode(Codec::Hex, "abc").unwrap_err().contains("even"));
        assert!(decode(Codec::Hex, "zz").unwrap_err().contains("'z'"));
        assert!(decode(Codec::Hex, "6\u{e9}").is_err());
    }

    #[test]
    fn uri_component_uses_utf8_percent_bytes_and_strict_decoding() {
        let codec: Codec = serde_json::from_str("\"uri_component\"").unwrap();
        assert_eq!(serde_json::to_string(&codec).unwrap(), "\"uri_component\"");
        for (text, escaped) in [
            ("", ""),
            ("AZaz09-._~", "AZaz09-._~"),
            ("a b/é", "a%20b%2F%C3%A9"),
            ("+%&=?!'()*", "%2B%25%26%3D%3F%21%27%28%29%2A"),
            ("😀\n行\t", "%F0%9F%98%80%0A%E8%A1%8C%09"),
        ] {
            assert_eq!(encode(codec, text), escaped);
            assert_eq!(decode(codec, escaped).as_deref(), Ok(text));
        }
        assert_eq!(decode(codec, " +é\t\n%2b%c3%a9").unwrap(), " +é\t\n+é");
        for invalid in ["%", "%2", "%GG", "%2G", "% G", "%é", "%FF", "%C3", "%C0%AF", "%ED%A0%80"] {
            assert!(decode(codec, invalid).is_err(), "{invalid}");
        }
        for (json, old) in [("base64", Codec::Base64), ("base64_url", Codec::Base64Url), ("hex", Codec::Hex)] {
            assert_eq!(serde_json::from_str::<Codec>(&format!("\"{json}\"")).unwrap(), old);
        }
    }

    #[test]
    fn html_entities_escape_five_characters_and_decode_source_once() {
        let codec: Codec = serde_json::from_str("\"html_entities\"").unwrap();
        assert_eq!(serde_json::to_string(&codec).unwrap(), "\"html_entities\"");
        for (text, escaped) in [
            ("", ""),
            ("&<>\"'", "&amp;&lt;&gt;&quot;&#39;"),
            (" +é😀\t\r\n", " +é😀\t\r\n"),
            ("&amp;lt;", "&amp;amp;lt;"),
        ] {
            assert_eq!(encode(codec, text), escaped);
            assert_eq!(decode(codec, escaped).as_deref(), Ok(text));
        }
        assert_eq!(decode(codec, "&amp;lt; &amp;#65; &#38;lt;").unwrap(), "&lt; &#65; &lt;");
        assert_eq!(decode(codec, "&unknown; &apos; &AMP; &lt & +é\n").unwrap(), "&unknown; &apos; &AMP; &lt & +é\n");
        assert_eq!(decode(codec, "&unknown;&lt; &bad&amp; &#x26;#65;").unwrap(), "&unknown;< &bad& &#65;");
    }

    #[test]
    fn html_entities_numeric_references_are_strict_unicode_scalars() {
        let codec: Codec = serde_json::from_str("\"html_entities\"").unwrap();
        assert_eq!(
            decode(codec, "&#0;&#9;&#13;&#x80;&#00065;&#x000e9;&#X1F600;&#x10FFFF;").unwrap(),
            "\0\t\r\u{80}Aé😀\u{10ffff}"
        );
        for invalid in [
            "&#",
            "&#;",
            "&#x;",
            "&#X;",
            "&#65",
            "&#x41",
            "&#+65;",
            "&#-1;",
            "&# 65;",
            "&#65 ;",
            "&#1a;",
            "&#xGG;",
            "&#x１２;",
            "&#１２;",
            "&#١;",
            "&#x1_0;",
            "&#0x41;",
            "&#x+41;",
            "&#55296;",
            "&#xDFFF;",
            "&#xD800;&#xDC00;",
            "&#1114112;",
            "&#x110000;",
            "&#4294967296;",
            "&#x100000000;",
            "&#999999999999999999999999999999999;",
            "&#65&lt;",
        ] {
            assert!(decode(codec, invalid).is_err(), "accepted malformed numeric entity: {invalid}");
        }
    }

    #[test]
    fn n03_regex_literal_matches_original_in_orlo_engines() {
        use crate::buffer::FindOpts;
        use crate::search::pattern::Pattern;
        let codec: Codec = serde_json::from_str("\"regex_literal\"").expect("named regex literal codec");
        let opts = FindOpts { regex: true, case_sensitive: true, ..FindOpts::default() };
        for text in ["", ".*+?()[]{}^$|\\#&-~", "(?<=x)(a)\\1", "é😀 e\u{301}\n行\t", "a/b!% ' \""] {
            let escaped = encode(codec, text);
            let query = format!(r"\A(?:{escaped})\z");
            let hay = format!("{text}!");
            for pattern in [Pattern::new(&query, &opts).unwrap(), Pattern::fancy(&query, &opts, "").unwrap()] {
                let matches = pattern.find_all(text, &|| false).unwrap();
                assert_eq!(matches.len(), 1, "{query:?}");
                assert_eq!(matches[0], 0..text.len(), "{query:?}");
                assert!(pattern.find_all(&hay, &|| false).unwrap().is_empty(), "literal is anchored exactly");
            }
        }
        assert_eq!(encode(codec, r"a.b[0]+\"), r"a\.b\[0\]\+\\");
        assert!(decode(codec, "abc").is_err(), "no implicit regex unescape dialect");
    }

    #[test]
    fn n03_json_string_roundtrips_and_requires_one_string() {
        let codec: Codec = serde_json::from_str("\"json_string\"").expect("named JSON string codec");
        assert_eq!(serde_json::to_string(&codec).unwrap(), "\"json_string\"");
        for text in ["", "quote \" and \\ and ' café", "line\nnext\r\n\t\u{8}\u{c}\0", "😀 e\u{301} 行"] {
            let quoted = encode(codec, text);
            assert_eq!(serde_json::from_str::<String>(&quoted).unwrap(), text);
            assert_eq!(decode(codec, &quoted).unwrap(), text);
        }
        assert_eq!(decode(codec, " \t\"\\uD83D\\uDE00\"\r\n").unwrap(), "😀");
        for bad in
            ["", "null", "true", "123", "[]", "{}", "'text'", "\"a\" \"b\"", "\"a\n\"", r#""\x41""#, r#""\uD800""#]
        {
            assert!(decode(codec, bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn hashes_match_the_known_vectors() {
        let vectors = [
            (Hash::Md5, "d41d8cd98f00b204e9800998ecf8427e", "900150983cd24fb0d6963f7d28e17f72"),
            (Hash::Sha1, "da39a3ee5e6b4b0d3255bfef95601890afd80709", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (
                Hash::Sha256,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                Hash::Sha512,
                "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
                 47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                 2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
        ];
        for (h, empty, abc) in vectors {
            assert_eq!(digest(h, b""), empty, "{h:?}");
            assert_eq!(digest(h, b"abc"), abc, "{h:?}");
        }
        let names = Hash::ALL.map(Hash::name);
        assert_eq!(names, ["MD5", "SHA-1", "SHA-256", "SHA-512"]);
    }

    #[test]
    fn digest_file_streams_a_binary_file_in_hash_order() {
        let dir = std::env::temp_dir().join(format!("orlo-digest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        // every byte value, past the 64 KiB chunk twice over
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i * 7 + i / 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let got = digest_file(&path).unwrap();
        assert_eq!(got, Hash::ALL.map(|h| digest(h, &bytes)));
        std::fs::write(&path, b"").unwrap();
        assert_eq!(digest_file(&path).unwrap()[0], digest(Hash::Md5, b""));
        let missing = digest_file(&dir.join("missing")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        // a folder is a read error, not an empty file
        assert!(digest_file(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
