//! Foundation collation with a fixed locale and owned UTF-8 keys for one sort.

use crate::objc::{class, objc_msgSend, pool, sel, Id, NsString};
use std::cmp::Ordering;

/// One NSString per key, with the full UTF-16 range (including embedded NULs).
pub struct Key {
    value: NsString,
    length: usize,
}

impl Key {
    pub fn new(text: &str) -> Result<Self, String> {
        let value = NsString::new(text);
        if value.0.is_null() {
            return Err("macOS could not create a sort key".into());
        }
        Ok(Self { value, length: text.encode_utf16().count() })
    }
}

/// A retained, non-autoupdating locale. A preference change applies to the next sort.
pub struct Collator {
    locale: Id,
}

impl Collator {
    pub fn current() -> Result<Self, String> {
        pool(|| {
            // SAFETY: currentLocale returns an NSLocale, retained before the pool is drained.
            unsafe {
                let send: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
                let locale = send(class(c"NSLocale"), sel(c"currentLocale"));
                let locale = send(locale, sel(c"retain"));
                Self::from_owned(locale)
            }
        })
    }

    fn from_owned(locale: Id) -> Result<Self, String> {
        if locale.is_null() {
            return Err("macOS could not read the sorting locale".into());
        }
        Ok(Self { locale })
    }

    pub fn compare(&self, a: &Key, b: &Key, ignore_case: bool) -> Ordering {
        #[repr(C)]
        struct Range {
            location: usize,
            length: usize,
        }
        // SAFETY: owned NSString/NSLocale receivers live through the call, NSUInteger/NSRange
        // match the SDK ABI, and the UTF-16 range covers exactly the receiver's contents.
        let result = pool(|| unsafe {
            let compare: unsafe extern "C" fn(Id, Id, Id, usize, Range, Id) -> isize =
                std::mem::transmute(objc_msgSend as *const ());
            compare(
                a.value.0,
                sel(c"compare:options:range:locale:"),
                b.value.0,
                usize::from(ignore_case), // NSCaseInsensitiveSearch = 1
                Range { location: 0, length: a.length },
                self.locale,
            )
        });
        result.cmp(&0)
    }
}

impl Drop for Collator {
    fn drop(&mut self) {
        // SAFETY: release exactly the reference retained or initialized for this instance.
        unsafe {
            let release: unsafe extern "C" fn(Id, Id) = std::mem::transmute(objc_msgSend as *const ());
            release(self.locale, sel(c"release"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Collator {
        let name = NsString::new(name);
        // SAFETY: NSLocale copies the valid identifier and returns one owned reference.
        unsafe {
            let alloc: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let init: unsafe extern "C" fn(Id, Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let locale = init(alloc(class(c"NSLocale"), sel(c"alloc")), sel(c"initWithLocaleIdentifier:"), name.0);
            Collator::from_owned(locale).unwrap()
        }
    }

    #[test]
    fn n30_native_locale_fixture_honors_region_canonical_keys_and_case_options() {
        let a = Key::new("ä").unwrap();
        let z = Key::new("z").unwrap();
        assert_eq!(named("en_US").compare(&a, &z, false), Ordering::Less);
        assert_eq!(named("sv_SE").compare(&a, &z, false), Ordering::Greater);
        let composed = Key::new("é").unwrap();
        let combining = Key::new("e\u{301}").unwrap();
        assert_eq!(named("en_US").compare(&composed, &combining, false), Ordering::Equal);
        let upper = Key::new("I").unwrap();
        let lower = Key::new("ı").unwrap();
        assert_eq!(named("tr_TR").compare(&upper, &lower, true), Ordering::Equal);
    }

    #[test]
    fn a_key_that_starts_with_a_byte_order_mark_keeps_its_whole_length() {
        // a UTF-8 NSString drops a leading U+FEFF: comparing the range Rust counts threw
        let bom = Key::new("\u{feff}b").unwrap();
        let length: usize = unsafe {
            let length: unsafe extern "C" fn(Id, Id) -> usize = std::mem::transmute(objc_msgSend as *const ());
            length(bom.value.0, sel(c"length"))
        };
        assert_eq!(length, bom.length);
        let a = Key::new("a").unwrap();
        assert_ne!(named("en_US").compare(&bom, &a, false), Ordering::Equal);
    }

    #[test]
    fn n30_current_locale_matches_native_localized_compare_for_full_keys() {
        let locale = Collator::current().unwrap();
        for (a, b) in [("ä", "z"), ("İ", "i"), ("a\0z", "a\0b"), ("👩‍💻", "😀"), ("é", "e\u{301}")] {
            let a = Key::new(a).unwrap();
            let b = Key::new(b).unwrap();
            // Independent Foundation entry point, documented to use the user's current locale.
            let native = pool(|| unsafe {
                let compare: unsafe extern "C" fn(Id, Id, Id) -> isize = std::mem::transmute(objc_msgSend as *const ());
                compare(a.value.0, sel(c"localizedCompare:"), b.value.0)
            });
            assert_eq!(locale.compare(&a, &b, false), native.cmp(&0));
        }
    }
}
