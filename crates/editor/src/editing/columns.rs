//! Column editing: display columns and the Column Editor's number sequences.

use crate::viewport::is_wide;
use unicode_segmentation::UnicodeSegmentation;

/// The most digits a number is padded to, so one typo cannot ask for gigabytes.
pub const MAX_WIDTH: usize = 1000;

/// The display column after the first `char_idx` chars of `line` (all of it when it is shorter):
/// a tab advances to the next tab stop, a wide or emoji cluster takes 2, and marks, joiners and
/// variation selectors take 0.
pub fn display_col(line: &str, char_idx: usize, tab_size: usize) -> usize {
    let end = line.char_indices().nth(char_idx).map_or(line.len(), |(b, _)| b);
    display_col_from(&line[..end], 0, tab_size)
}

fn display_col_from(line: &str, start: usize, tab_size: usize) -> usize {
    line.graphemes(true).fold(start, |col, g| {
        let first = g.chars().next().unwrap_or(' ');
        col + match first {
            '\t' => tab_size - col % tab_size,
            // marks that start a cluster (line start) count 1; only the joiner, the
            // variation selectors and invisible spaces are listed
            '\u{200B}'..='\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}' => 0,
            c if is_wide(c) => 2,
            _ => 1,
        }
    })
}

/// The Column Editor's numbers: value `i` is `initial + step * (i / repeat)`.
pub struct Sequence {
    pub initial: i64,
    pub step: i64,
    pub repeat: u32,
    pub base: u32,
    pub upper: bool,
    /// Least digits; the sign comes before the padding zeros (`-007`).
    pub width: usize,
    pub zero_pad: bool,
}

/// The first `n` values of `seq`, written in its base and padding.
pub fn sequence(seq: &Sequence, n: usize) -> Result<Vec<String>, String> {
    if seq.repeat == 0 {
        return Err("Repeat must be at least 1".into());
    }
    if !matches!(seq.base, 2 | 8 | 10 | 16) {
        return Err("Base must be 10, 16, 8 or 2".into());
    }
    if seq.width > MAX_WIDTH {
        return Err(format!("Pad width must be at most {MAX_WIDTH}"));
    }
    (0..n)
        .map(|i| {
            let steps = (i / seq.repeat as usize) as i64;
            let value = (seq.step.checked_mul(steps))
                .and_then(|d| seq.initial.checked_add(d))
                .ok_or("The numbers grow beyond 64 bits")?;
            Ok(format_number(value, seq))
        })
        .collect()
}

