//! Shared Objective-C runtime helpers for macOS integrations.

use std::ffi::{c_char, c_void, CStr};

pub type Id = *mut c_void;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    pub fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

pub fn class(name: &CStr) -> Id {
    // SAFETY: a NUL-terminated name; the runtime returns null for an unknown class.
    unsafe { objc_getClass(name.as_ptr()) }
}

pub fn sel(name: &CStr) -> Id {
    // SAFETY: a NUL-terminated selector name; registering one is idempotent.
    unsafe { sel_registerName(name.as_ptr()) }
}

/// Runs `f` inside an autorelease pool: objects Cocoa hands back autoreleased go with it.
pub fn pool<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: push and pop are paired on this thread.
    let p = unsafe { objc_autoreleasePoolPush() };
    let r = f();
    unsafe { objc_autoreleasePoolPop(p) };
    r
}

/// An NSString of `s`'s characters (a NUL among them too), released when dropped: as long, in
/// UTF-16 units, as `s.encode_utf16()`, which callers count ranges in. Null if Cocoa would not
/// make one.
pub struct NsString(pub Id);

impl NsString {
    pub fn new(s: &str) -> Self {
        // from UTF-16: every UTF-8 initializer drops a leading U+FEFF, leaving the string a unit
        // shorter than the ranges counted for it (an exception, through Rust frames, on use)
        let units: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: each objc_msgSend is cast to the message's signature; -initWithCharacters:
        // length: copies the units, which live through the call, and answers nil if it cannot.
        unsafe {
            let alloc: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let init: unsafe extern "C" fn(Id, Id, *const u16, usize) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let fresh = alloc(class(c"NSString"), sel(c"alloc"));
            Self(init(fresh, sel(c"initWithCharacters:length:"), units.as_ptr(), units.len()))
        }
    }
}

impl Drop for NsString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the one reference +alloc/-init gave, given back once.
            unsafe {
                let release: unsafe extern "C" fn(Id, Id) = std::mem::transmute(objc_msgSend as *const ());
                release(self.0, sel(c"release"));
            }
        }
    }
}
