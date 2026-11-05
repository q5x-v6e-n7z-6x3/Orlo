//! Native spelling and word boundaries, owned by the spelling worker.

use crate::spell::{is_owner_thread, Checker, Misspelling, WordAction};
use orlo_utils::objc::{class, objc_msgSend, pool, sel, Id, NsString};
use std::ffi::{c_char, CStr};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[repr(C)]
#[derive(Clone, Copy)]
struct CfRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringTokenizerCreate(allocator: Id, string: Id, range: CfRange, options: usize, locale: Id) -> Id;
    fn CFStringTokenizerGoToTokenAtIndex(tokenizer: Id, index: isize) -> usize;
    fn CFStringTokenizerGetCurrentTokenRange(tokenizer: Id) -> CfRange;
    fn CFRelease(object: Id);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NsRange {
    location: usize,
    length: usize,
}

/// A platform word-boundary view, retained only within an owner-thread native read.
struct NativeWords {
    tokenizer: Id,
    length: isize,
}

impl NativeWords {
    fn new(string: &NsString) -> Option<Self> {
        assert!(is_owner_thread(), "native word boundary owner");
        // SAFETY: NSString is toll-free bridged to CFString. SDK CFIndex/CFRange
        // are signed pointer-sized fields; UnitWord=0 and NULL locale auto-detects.
        unsafe {
            let length: unsafe extern "C" fn(Id, Id) -> usize = std::mem::transmute(objc_msgSend as *const ());
            let length = isize::try_from(length(string.0, sel(c"length"))).ok()?;
            let tokenizer = CFStringTokenizerCreate(
                std::ptr::null_mut(),
                string.0,
                CfRange { location: 0, length },
                0,
                std::ptr::null_mut(),
            );
            (!tokenizer.is_null()).then(|| Self { tokenizer, length })
        }
    }

    fn at(&self, text: &str, utf16: usize, stop: &AtomicBool) -> Option<Range<usize>> {
        if stop.load(Relaxed) {
            return None;
        }
        let index = isize::try_from(utf16).ok()?;
        if index >= self.length {
            return None;
        }
        // SAFETY: this owned tokenizer and valid UTF-16 index remain inside the native read.
        let range = unsafe {
            if CFStringTokenizerGoToTokenAtIndex(self.tokenizer, index) == 0 {
                return None;
            }
            CFStringTokenizerGetCurrentTokenRange(self.tokenizer)
        };
        let start = usize::try_from(range.location).ok()?;
        let length = usize::try_from(range.length).ok()?;
        if length == 0 {
            return None;
        }
        utf16_range_stopping(text, start, length, stop)
    }
}

impl Drop for NativeWords {
    fn drop(&mut self) {
        // SAFETY: Create returned one owned tokenizer; release in the same native read.
        unsafe { CFRelease(self.tokenizer) };
    }
}

/// A private document tag on AppKit's singleton; all handles belong to `orlo-spell` (!Send/!Sync).
pub struct SpellChecker {
    checker: Id,
    tag: isize,
}