fn format_number(value: i64, seq: &Sequence) -> String {
    let magnitude = value.unsigned_abs();
    let digits = match (seq.base, seq.upper) {
        (16, true) => format!("{magnitude:X}"),
        (16, false) => format!("{magnitude:x}"),
        (8, _) => format!("{magnitude:o}"),
        (2, _) => format!("{magnitude:b}"),
        _ => magnitude.to_string(),
    };
    let sign = if value < 0 { "-" } else { "" };
    let pad = seq.width.saturating_sub(digits.len());
    match seq.zero_pad {
        true => format!("{sign}{}{digits}", "0".repeat(pad)),
        false => format!("{}{sign}{digits}", " ".repeat(pad)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(initial: i64, step: i64, repeat: u32, base: u32) -> Sequence {
        Sequence { initial, step, repeat, base, upper: false, width: 0, zero_pad: false }
    }

    fn run(s: &Sequence, n: usize) -> Vec<String> {
        sequence(s, n).unwrap()
    }

    #[test]
    fn a_sequence_counts_from_the_initial_by_the_step() {
        assert_eq!(run(&seq(1, 1, 1, 10), 4), ["1", "2", "3", "4"]);
        assert_eq!(run(&seq(10, -3, 1, 10), 3), ["10", "7", "4"]);
        assert_eq!(run(&seq(-2, 1, 1, 10), 4), ["-2", "-1", "0", "1"]);
        assert!(run(&seq(1, 1, 1, 10), 0).is_empty());
    }

    #[test]
    fn each_value_repeats_before_the_step() {
        assert_eq!(run(&seq(0, 5, 2, 10), 4), ["0", "0", "5", "5"]);
        assert_eq!(run(&seq(1, 1, 3, 10), 7), ["1", "1", "1", "2", "2", "2", "3"]);
    }

    #[test]
    fn hex_octal_and_binary_with_case_and_padding() {
        let hex = Sequence { upper: true, width: 4, zero_pad: true, ..seq(250, 1, 1, 16) };
        assert_eq!(run(&hex, 8), ["00FA", "00FB", "00FC", "00FD", "00FE", "00FF", "0100", "0101"]);
        let lower = Sequence { width: 2, zero_pad: true, ..seq(10, 1, 1, 16) };
        assert_eq!(run(&lower, 2), ["0a", "0b"]);
        assert_eq!(run(&seq(7, 1, 1, 8), 3), ["7", "10", "11"]);
        let bin = Sequence { width: 4, zero_pad: true, ..seq(5, 1, 1, 2) };
        assert_eq!(run(&bin, 3), ["0101", "0110", "0111"]);
    }

    #[test]
    fn spaces_pad_the_digits_and_the_sign_stays_beside_them() {
        let spaced = Sequence { width: 3, ..seq(8, 1, 1, 10) };
        assert_eq!(run(&spaced, 3), ["  8", "  9", " 10"]);
        let signed = Sequence { width: 3, ..seq(-1, 1, 1, 10) };
        assert_eq!(run(&signed, 3), ["  -1", "  0", "  1"]);
    }

    #[test]
    fn a_negative_number_keeps_its_sign_before_the_zeros() {
        let s = Sequence { width: 3, zero_pad: true, ..seq(-7, 1, 1, 10) };
        assert_eq!(run(&s, 2), ["-007", "-006"]);
        // base 16 writes the magnitude, never two's complement
        let h = Sequence { width: 2, zero_pad: true, ..seq(-255, 1, 1, 16) };
        assert_eq!(run(&h, 1), ["-ff"]);
        assert_eq!(run(&seq(i64::MIN, 1, 1, 10), 1), ["-9223372036854775808"]);
    }

    #[test]
    fn a_sequence_that_leaves_64_bits_is_an_error() {
        assert!(sequence(&seq(i64::MAX - 1, 1, 1, 10), 3).is_err());
        assert!(sequence(&seq(0, i64::MAX, 1, 10), 3).is_err());
        assert!(sequence(&seq(i64::MIN, -1, 1, 10), 2).is_err());
        // as far as it goes is fine
        assert_eq!(run(&seq(i64::MAX - 1, 1, 1, 10), 2).len(), 2);
    }

    #[test]
    fn nonsense_settings_are_errors() {
        assert!(sequence(&seq(1, 1, 0, 10), 2).is_err(), "repeat 0");
        assert!(sequence(&seq(1, 1, 1, 7), 2).is_err(), "base 7");
        let wide = Sequence { width: MAX_WIDTH + 1, ..seq(1, 1, 1, 10) };
        assert!(sequence(&wide, 2).is_err());
    }

    #[test]
    fn display_columns_follow_tab_stops_wide_clusters_and_marks() {
        assert_eq!(display_col("\tab", 1, 4), 4);
        assert_eq!(display_col("\tab", 2, 4), 5);
        assert_eq!(display_col("ab\tc", 3, 4), 4);
        assert_eq!(display_col("abcd\te", 5, 8), 8);
        assert_eq!(display_col("日本", 0, 4), 0);
        assert_eq!(display_col("日本", 1, 4), 2);
        assert_eq!(display_col("日本", 2, 4), 4);
        // a decomposed letter is one column, the mark none
        assert_eq!(display_col("e\u{301}x", 2, 4), 1);
        assert_eq!(display_col("e\u{301}x", 3, 4), 2);
        // a family emoji is one cluster of two columns
        let family = "👨\u{200D}👩\u{200D}👧x";
        assert_eq!(display_col(family, family.chars().count() - 1, 4), 2);
        // a variation selector and a lone joiner are none
        assert_eq!(display_col("a\u{FE0F}b\u{200D}c", 5, 4), 3);
        // past the end is the end
        assert_eq!(display_col("ab", 99, 4), 2);
    }
}

/// Signed adjustment serialized as a decimal string: JSON's number intermediary is only 64-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueStep(pub i128);

impl Default for ValueStep {
    fn default() -> Self {
        Self(1)
    }
}
impl serde::Serialize for ValueStep {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> serde::Deserialize<'de> for ValueStep {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(deserializer)?;
        text.parse::<i128>().map(Self).map_err(serde::de::Error::custom)
    }
}

/// Adjust one complete integer, ISO date, time or date-time, preserving its admitted format.
pub fn adjust_value(text: &str, step: ValueStep) -> Result<String, String> {
    let result = if text.as_bytes().get(4) == Some(&b'-') {
        adjust_calendar(text, step.0)
    } else if text.as_bytes().get(2) == Some(&b':') {
        let minutes = time_minutes(text)?;
        let minute = (minutes + step.0.rem_euclid(1440)).rem_euclid(1440);
        Ok(format!("{:02}:{:02}{}", minute / 60, minute % 60, &text[5..]))
    } else {
        adjust_integer(text, step.0)
    }?;
    Ok(if step.0 == 0 { text.to_string() } else { result })
}

fn adjust_integer(text: &str, step: i128) -> Result<String, String> {
    let invalid = || "Select a whole decimal, 0x hex, 0b binary, 0o octal integer or supported ISO value".to_string();
    let (negative, plus, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, false, &text[1..]),
        Some(b'+') => (false, true, &text[1..]),
        _ => (false, false, text),
    };
    let (base, prefix, digits) = match unsigned.as_bytes().get(..2) {
        Some(b"0x" | b"0X") => (16, &unsigned[..2], &unsigned[2..]),
        Some(b"0b" | b"0B") => (2, &unsigned[..2], &unsigned[2..]),
        Some(b"0o" | b"0O") => (8, &unsigned[..2], &unsigned[2..]),
        _ => (10, "", unsigned),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return Err(invalid());
    }
    let letters: Vec<_> = digits.bytes().filter(|b| *b != b'_').collect();
    let default_upper = letters
        .iter()
        .find(|b| b.is_ascii_alphabetic())
        .map_or_else(|| prefix.bytes().any(|b| b.is_ascii_uppercase()), |b| b.is_ascii_uppercase());
    let mut value = 0i128;
    let mut width: usize = 0;
    let mut boundaries = Vec::new();
    for c in digits.chars().rev() {
        if c == '_' {
            boundaries.push(width);
            continue;
        }
        if !c.is_ascii() || c.to_digit(base).is_none() {
            return Err(invalid());
        }
        width += 1;
    }
    // Accumulate with the source sign so MIN's magnitude never needs a positive i128.
    for c in digits.chars().filter(|c| *c != '_') {
        let digit = i128::from(c.to_digit(base).ok_or_else(invalid)?);
        value = value
            .checked_mul(i128::from(base))
            .and_then(|v| if negative { v.checked_sub(digit) } else { v.checked_add(digit) })
            .ok_or("The selected integer exceeds signed 128 bits")?;
    }
    let value = value.checked_add(step).ok_or("The adjustment exceeds signed 128 bits")?;
    let magnitude = value.unsigned_abs();
    let raw = match base {
        16 => format!("{magnitude:x}"),
        8 => format!("{magnitude:o}"),
        2 => format!("{magnitude:b}"),
        _ => magnitude.to_string(),
    };
    let raw = format!("{}{}", "0".repeat(width.saturating_sub(raw.len())), raw);
    let mut out = String::new();
    if value < 0 {
        out.push('-');
    } else if plus {
        out.push('+');
    }
    out.push_str(prefix);
    let mut boundaries = boundaries.iter().rev().peekable();
    for (i, mut c) in raw.chars().enumerate() {
        let from_right = raw.len() - i;
        if boundaries.peek().is_some_and(|b| **b == from_right) {
            out.push('_');
            boundaries.next();
        }
        if c.is_ascii_alphabetic() {
            let upper = letters
                .len()
                .checked_sub(from_right)
                .and_then(|at| letters.get(at))
                .filter(|b| b.is_ascii_alphabetic())
                .map_or(default_upper, |b| b.is_ascii_uppercase());
            if upper {
                c = c.to_ascii_uppercase();
            }
        }
        out.push(c);
    }
    Ok(out)
}

