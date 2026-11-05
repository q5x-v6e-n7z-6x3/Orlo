//! Explicit scalar warnings. Never classify all non-ASCII text as unusual.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A single overlay glyph and the explanation shown by Character Info.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnicodeWarning {
    #[serde(deserialize_with = "glyph")]
    pub glyph: char,
    pub description: String,
}

fn glyph<'de, D: serde::Deserializer<'de>>(d: D) -> Result<char, D::Error> {
    let ch = char::deserialize(d)?;
    if ch.is_control() || ch.is_whitespace() {
        return Err(serde::de::Error::custom("warning glyph must not be whitespace or a control character"));
    }
    Ok(ch)
}

/// JSON object keys are single Unicode scalars, including JSON Unicode escapes.
pub type WarningCharacters = BTreeMap<char, UnicodeWarning>;

/// Inclusive scalar intervals; deserialization validates keys without expanding their members.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodepointRange {
    start: u32,
    end: u32,
}

impl<'de> Deserialize<'de> for CodepointRange {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let scalar = |s: &str| {
            let hex = s.strip_prefix("U+")?;
            if !(4..=6).contains(&hex.len()) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let n = u32::from_str_radix(hex, 16).ok()?;
            char::from_u32(n).map(|_| n)
        };
        let (a, b) = text.split_once("..").unwrap_or((&text, &text));
        match (scalar(a), scalar(b)) {
            (Some(start), Some(end)) if start <= end && !(start <= 0xDFFF && end >= 0xD800) => Ok(Self { start, end }),
            _ => Err(serde::de::Error::custom(
                "warning range must be U+XXXX or U+XXXX..U+XXXX (4–6 hex digits, ascending Unicode scalars, no surrogates)",
            )),
        }
    }
}

impl Serialize for CodepointRange {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let key = if self.start == self.end {
            format!("U+{:04X}", self.start)
        } else {
            format!("U+{:04X}..U+{:04X}", self.start, self.end)
        };
        s.serialize_str(&key)
    }
}

/// Original settings plus disjoint lookup intervals, compiled once on settings load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WarningRanges {
    entries: BTreeMap<CodepointRange, UnicodeWarning>,
    intervals: Vec<(u32, u32, CodepointRange)>,
}

impl WarningRanges {
    fn compile(entries: BTreeMap<CodepointRange, UnicodeWarning>) -> Self {
        let mut events: BTreeMap<u32, Vec<(CodepointRange, bool)>> = BTreeMap::new();
        for range in entries.keys() {
            events.entry(range.start).or_default().push((*range, true));
            events.entry(range.end + 1).or_default().push((*range, false));
        }
        let mut active = BTreeSet::new();
        let mut intervals = Vec::new();
        let mut previous = 0;
        for (at, changes) in events {
            if previous < at {
                if let Some(&range) = active.last() {
                    intervals.push((previous, at - 1, range));
                }
            }
            for (range, starts) in changes {
                if starts {
                    active.insert(range);
                } else {
                    active.remove(&range);
                }
            }
            previous = at;
        }
        Self { entries, intervals }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.intervals.clear();
    }

    fn get(&self, ch: char) -> Option<&UnicodeWarning> {
        let code = ch as u32;
        let i = self.intervals.partition_point(|&(start, _, _)| start <= code).checked_sub(1)?;
        let (_, end, key) = self.intervals[i];
        (code <= end).then(|| &self.entries[&key])
    }
}

impl<'de> Deserialize<'de> for WarningRanges {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct RangesVisitor;

        impl<'de> serde::de::Visitor<'de> for RangesVisitor {
            type Value = WarningRanges;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object of distinct Unicode warning ranges")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut entries = BTreeMap::new();
                while let Some((range, warning)) = map.next_entry::<CodepointRange, UnicodeWarning>()? {
                    if entries.insert(range, warning).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate warning range U+{:04X}..U+{:04X}",
                            range.start, range.end
                        )));
                    }
                }
                Ok(WarningRanges::compile(entries))
            }
        }

        d.deserialize_map(RangesVisitor)
    }
}