impl SpellChecker {
    /// Only the registered spelling owner may touch the shared checker, including initialization.
    pub fn new() -> std::io::Result<Self> {
        if !is_owner_thread() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "The shared spell checker belongs to the spelling worker",
            ));
        }
        pool(|| unsafe {
            // SAFETY: the SDK declares object/NSInteger/BOOL results and arguments as cast here.
            let object: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let integer: unsafe extern "C" fn(Id, Id) -> isize = std::mem::transmute(objc_msgSend as *const ());
            let set: unsafe extern "C" fn(Id, Id, u8) = std::mem::transmute(objc_msgSend as *const ());
            let class = class(c"NSSpellChecker");
            let checker = object(class, sel(c"sharedSpellChecker"));
            if checker.is_null() {
                return Err(std::io::Error::other("System spell checking is unavailable"));
            }
            set(checker, sel(c"setAutomaticallyIdentifiesLanguages:"), 1);
            let document = Self { checker, tag: integer(class, sel(c"uniqueSpellDocumentTag")) };
            #[cfg(test)]
            tests::record_spell_call("new", document.tag);
            Ok(document)
        })
    }

    pub fn misspelled(&self, text: &str, stop: &AtomicBool) -> Vec<Misspelling> {
        assert!(is_owner_thread(), "shared spell checker owner");
        #[cfg(test)]
        tests::record_spell_call("check", self.tag);
        pool(|| {
            let string = NsString::new(text);
            if string.0.is_null() || stop.load(Relaxed) {
                return Vec::new();
            }
            // SAFETY: SDK NSSpellChecker.h/NSTextCheckingResult.h signatures, with NSRange's
            // two NSUInteger fields. All Cocoa objects stay on this thread and inside this pool.
            unsafe {
                let check: unsafe extern "C" fn(Id, Id, Id, NsRange, u64, Id, isize, *mut Id, *mut isize) -> Id =
                    std::mem::transmute(objc_msgSend as *const ());
                let count: unsafe extern "C" fn(Id, Id) -> usize = std::mem::transmute(objc_msgSend as *const ());
                let at: unsafe extern "C" fn(Id, Id, usize) -> Id = std::mem::transmute(objc_msgSend as *const ());
                let kind: unsafe extern "C" fn(Id, Id) -> u64 = std::mem::transmute(objc_msgSend as *const ());
                let range: unsafe extern "C" fn(Id, Id) -> NsRange = std::mem::transmute(objc_msgSend as *const ());
                let length: unsafe extern "C" fn(Id, Id) -> usize = std::mem::transmute(objc_msgSend as *const ());
                // Orthography lets the unified API identify language instead of fixing English.
                let results = check(
                    self.checker,
                    sel(c"checkString:range:types:options:inSpellDocumentWithTag:orthography:wordCount:"),
                    string.0,
                    NsRange { location: 0, length: length(string.0, sel(c"length")) },
                    1 | 2,
                    std::ptr::null_mut(),
                    self.tag,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if stop.load(Relaxed) {
                    return Vec::new();
                }
                // One native boundary owner reuses the same NSString for every returned error.
                let Some(words) = NativeWords::new(&string) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                for i in 0..count(results, sel(c"count")) {
                    if stop.load(Relaxed) {
                        break;
                    }
                    let result = at(results, sel(c"objectAtIndex:"), i);
                    if kind(result, sel(c"resultType")) & 2 != 0 {
                        let r = range(result, sel(c"range"));
                        if let Some(bytes) = utf16_range_stopping(text, r.location, r.length, stop) {
                            let source = words
                                .at(text, r.location, stop)
                                .filter(|source| source.start <= bytes.start && source.end >= bytes.end)
                                .unwrap_or_else(|| bytes.clone());
                            if stop.load(Relaxed) {
                                break;
                            }
                            out.push(Misspelling { range: bytes, source });
                        }
                    }
                }
                out
            }
        })
    }

    fn change_word(&self, word: &str, action: WordAction) {
        assert!(is_owner_thread(), "shared spell checker owner");
        pool(|| unsafe {
            let word = NsString::new(word);
            // SAFETY: SDK word mutations take NSString; ignore additionally takes the private NSInteger tag.
            match action {
                WordAction::Ignore => {
                    let ignore: unsafe extern "C" fn(Id, Id, Id, isize) =
                        std::mem::transmute(objc_msgSend as *const ());
                    ignore(self.checker, sel(c"ignoreWord:inSpellDocumentWithTag:"), word.0, self.tag);
                    #[cfg(test)]
                    tests::record_spell_call("ignore", self.tag);
                }
                WordAction::Learn | WordAction::Unlearn => {
                    let change: unsafe extern "C" fn(Id, Id, Id) = std::mem::transmute(objc_msgSend as *const ());
                    let method = if matches!(action, WordAction::Learn) { c"learnWord:" } else { c"unlearnWord:" };
                    change(self.checker, sel(method), word.0);
                }
                WordAction::Replace(_) => {
                    unreachable!("replacements belong to the editor")
                }
            }
        });
    }
}

impl Checker for SpellChecker {
    /// Native word containing a UTF-8 caret, preferring the preceding word at its end.
    fn word_at(&self, text: &str, at: usize, stop: &AtomicBool) -> Option<Range<usize>> {
        use Relaxed;
        assert!(is_owner_thread(), "shared spell checker owner");
        #[cfg(test)]
        tests::record_spell_call("word_at", self.tag);
        if !text.is_char_boundary(at) || stop.load(Relaxed) {
            return None;
        }
        let mut caret = 0;
        for c in text[..at].chars() {
            if stop.load(Relaxed) {
                return None;
            }
            caret += c.len_utf16();
        }
        let previous = text[..at].chars().next_back().map(|c| caret - c.len_utf16());
        pool(|| {
            let string = NsString::new(text);
            if string.0.is_null() || stop.load(Relaxed) {
                return None;
            }
            let words = NativeWords::new(&string)?;
            // Probing the previous scalar first makes document/word ends inclusive.
            for index in previous.into_iter().chain(std::iter::once(caret)) {
                if stop.load(Relaxed) {
                    return None;
                }
                if let Some(bytes) = words.at(text, index, stop) {
                    if bytes.start <= at && at <= bytes.end {
                        for c in text[bytes.clone()].chars() {
                            if stop.load(Relaxed) {
                                return None;
                            }
                            if c.is_alphanumeric() {
                                return Some(bytes);
                            }
                        }
                    }
                }
            }
            None
        })
    }