fn decimal(text: &str) -> Result<i128, String> {
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid ISO value".into());
    }
    text.parse().map_err(|_| "Invalid ISO value".into())
}

fn time_minutes(text: &str) -> Result<i128, String> {
    if !text.is_ascii()
        || !matches!(text.len(), 5 | 8)
        || text.as_bytes()[2] != b':'
        || (text.len() == 8 && text.as_bytes()[5] != b':')
    {
        return Err("Use HH:MM or HH:MM:SS".into());
    }
    let (hour, minute) = (decimal(&text[..2])?, decimal(&text[3..5])?);
    if hour > 23 || minute > 59 || (text.len() == 8 && decimal(&text[6..])? > 59) {
        return Err("ISO time is out of range".into());
    }
    Ok(hour * 60 + minute)
}

fn year_days(year: i128) -> i128 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}
fn month_days(year: i128, month: i128) -> i128 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
fn iso_day(text: &str) -> Result<i128, String> {
    if text.len() != 10 || !text.is_ascii() || text.as_bytes()[4] != b'-' || text.as_bytes()[7] != b'-' {
        return Err("Use YYYY-MM-DD (years 0001 through 9999)".into());
    }
    let (year, month, day) = (decimal(&text[..4])?, decimal(&text[5..7])?, decimal(&text[8..])?);
    if year == 0 || !(1..=12).contains(&month) || day == 0 || day > month_days(year, month) {
        return Err("ISO date is out of range".into());
    }
    Ok(year_days(year) + (1..month).map(|m| month_days(year, m)).sum::<i128>() + day - 1)
}
fn format_day(day: i128) -> String {
    let (mut low, mut high) = (1, 10000);
    while low + 1 < high {
        let middle = (low + high) / 2;
        if year_days(middle) <= day {
            low = middle;
        } else {
            high = middle;
        }
    }
    let (year, mut day, mut month) = (low, day - year_days(low), 1);
    while day >= month_days(year, month) {
        day -= month_days(year, month);
        month += 1;
    }
    format!("{year:04}-{month:02}-{:02}", day + 1)
}
fn adjust_calendar(text: &str, step: i128) -> Result<String, String> {
    if !text.is_ascii() || text.len() < 10 {
        return Err("Invalid ISO date".into());
    }
    let day = iso_day(&text[..10])?;
    let overflow = "ISO adjustment leaves years 0001 through 9999";
    if text.len() == 10 {
        let day = day.checked_add(step).filter(|d| (0..year_days(10000)).contains(d)).ok_or(overflow)?;
        return Ok(format_day(day));
    }
    if text.as_bytes()[10] != b'T' {
        return Err("Use uppercase T between ISO date and time".into());
    }
    let rest = &text[11..];
    let zone_at = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(zone_at);
    if !zone.is_empty()
        && zone != "Z"
        && (zone.len() != 6
            || !matches!(zone.as_bytes()[0], b'+' | b'-')
            || zone.as_bytes()[3] != b':'
            || decimal(&zone[1..3])? > 23
            || decimal(&zone[4..])? > 59)
    {
        return Err("Use Z or an ISO offset ±HH:MM".into());
    }
    let minute = (day * 1440 + time_minutes(time)?)
        .checked_add(step)
        .filter(|m| (0..year_days(10000) * 1440).contains(m))
        .ok_or(overflow)?;
    Ok(format!("{}T{:02}:{:02}{}{}", format_day(minute / 1440), minute % 1440 / 60, minute % 60, &time[5..], zone))
}