impl Serialize for WarningRanges {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.entries.serialize(s)
    }
}

/// Scalar entries win; ranges prefer greatest start, then greatest end, independent of JSON order.
/// Paint does two logarithmic lookups, without allocation or interval/member enumeration.
pub fn warning_for<'a>(
    ch: char,
    characters: &'a WarningCharacters,
    ranges: &'a WarningRanges,
) -> Option<&'a UnicodeWarning> {
    characters.get(&ch).or_else(|| ranges.get(ch))
}

pub fn default_characters() -> WarningCharacters {
    [
        ('\0', "Null character"),
        ('\u{00a0}', "Non-breaking space"),
        ('\u{00ad}', "Soft hyphen"),
        ('\u{061c}', "Arabic letter mark"),
        ('\u{200b}', "Zero width space"),
        ('\u{200e}', "Left-to-right mark"),
        ('\u{200f}', "Right-to-left mark"),
        ('\u{2028}', "Line separator"),
        ('\u{2029}', "Paragraph separator"),
        ('\u{202a}', "Left-to-right embedding"),
        ('\u{202b}', "Right-to-left embedding"),
        ('\u{202c}', "Pop directional formatting"),
        ('\u{202d}', "Left-to-right override"),
        ('\u{202e}', "Right-to-left override"),
        ('\u{202f}', "Narrow non-breaking space"),
        ('\u{2060}', "Word joiner"),
        ('\u{2066}', "Left-to-right isolate"),
        ('\u{2067}', "Right-to-left isolate"),
        ('\u{2068}', "First strong isolate"),
        ('\u{2069}', "Pop directional isolate"),
        ('\u{feff}', "Embedded byte order mark / zero width non-breaking space"),
    ]
    .into_iter()
    .map(|(ch, description)| (ch, UnicodeWarning { glyph: '!', description: description.into() }))
    .collect()
}

/// Inspect the actual cluster, even when overlay painting is disabled.
pub fn describe(cluster: &str, characters: &WarningCharacters, enabled: bool) -> String {
    describe_with_ranges(cluster, characters, &WarningRanges::default(), enabled)
}