    fn guesses(&self, word: &str) -> Vec<String> {
        assert!(is_owner_thread(), "shared spell checker owner");
        #[cfg(test)]
        tests::record_spell_call("guesses", self.tag);
        pool(|| unsafe {
            // SAFETY: SDK guessesForWordRange uses NSRange/NSString/NSInteger; strings are copied in this pool.
            let guesses: unsafe extern "C" fn(Id, Id, NsRange, Id, Id, isize) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let count: unsafe extern "C" fn(Id, Id) -> usize = std::mem::transmute(objc_msgSend as *const ());
            let at: unsafe extern "C" fn(Id, Id, usize) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let utf8: unsafe extern "C" fn(Id, Id) -> *const c_char = std::mem::transmute(objc_msgSend as *const ());
            let string = NsString::new(word);
            let array = guesses(
                self.checker,
                sel(c"guessesForWordRange:inString:language:inSpellDocumentWithTag:"),
                NsRange { location: 0, length: word.encode_utf16().count() },
                string.0,
                std::ptr::null_mut(),
                self.tag,
            );
            (0..count(array, sel(c"count")))
                .filter_map(|i| {
                    let p = utf8(at(array, sel(c"objectAtIndex:"), i), sel(c"UTF8String"));
                    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
                })
                .collect()
        })
    }
    fn has_learned_word(&self, word: &str) -> bool {
        assert!(is_owner_thread(), "shared spell checker owner");
        #[cfg(test)]
        tests::record_spell_call("learned", self.tag);
        pool(|| unsafe {
            // SAFETY: hasLearnedWord: takes NSString and returns Objective-C BOOL.
            let learned: unsafe extern "C" fn(Id, Id, Id) -> u8 = std::mem::transmute(objc_msgSend as *const ());
            learned(self.checker, sel(c"hasLearnedWord:"), NsString::new(word).0) != 0
        })
    }
    fn ignore_word(&self, word: &str) {
        self.change_word(word, WordAction::Ignore);
    }
    fn learn_word(&self, word: &str) {
        self.change_word(word, WordAction::Learn);
    }
    fn unlearn_word(&self, word: &str) {
        self.change_word(word, WordAction::Unlearn);
    }
    fn misspelled(&self, text: &str, stop: &AtomicBool) -> Vec<Misspelling> {
        self.misspelled(text, stop)
    }
}

impl Drop for SpellChecker {
    fn drop(&mut self) {
        assert!(is_owner_thread(), "shared spell checker owner");
        pool(|| unsafe {
            // SAFETY: the private tag is closed once, on its creating thread.
            let close: unsafe extern "C" fn(Id, Id, isize) = std::mem::transmute(objc_msgSend as *const ());
            close(self.checker, sel(c"closeSpellDocumentWithTag:"), self.tag);
        });
        #[cfg(test)]
        tests::record_spell_call("close", self.tag);
    }
}

#[cfg(test)]
fn utf16_range(text: &str, start: usize, length: usize) -> Option<Range<usize>> {
    utf16_range_stopping(text, start, length, &AtomicBool::new(false))
}

