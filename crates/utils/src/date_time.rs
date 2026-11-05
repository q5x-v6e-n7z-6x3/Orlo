//! Safe wall-clock values and the small, locale-independent timestamp format used by editing.

use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%:z";

#[derive(Clone, Copy, Debug)]
pub struct DateTime {
    seconds: i64,
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    offset: i32,
}

impl DateTime {
    fn native(seconds: i64, local: bool) -> Result<Self, String> {
        let input: libc::time_t = seconds;
        let mut result = std::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: native conversion initializes the correctly sized tm before it is read.
        let value = unsafe {
            let output = if local {
                libc::localtime_r(&input, result.as_mut_ptr())
            } else {
                libc::gmtime_r(&input, result.as_mut_ptr())
            };
            if output.is_null() {
                return Err("Timestamp is out of range".into());
            }
            result.assume_init()
        };
        Ok(Self {
            seconds,
            year: value.tm_year.checked_add(1900).ok_or("Timestamp year is out of range")?,
            month: value.tm_mon + 1,
            day: value.tm_mday,
            hour: value.tm_hour,
            minute: value.tm_min,
            second: value.tm_sec,
            offset: value.tm_gmtoff.try_into().map_err(|_| "Timestamp offset is out of range")?,
        })
    }

    pub fn local(seconds: i64) -> Result<Self, String> {
        Self::native(seconds, true)
    }

    pub fn utc(seconds: i64) -> Result<Self, String> {
        Self::native(seconds, false)
    }

    /// A supplied zone offset, also allowing deterministic conversion without changing process TZ.
    pub fn with_offset(seconds: i64, offset: i32) -> Result<Self, String> {
        let shifted = seconds.checked_add(i64::from(offset)).ok_or("Timestamp is out of range")?;
        Ok(Self { seconds, offset, ..Self::utc(shifted)? })
    }

    pub fn format(&self, format: &str) -> Result<String, String> {
        render(format, |code| match code {
            'Y' => format!("{:04}", self.year),
            'm' => format!("{:02}", self.month),
            'd' => format!("{:02}", self.day),
            'H' => format!("{:02}", self.hour),
            'M' => format!("{:02}", self.minute),
            'S' => format!("{:02}", self.second),
            '%' => "%".into(),
            'z' | ':' => {
                let minutes = self.offset.unsigned_abs() / 60;
                let sign = if self.offset < 0 { '-' } else { '+' };
                let separator = if code == ':' { ":" } else { "" };
                format!("{sign}{:02}{separator}{:02}", minutes / 60, minutes % 60)
            }
            _ => unreachable!(),
        })
    }

    pub fn variable(&self, name: &str) -> Option<String> {
        let directive = match name {
            "CURRENT_YEAR" => 'Y',
            "CURRENT_MONTH" => 'm',
            "CURRENT_DATE" => 'd',
            "CURRENT_HOUR" => 'H',
            "CURRENT_MINUTE" => 'M',
            "CURRENT_SECOND" => 'S',
            "CURRENT_SECONDS_UNIX" => return Some(self.seconds.to_string()),
            _ => return None,
        };
        self.format(&format!("%{directive}")).ok()
    }
}

/// One instant, converted into local time and UTC once for an entire command.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub local: DateTime,
    pub utc: DateTime,
}

impl Snapshot {
    pub fn now() -> Result<Self, String> {
        Self::capture(SystemTime::now, DateTime::local)
    }

    pub fn capture(
        clock: impl FnOnce() -> SystemTime,
        local: impl FnOnce(i64) -> Result<DateTime, String>,
    ) -> Result<Self, String> {
        let seconds = match clock().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).map_err(|_| "Timestamp is out of range")?,
            Err(e) => {
                let d = e.duration();
                let seconds = i64::try_from(d.as_secs()).map_err(|_| "Timestamp is out of range")?;
                seconds
                    .checked_neg()
                    .and_then(|s| s.checked_sub(i64::from(d.subsec_nanos() != 0)))
                    .ok_or("Timestamp is out of range")?
            }
        };
        Ok(Self { local: local(seconds)?, utc: DateTime::utc(seconds)? })
    }
}