pub fn describe_with_ranges(
    cluster: &str,
    characters: &WarningCharacters,
    ranges: &WarningRanges,
    enabled: bool,
) -> String {
    let mut out = String::new();
    for ch in cluster.chars() {
        if let Some(warning) = warning_for(ch, characters, ranges) {
            use std::fmt::Write;
            let state = if enabled { "" } else { " (marks disabled)" };
            let _ = write!(out, "\nUnicode warning{state}: U+{:04X} — {}", ch as u32, warning.description);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_warning_ranges_reject_equivalent_keys_in_either_order() {
        for alias in ["U+00e000", "U+E000..U+E000", "U+E000"] {
            for reverse in [false, true] {
                let first = r#""U+E000":{"glyph":"!","description":"first"}"#;
                let second = format!(r#""{alias}":{{"glyph":"?","description":"second"}}"#);
                let json = if reverse { format!("{{{second},{first}}}") } else { format!("{{{first},{second}}}") };
                let error = serde_json::from_str::<WarningRanges>(&json).expect_err("duplicate numeric interval");
                assert!(error.to_string().contains("duplicate warning range"), "{error}");
            }
        }
        let distinct =
            r#"{"U+E000":{"glyph":"!","description":"first"},"U+E001":{"glyph":"?","description":"second"}}"#;
        let ranges: WarningRanges = serde_json::from_str(distinct).unwrap();
        assert_eq!(serde_json::from_str::<WarningRanges>(&serde_json::to_string(&ranges).unwrap()).unwrap(), ranges);
    }

    #[test]
    fn ranges_include_endpoints_skip_gaps_and_prefer_numeric_bounds_then_scalars() {
        let ranges: WarningRanges = serde_json::from_str(
            r#"{
            "U+E000..U+E010":{"glyph":"1","description":"outer"},
            "U+E004..U+E006":{"glyph":"2","description":"short"},
            "U+00E004..U+00e008":{"glyph":"3","description":"long"},
            "U+E005":{"glyph":"4","description":"singleton"},
            "U+10FFFF":{"glyph":"5","description":"last"},
            "U+0000":{"glyph":"6","description":"first"}
        }"#,
        )
        .unwrap();
        let mut characters = WarningCharacters::new();
        for (code, glyph) in [
            (0, '6'),
            (0xE000, '1'),
            (0xE003, '1'),
            (0xE004, '3'),
            (0xE005, '4'),
            (0xE006, '3'),
            (0xE008, '3'),
            (0xE009, '1'),
            (0xE010, '1'),
            (0x10FFFF, '5'),
        ] {
            assert_eq!(warning_for(char::from_u32(code).unwrap(), &characters, &ranges).unwrap().glyph, glyph);
        }
        for code in [1, 0xD7FF, 0xE011, 0xFFFF, 0x10FFFE] {
            assert!(warning_for(char::from_u32(code).unwrap(), &characters, &ranges).is_none());
        }
        characters.insert('\u{e005}', UnicodeWarning { glyph: '!', description: "explicit".into() });
        assert_eq!(warning_for('\u{e005}', &characters, &ranges).unwrap().glyph, '!');
        assert!(describe_with_ranges("\u{e005}", &characters, &ranges, false).contains("U+E005 — explicit"));
        let encoded = serde_json::to_string(&ranges).unwrap();
        assert_eq!(serde_json::from_str::<WarningRanges>(&encoded).unwrap(), ranges);
    }

    #[test]
    fn many_overlapping_ranges_are_not_enumerated_into_scalars() {
        let ranges = WarningRanges::compile(
            (0..4096)
                .map(|start| {
                    (
                        CodepointRange { start: 0xE000 + start, end: 0x10FFFF },
                        UnicodeWarning { glyph: '!', description: start.to_string() },
                    )
                })
                .collect(),
        );
        assert_eq!(ranges.len(), 4096);
        assert!(ranges.intervals.len() < 2 * ranges.len());
        let characters = WarningCharacters::new();
        for _ in 0..100 {
            assert_eq!(warning_for('\u{10ffff}', &characters, &ranges).unwrap().description, "4095");
            assert!(warning_for('x', &characters, &ranges).is_none());
        }
        assert!(warning_for('\u{d7ff}', &characters, &ranges).is_none());
    }
    #[test]
    fn compiled_ranges_match_precedence_across_expired_intervals_and_gaps() {
        let mut entries: BTreeMap<_, _> = (0xE001..0xF001)
            .map(|at| {
                (CodepointRange { start: at, end: at }, UnicodeWarning { glyph: '!', description: at.to_string() })
            })
            .collect();
        for live in [false, true] {
            if live {
                entries.insert(
                    CodepointRange { start: 0xE000, end: 0x10FFFF },
                    UnicodeWarning { glyph: '?', description: "lower live interval".into() },
                );
            }
            let ranges = WarningRanges::compile(entries.clone());
            assert!(ranges.intervals.len() <= 2 * entries.len());
            for code in [0, 0xD7FF, 0xE000, 0xE001, 0xE123, 0xEFFF, 0xF000, 0xF001, 0x10FFFF] {
                let expected = entries.iter().rev().find(|(r, _)| r.start <= code && code <= r.end).map(|(_, w)| w);
                assert_eq!(ranges.get(char::from_u32(code).unwrap()), expected);
            }
            let encoded = serde_json::to_value(&ranges).unwrap();
            assert_eq!(encoded.as_object().unwrap().len(), entries.len());
            assert_eq!(serde_json::from_value::<WarningRanges>(encoded).unwrap(), ranges);
        }
    }
}