#[cfg(test)]
mod n09_tests {
    use super::*;
    #[test]
    fn n09_mixed_hex_case_follows_aligned_letters_and_first_letter_for_carries() {
        for (text, step, want) in [
            ("0xaB", 1, "0xaC"),
            ("0xAf", 1, "0xB0"),
            ("0x9F", 16, "0xAF"),
            ("-0x00aB", 1, "-0x00aA"),
            ("0xfFf", 1, "0x1000"),
            ("+0X0a_Bf", 1, "+0X0a_C0"),
            ("0xaB", 0, "0xaB"),
        ] {
            assert_eq!(adjust_value(text, ValueStep(step)).unwrap(), want, "{text} + {step}");
        }
    }
}

#[cfg(test)]
mod n09_boundary_tests {
    use super::*;

    #[test]
    fn n09_integer_radix_sign_width_grouping_and_zero_step() {
        for (text, step, want) in [
            ("009", 1, "010"),
            ("0x0f", 1, "0x10"),
            ("0X09", 1, "0X0A"),
            ("0b0011", -1, "0b0010"),
            ("0B0111", 1, "0B1000"),
            ("0o007", 1, "0o010"),
            ("0O010", -1, "0O007"),
            ("-009", 10, "001"),
            ("+001", -2, "-001"),
            ("+001", -1, "+000"),
            ("-000", 0, "-000"),
            ("-000", 1, "001"),
            ("000", -1, "-001"),
            ("9_999", 1, "10_000"),
            ("00_9_99", 1, "01_0_00"),
            ("0x0_fF", 1, "0x1_00"),
            ("-0b00_01", 1, "0b00_00"),
            ("+0o007", 1, "+0o010"),
        ] {
            assert_eq!(adjust_value(text, ValueStep(step)).unwrap(), want, "{text} + {step}");
        }
        for bad in
            ["", "+", "_1", "1_", "1__2", "0x_1", "0b2", "0o8", "0x", "1.0", "1e2", " 9", "9\n", "９", "é9", "0b-1"]
        {
            assert!(adjust_value(bad, ValueStep(0)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn n09_all_radices_admit_signed_min_max_and_reject_overflow() {
        let sources = [
            (i128::MIN.to_string(), i128::MAX.to_string()),
            (format!("-0x{:x}", 1u128 << 127), format!("0x{:x}", i128::MAX)),
            (format!("-0b1{}", "0".repeat(127)), format!("0b{}", "1".repeat(127))),
            (format!("-0o{:o}", 1u128 << 127), format!("0o{:o}", i128::MAX)),
        ];
        for (min, max) in sources {
            assert_eq!(adjust_value(&min, ValueStep(0)).unwrap(), min);
            assert_eq!(adjust_value(&max, ValueStep(0)).unwrap(), max);
            assert!(adjust_value(&min, ValueStep(-1)).is_err());
            assert!(adjust_value(&max, ValueStep(1)).is_err());
            assert!(
                adjust_value(min.strip_prefix('-').unwrap(), ValueStep(-1)).is_err(),
                "invalid source cannot be rescued by step"
            );
        }
        assert_eq!(adjust_value("0", ValueStep(i128::MIN)).unwrap(), i128::MIN.to_string());
        assert_eq!(adjust_value("0", ValueStep(i128::MAX)).unwrap(), i128::MAX.to_string());
        assert_eq!(adjust_value(&i128::MIN.to_string(), ValueStep(i128::MAX)).unwrap(), format!("-{:039}", 1));
        assert_eq!(adjust_value(&i128::MAX.to_string(), ValueStep(i128::MIN)).unwrap(), format!("-{:039}", 1));
    }

    #[test]
    fn n09_steps_serde_json_value_roundtrips_losslessly_and_rejects_numbers() {
        for value in [i128::MIN, i128::MAX, -1, 0, 1, i64::MAX as i128 + 1] {
            let serialized = serde_json::to_value(ValueStep(value)).unwrap();
            assert_eq!(serialized, serde_json::Value::String(value.to_string()));
            assert_eq!(serde_json::from_value::<ValueStep>(serialized).unwrap(), ValueStep(value));
        }
        for bad in ["1", "1.5", "null", "\"1.0\"", "\" 1\"", "\"170141183460469231731687303715884105728\""] {
            assert!(serde_json::from_str::<ValueStep>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn n09_gregorian_days_validate_centuries_and_both_calendar_edges() {
        for (text, step, want) in [
            ("2000-02-28", 1, "2000-02-29"),
            ("2000-03-01", -1, "2000-02-29"),
            ("1900-02-28", 1, "1900-03-01"),
            ("2100-03-01", -1, "2100-02-28"),
            ("2024-01-31", 1, "2024-02-01"),
            ("2024-01-01", -1, "2023-12-31"),
            ("0001-01-01", 3652058, "9999-12-31"),
            ("9999-12-31", -3652058, "0001-01-01"),
        ] {
            assert_eq!(adjust_value(text, ValueStep(step)).unwrap(), want);
        }
        for (text, step) in
            [("0001-01-01", -1), ("9999-12-31", 1), ("2000-01-01", i128::MAX), ("2000-01-01", i128::MIN)]
        {
            assert!(adjust_value(text, ValueStep(step)).is_err());
        }
        for bad in [
            "0000-01-01",
            "1900-02-29",
            "2100-02-29",
            "2024-04-31",
            "2024-13-01",
            "2024-00-01",
            "2024-01-00",
            "2024-1-01",
            "2024-01-01 ",
        ] {
            assert!(adjust_value(bad, ValueStep(0)).is_err(), "{bad}");
        }
    }

    #[test]
    fn n09_minutes_wrap_times_and_preserve_seconds_zone_and_datetime_limits() {
        for (text, step, want) in [
            ("23:59:37", 1, "00:00:37"),
            ("00:00", -1, "23:59"),
            ("12:30:00", 1440, "12:30:00"),
            ("00:00", i128::MAX, "02:07"),
            ("00:00", i128::MIN, "21:52"),
            ("1999-12-31T23:59:17Z", 1, "2000-01-01T00:00:17Z"),
            ("2000-03-01T00:00+05:30", -1, "2000-02-29T23:59+05:30"),
            ("1900-03-01T00:00:59-00:00", -1, "1900-02-28T23:59:59-00:00"),
            ("2024-12-31T23:59-23:59", 1, "2025-01-01T00:00-23:59"),
            ("2024-01-01T00:00", -1, "2023-12-31T23:59"),
        ] {
            assert_eq!(adjust_value(text, ValueStep(step)).unwrap(), want, "{text} + {step}");
        }
        for (text, step) in [
            ("0001-01-01T00:00Z", -1),
            ("9999-12-31T23:59:59+00:00", 1),
            ("2024-01-01T00:00", i128::MAX),
            ("2024-01-01T00:00", i128::MIN),
        ] {
            assert!(adjust_value(text, ValueStep(step)).is_err());
        }
        for bad in [
            "24:00",
            "23:60",
            "12:00:60",
            "12:00:01.0",
            "12:00Z",
            "2024-01-01t12:00",
            "2024-01-01 12:00",
            "2024-01-01T12:00z",
            "2024-01-01T12:00+24:00",
            "2024-01-01T12:00+01:60",
            "2024-01-01T12:00+0100",
            "2024-01-01T12:00:01.2Z",
        ] {
            assert!(adjust_value(bad, ValueStep(1)).is_err(), "{bad}");
        }
    }
}

/// Padding at each ordinal field anchor, after earlier columns have shifted the tab stops.
/// Missing captures retain their slot. Callers validate all original grapheme boundaries first.
pub(crate) fn alignment_padding(rows: &[(String, Vec<Option<usize>>)], tab_size: usize) -> Vec<Vec<usize>> {
    let mut fields = vec![Vec::new(); rows.iter().map(|(_, fields)| fields.len()).max().unwrap_or(0)];
    for (row, (text, anchors)) in rows.iter().enumerate() {
        let bytes: Vec<_> = text.char_indices().map(|(b, _)| b).chain(std::iter::once(text.len())).collect();
        for (field, at) in anchors.iter().enumerate() {
            if let Some(at) = at {
                fields[field].push((row, bytes[*at]));
            }
        }
    }
    let mut previous = vec![0; rows.len()];
    let mut columns = vec![0; rows.len()];
    let mut pads: Vec<_> = rows.iter().map(|(_, fields)| vec![0; fields.len()]).collect();
    for (field, anchors) in fields.into_iter().enumerate() {
        let mut target = 0;
        for &(row, at) in &anchors {
            columns[row] = display_col_from(&rows[row].0[previous[row]..at], columns[row], tab_size);
            previous[row] = at;
            target = target.max(columns[row]);
        }
        for (row, _) in anchors {
            pads[row][field] = target - columns[row];
            columns[row] = target;
        }
    }
    pads
}