pub fn validate_format(format: &str) -> Result<(), String> {
    render(format, |_| String::new()).map(|_| ())
}

fn render(format: &str, mut value: impl FnMut(char) -> String) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            output.push(c);
            continue;
        }
        let code = match chars.next() {
            Some(c @ ('Y' | 'm' | 'd' | 'H' | 'M' | 'S' | 'z' | '%')) => c,
            Some(':') if chars.next() == Some('z') => ':',
            _ => return Err("Unsupported date/time format; use %Y, %m, %d, %H, %M, %S, %z, %:z or %%".into()),
        };
        output.push_str(&value(code));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;

    #[test]
    fn n32_clock_zone_padding_offsets_and_escaping_are_deterministic() {
        let calls = Cell::new(0);
        let snapshot = Snapshot::capture(
            || {
                calls.set(calls.get() + 1);
                UNIX_EPOCH + Duration::from_secs(1_709_251_645)
            },
            |seconds| DateTime::with_offset(seconds, -18_000),
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(snapshot.utc.format(DEFAULT_FORMAT).unwrap(), "2024-03-01T00:07:25+00:00");
        assert_eq!(snapshot.local.format(DEFAULT_FORMAT).unwrap(), "2024-02-29T19:07:25-05:00");
        assert_eq!(snapshot.local.format("界 %% %z %:z").unwrap(), "界 % -0500 -05:00");
        assert_eq!(
            DateTime::with_offset(0, 20_700).unwrap().format(DEFAULT_FORMAT).unwrap(),
            "1970-01-01T05:45:00+05:45"
        );
        for (name, value) in [
            ("CURRENT_YEAR", "2024"),
            ("CURRENT_MONTH", "02"),
            ("CURRENT_DATE", "29"),
            ("CURRENT_HOUR", "19"),
            ("CURRENT_MINUTE", "07"),
            ("CURRENT_SECOND", "25"),
            ("CURRENT_SECONDS_UNIX", "1709251645"),
        ] {
            assert_eq!(snapshot.local.variable(name).as_deref(), Some(value));
        }
        assert!(snapshot.local.variable("RANDOM").is_none());
        assert!(snapshot.local.variable("UUID").is_none());
    }

    #[test]
    fn n32_offset_transition_and_negative_unix_seconds_use_the_captured_instant() {
        for (seconds, offset, expected) in [
            (1_710_053_999, -18_000, "2024-03-10T01:59:59-05:00"),
            (1_710_054_000, -14_400, "2024-03-10T03:00:00-04:00"),
        ] {
            let s =
                Snapshot::capture(|| UNIX_EPOCH + Duration::from_secs(seconds), |s| DateTime::with_offset(s, offset))
                    .unwrap();
            assert_eq!(s.local.format(DEFAULT_FORMAT).unwrap(), expected);
        }
        let s = Snapshot::capture(|| UNIX_EPOCH - Duration::from_millis(1), DateTime::utc).unwrap();
        assert_eq!(s.local.variable("CURRENT_SECONDS_UNIX").as_deref(), Some("-1"));
        assert_eq!(s.local.format(DEFAULT_FORMAT).unwrap(), "1969-12-31T23:59:59+00:00");
    }

    #[test]
    fn n32_only_the_exact_format_whitelist_is_accepted() {
        assert!(validate_format("%Y%m%d%H%M%S%z%:z%%").is_ok());
        assert!(validate_format("").is_ok());
        for invalid in ["%", "%Q", "%s", "%y", "%F", "%Z", "%::z", "%:Y", "%0H", "%Ez", "ok%Y bad%"] {
            assert!(validate_format(invalid).is_err(), "{invalid}");
        }
    }
}