fn utf16_range_stopping(text: &str, start: usize, length: usize, stop: &AtomicBool) -> Option<Range<usize>> {
    let end = start.checked_add(length)?;
    let (mut utf16, mut from) = (0, None);
    for (byte, c) in text.char_indices().chain(std::iter::once((text.len(), '\0'))) {
        if stop.load(Relaxed) {
            return None;
        }
        if utf16 == start {
            from = Some(byte);
        }
        if utf16 == end {
            return Some(from?..byte);
        }
        utf16 += c.len_utf16();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    thread_local! {
        static SPELL_CALLS: std::cell::RefCell<Vec<(&'static str, isize, std::thread::ThreadId)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    pub(super) fn record_spell_call(call: &'static str, tag: isize) {
        assert!(crate::spell::is_owner_thread());
        SPELL_CALLS.with_borrow_mut(|calls| calls.push((call, tag, std::thread::current().id())));
    }

    #[test]
    fn native_constructor_rejects_non_owner_even_with_worker_name() {
        let refused = || SpellChecker::new().err().map(|error| error.kind());
        assert_eq!(refused(), Some(std::io::ErrorKind::PermissionDenied));
        let named = std::thread::Builder::new().name("orlo-spell".into()).spawn(refused).unwrap();
        assert_eq!(named.join().unwrap(), Some(std::io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn native_spelling_uses_private_tags_and_utf16_ranges() {
        crate::spell::test_on_worker(|| {
            assert!(crate::spell::is_owner_thread());
            SPELL_CALLS.take();
            let owner = std::thread::current().id();
            let checker = SpellChecker::new().expect("native spell checker");
            let tag = checker.tag;
            assert_ne!(tag, 0, "a private spell document");
            let text = "😀 café this is a qzxqvzzmispelling.";
            let stop = AtomicBool::new(false);
            let ranges = checker.misspelled(text, &stop);
            assert!(
                ranges
                    .iter()
                    .any(|r| &text[r.range.clone()] == "qzxqvzzmispelling"
                        && &text[r.source.clone()] == "qzxqvzzmispelling"),
                "{ranges:?}"
            );
            assert!(checker.misspelled("this is a correct sentence", &stop).is_empty());
            assert!(!checker.guesses("mispelled").is_empty());
            let _already_learned = checker.has_learned_word("qzxqvzzmispelling");
            checker.change_word("qzxqvzzmispelling", crate::spell::WordAction::Ignore);
            assert!(checker.misspelled("qzxqvzzmispelling", &stop).is_empty());
            drop(checker);
            assert_eq!(
                SPELL_CALLS.take(),
                [
                    ("new", tag, owner),
                    ("check", tag, owner),
                    ("check", tag, owner),
                    ("guesses", tag, owner),
                    ("learned", tag, owner),
                    ("ignore", tag, owner),
                    ("check", tag, owner),
                    ("close", tag, owner)
                ]
            );
        });
    }

    #[test]
    fn native_words_preserve_unicode_carets_and_source_exclusions() {
        crate::spell::test_on_worker(|| {
            SPELL_CALLS.take();
            let owner = std::thread::current().id();
            let checker = SpellChecker::new().unwrap();
            let tag = checker.tag;
            assert_ne!(tag, 0);
            let stop = AtomicBool::new(false);
            let text = "😀 café learned's, e\u{301}lan!";
            // UTF-16 3..7 is UTF-8 5..10; a caret may lie at either word end.
            for at in [5, 8, 10] {
                assert_eq!(checker.word_at(text, at, &stop), Some(5..10), "{at}");
            }
            for at in [11, 15, 20] {
                assert_eq!(checker.word_at(text, at, &stop), Some(11..20), "{at}");
            }
            for at in [22, 23, 28] {
                assert_eq!(checker.word_at(text, at, &stop), Some(22..28), "{at}");
            }
            for at in [0, 1, 4, 9, 21, 24, text.len() + 1, usize::MAX] {
                assert_eq!(checker.word_at(text, at, &stop), None, "{at}");
            }
            assert_eq!(checker.word_at("", 0, &stop), None);
            assert_eq!(checker.word_at("café", 5, &stop), Some(0..5));
            assert_eq!(checker.word_at("e\u{301}lan", 6, &stop), Some(0..6));
            assert_eq!(checker.word_at("foo, ... bar", 5, &stop), None);
            for (text, at, source, eligible) in [
                ("word2", 2, "word2", false),
                ("under_score", 2, "under", false),
                ("under_score", 8, "score", false),
                ("camelCase", 4, "camelCase", false),
                ("caféCase", 2, "caféCase", false),
                ("foo'Bar", 5, "foo'Bar", false),
                ("foo’Bar", 7, "foo’Bar", false),
                ("e\u{301}lanCase", 4, "e\u{301}lanCase", false),
                ("don't", 3, "don't", true),
                ("qzxword's", 4, "qzxword's", true),
                ("Capital", 3, "Capital", true),
                ("HELLO", 3, "HELLO", true),
                ("good,under_score", 2, "good", true),
                ("中文camelCase", 3, "中文", true),
                ("中文camelCase", 7, "camelCase", false),
                ("カタカナcamelCase", 3, "カタカナ", true),
                ("カタカナcamelCase", 13, "camelCase", false),
            ] {
                let range = checker.word_at(text, at, &stop).unwrap();
                assert_eq!(&text[range.clone()], source, "{text}");
                assert_eq!(crate::spell::source_eligible(text, &range, &stop), eligible, "{text}");
            }
            stop.store(true, Ordering::Relaxed);
            assert_eq!(checker.word_at(text, 8, &stop), None);
            drop(checker);
            let calls = SPELL_CALLS.take();
            assert_eq!(calls.first(), Some(&("new", tag, owner)));
            assert_eq!(calls.last(), Some(&("close", tag, owner)));
            assert!(calls[1..calls.len() - 1].iter().all(|call| *call == ("word_at", tag, owner)));
        });
    }

    #[test]
    fn native_ranges_reject_surrogate_splits_and_invalid_offsets() {
        let text = "😀éword";
        assert_eq!(utf16_range(text, 3, 4), Some(6..10));
        assert_eq!(utf16_range(text, 1, 1), None);
        assert_eq!(utf16_range(text, usize::MAX, 1), None);
        assert_eq!(utf16_range(text, 7, 1), None);
    }
}
