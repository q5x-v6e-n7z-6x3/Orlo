//! Cocoa integrations through the Objective-C runtime gpui already links.

pub mod printing;

use orlo_utils::objc::{class, objc_msgSend, pool, sel, Id, NsString};
#[cfg(test)]
use std::ffi::CString;
use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

type KeyNotification = unsafe extern "C" fn(Id, Id, Id);

#[link(name = "objc")]
extern "C" {
    fn class_getSuperclass(class: Id) -> Id;
    fn class_getInstanceMethod(class: Id, selector: Id) -> Id;
    fn method_getTypeEncoding(method: Id) -> *const c_char;
    fn method_getImplementation(method: Id) -> KeyNotification;
    fn method_setImplementation(method: Id, implementation: KeyNotification) -> KeyNotification;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

/// `objc_msgSend` as the `unsafe extern "C" fn` type `F`: the message's own signature.
///
/// # Safety
/// `F` must be a function pointer type matching the message sent through it.
unsafe fn msg<F: Copy>() -> F {
    std::mem::transmute_copy(&(objc_msgSend as *const ()))
}

static WINDOW_KEY_NOTIFICATION: OnceLock<KeyNotification> = OnceLock::new();
static PANEL_KEY_NOTIFICATION: OnceLock<KeyNotification> = OnceLock::new();

/// Install on the main thread before opening a native window. A false key-window
/// notification must be balanced before entering gpui's non-reentrant state lock.
// gpui 0.2.2 can deadlock during a false key notification. Remove this
// integration guard after upgrading to a released version with safe reentry.
// Diagnosis: https://github.com/zed-industries/zed/issues/50151
pub fn install_key_window_guard() -> std::io::Result<()> {
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();
    match INSTALLED.get_or_init(|| {
        install_key_guard(c"GPUIWindow", &WINDOW_KEY_NOTIFICATION, window_became_key)?;
        install_key_guard(c"GPUIPanel", &PANEL_KEY_NOTIFICATION, panel_became_key)
    }) {
        Ok(()) => Ok(()),
        Err(why) => Err(std::io::Error::other(why.clone())),
    }
}

fn install_key_guard(
    name: &CStr,
    original: &OnceLock<KeyNotification>,
    replacement: KeyNotification,
) -> Result<(), String> {
    // SAFETY: named classes are registered by gpui before main. Only a method owned
    // by that class with the exact void/object/object/object ABI may be replaced.
    let what = name.to_string_lossy();
    unsafe {
        let class = class(name);
        if class.is_null() {
            return Err(format!("{what} is unavailable"));
        }
        let selector = sel(c"windowDidBecomeKey:");
        let method = class_getInstanceMethod(class, selector);
        let superclass = class_getSuperclass(class);
        if method.is_null() || (!superclass.is_null() && method == class_getInstanceMethod(superclass, selector)) {
            return Err(format!("{what} has no owned key callback"));
        }
        let encoding = method_getTypeEncoding(method);
        if encoding.is_null()
            || CStr::from_ptr(encoding)
                .to_bytes()
                .iter()
                .copied()
                .filter(|c| !c.is_ascii_digit())
                .ne(b"v@:@".iter().copied())
        {
            return Err(format!("{what} has an unsupported key callback"));
        }
        if original.get().is_some() {
            return if method_getImplementation(method) as usize == replacement as usize {
                Ok(())
            } else {
                Err(format!("{what} key callback changed"))
            };
        }
        original.set(method_getImplementation(method)).map_err(|_| "key callback was already installed".to_string())?;
        // Publish the original before Cocoa can dispatch through the replacement.
        method_setImplementation(method, replacement);
        Ok(())
    }
}

unsafe extern "C" fn window_became_key(window: Id, selector: Id, notification: Id) {
    dispatch_key_notification(window, selector, notification, &WINDOW_KEY_NOTIFICATION);
}

unsafe extern "C" fn panel_became_key(window: Id, selector: Id, notification: Id) {
    dispatch_key_notification(window, selector, notification, &PANEL_KEY_NOTIFICATION);
}

fn dispatch_key_notification(window: Id, selector: Id, notification: Id, original: &OnceLock<KeyNotification>) {
    if let Some(original) = original.get() {
        // SAFETY: Cocoa supplies a live window for this callback. Inspect its
        // actual key state before entering a delegate that owns a Rust mutex.
        unsafe {
            let is_key: unsafe extern "C" fn(Id, Id) -> u8 = msg();
            if is_key(window, sel(c"isKeyWindow")) == 0 {
                let resign: unsafe extern "C" fn(Id, Id) = msg();
                // The resulting resign notification may re-enter the delegate
                // synchronously. No application or framework lock is held here.
                resign(window, sel(c"resignKeyWindow"));
                return;
            }
            // The installation validated this ABI and published the saved
            // implementation before making this replacement reachable.
            original(window, selector, notification);
        }
    }
}

/// System Settings > Accessibility > Display > Reduce motion.
pub fn reduce_motion() -> bool {
    pool(|| {
        let workspace = class(c"NSWorkspace");
        if workspace.is_null() {
            return false;
        }
        // SAFETY: objc_msgSend cast to each message's signature: +sharedWorkspace returns an
        // object, -accessibilityDisplayShouldReduceMotion a BOOL (one byte).
        unsafe {
            let id: unsafe extern "C" fn(Id, Id) -> Id = msg();
            let flag: unsafe extern "C" fn(Id, Id) -> u8 = msg();
            let shared = id(workspace, sel(c"sharedWorkspace"));
            !shared.is_null() && flag(shared, sel(c"accessibilityDisplayShouldReduceMotion")) != 0
        }
    })
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> Id;
    fn TISGetInputSourceProperty(source: Id, property: Id) -> Id;
    static kTISPropertyInputSourceType: Id;
    static kTISTypeKeyboardLayout: Id;
    static kTISPropertyUnicodeKeyLayoutData: Id;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        key: u16,
        action: u16,
        modifiers: u32,
        keyboard: u32,
        options: u32,
        dead: *mut u32,
        capacity: usize,
        length: *mut usize,
        output: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFEqual(left: Id, right: Id) -> u8;
    fn CFRelease(value: Id);
    fn CFDataGetBytePtr(data: Id) -> *const u8;
}

/// Literal key interception may use layout text only when no native input method owns it.
pub fn direct_keyboard_input() -> bool {
    // SAFETY: TIS returns a retained source and a borrowed CFString type property. The source
    // stays alive through CFEqual and is released exactly once. A missing source fails closed.
    unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return false;
        }
        let kind = TISGetInputSourceProperty(source, kTISPropertyInputSourceType);
        let direct = !kind.is_null() && CFEqual(kind, kTISTypeKeyboardLayout) != 0;
        CFRelease(source);
        direct
    }
}

/// GPUI resolves layout dead keys to a spacing `key_char`; defer those native events to IME.
/// This main-thread probe inspects OS dead-key state only and never inserts translated text.
pub fn literal_keyboard_text() -> Option<String> {
    pool(|| {
        // SAFETY: currentEvent is borrowed for this main-thread callback; the source's retained
        // layout data outlives UCKeyTranslate. Exact SDK signatures and bounded output storage.
        unsafe {
            let id: unsafe extern "C" fn(Id, Id) -> Id = msg();
            let integer: unsafe extern "C" fn(Id, Id) -> usize = msg();
            let key_code: unsafe extern "C" fn(Id, Id) -> u16 = msg();
            let app = id(class(c"NSApplication"), sel(c"sharedApplication"));
            let event = id(app, sel(c"currentEvent"));
            if event.is_null() || integer(event, sel(c"type")) != 10 {
                return None;
            }
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let direct = (|| {
                let kind = TISGetInputSourceProperty(source, kTISPropertyInputSourceType);
                if kind.is_null() || CFEqual(kind, kTISTypeKeyboardLayout) == 0 {
                    return false;
                }
                let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
                if data.is_null() {
                    return false;
                }
                let layout = CFDataGetBytePtr(data);
                if layout.is_null() {
                    return false;
                }
                // Only Shift/Caps Lock remain after shortcut/Option admission. Carbon uses
                // bit 1 for Shift and bit 2 for Caps Lock; NSEvent uses bits 17 and 16.
                let flags = integer(event, sel(c"modifierFlags"));
                let modifiers = ((flags >> 16) & 2) | (((flags >> 16) & 1) << 2);
                let (mut dead, mut length, mut output) = (0, 0, [0u16; 8]);
                let status = UCKeyTranslate(
                    layout.cast(),
                    key_code(event, sel(c"keyCode")),
                    0,
                    modifiers as u32,
                    u32::from(LMGetKbdType()),
                    0,
                    &mut dead,
                    output.len(),
                    &mut length,
                    output.as_mut_ptr(),
                );
                status == 0 && dead == 0 && length > 0
            })();
            CFRelease(source);
            if !direct {
                return None;
            }
            // Use the exact native event string, including Caps Lock and non-layout text.
            // Count UTF-8 bytes explicitly so embedded NUL is preserved for input validation.
            let characters = id(event, sel(c"characters"));
            if characters.is_null() {
                return None;
            }
            let byte_length: unsafe extern "C" fn(Id, Id, usize) -> usize = msg();
            let utf8: unsafe extern "C" fn(Id, Id) -> *const u8 = msg();
            let length = byte_length(characters, sel(c"lengthOfBytesUsingEncoding:"), 4);
            let bytes = utf8(characters, sel(c"UTF8String"));
            if length == 0 || length > isize::MAX as usize || bytes.is_null() {
                return None;
            }
            std::str::from_utf8(std::slice::from_raw_parts(bytes, length)).ok().map(str::to_string)
        }
    })
}

pub use orlo_project::filesystem::platform::trash;

/// The pasteboard called `name`, made if it is new, or the general one for `None`. Autoreleased;
/// null if Cocoa has none.
fn pasteboard(name: Option<&str>) -> Id {
    let pasteboards = class(c"NSPasteboard");
    if pasteboards.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: each objc_msgSend is cast to the message's signature; the name outlives the call.
    unsafe {
        let general: unsafe extern "C" fn(Id, Id) -> Id = msg();
        let named: unsafe extern "C" fn(Id, Id, Id) -> Id = msg();
        match name {
            None => general(pasteboards, sel(c"generalPasteboard")),
            Some(name) => {
                let name = NsString::new(name);
                match name.0.is_null() {
                    true => std::ptr::null_mut(),
                    false => named(pasteboards, sel(c"pasteboardWithName:"), name.0),
                }
            }
        }
    }
}

/// The pasteboard's ownership generation: a later copy, even of identical text, changes it.
pub fn pasteboard_change_count(name: Option<&str>) -> Option<isize> {
    pool(|| {
        let board = pasteboard(name);
        if board.is_null() {
            return None;
        }
        // SAFETY: -changeCount returns NSInteger; the board is live within the pool.
        let count: unsafe extern "C" fn(Id, Id) -> isize = unsafe { msg() };
        Some(unsafe { count(board, sel(c"changeCount")) })
    })
}

/// Replace what is on the pasteboard called `name` (the general one for `None`) with `html` and
/// `text`, both in the one write: rich editors paste the HTML, plain ones the text. False if
/// Cocoa refused, leaving the pasteboard empty or half written.
pub fn write_html_and_text(name: Option<&str>, html: &str, text: &str) -> bool {
    pool(|| {
        let (board, html, text) = (pasteboard(name), NsString::new(html), NsString::new(text));
        if board.is_null() || html.0.is_null() || text.0.is_null() {
            return false;
        }
        // SAFETY: each objc_msgSend is cast to the message's signature; the strings are live
        // (the types autoreleased by the pool) until the pool ends.
        unsafe {
            let clear: unsafe extern "C" fn(Id, Id) -> isize = msg();
            let utf8: unsafe extern "C" fn(Id, Id, *const c_char) -> Id = msg();
            let set: unsafe extern "C" fn(Id, Id, Id, Id) -> u8 = msg();
            let uti = |name: &CStr| utf8(class(c"NSString"), sel(c"stringWithUTF8String:"), name.as_ptr());
            clear(board, sel(c"clearContents"));
            let set_string = sel(c"setString:forType:");
            set(board, set_string, html.0, uti(c"public.html")) != 0
                && set(board, set_string, text.0, uti(c"public.utf8-plain-text")) != 0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[link(name = "objc")]
    extern "C" {
        fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
        fn objc_registerClassPair(class: Id);
        fn class_addIvar(class: Id, name: *const c_char, size: usize, alignment: u8, encoding: *const c_char) -> u8;
        fn class_addMethod(class: Id, selector: Id, implementation: *const (), encoding: *const c_char) -> u8;
        fn object_setInstanceVariable(object: Id, name: *const c_char, value: Id) -> Id;
        fn object_getInstanceVariable(object: Id, name: *const c_char, value: *mut Id) -> Id;
    }

    #[derive(Default)]
    struct NativeKeyState {
        active: AtomicBool,
        native_resigns: AtomicUsize,
        becomes: AtomicUsize,
        resigns: AtomicUsize,
        original: AtomicUsize,
        receiver: AtomicUsize,
        selector: AtomicUsize,
        notification: AtomicUsize,
    }

    struct NativeKeyFixture {
        name: CString,
        object: Id,
        state: Box<NativeKeyState>,
    }

    impl Drop for NativeKeyFixture {
        fn drop(&mut self) {
            // SAFETY: +new below returns one owned NSObject, released before state drops.
            unsafe {
                let send: unsafe extern "C" fn(Id, Id) = msg();
                send(self.object, sel(c"release"));
            }
        }
    }

    unsafe fn key_state(object: Id) -> &'static NativeKeyState {
        let mut state = std::ptr::null_mut();
        object_getInstanceVariable(object, c"orloKeyState".as_ptr(), &mut state);
        // Fixture owns the stable Box until after the object's final native dispatch.
        &*(state as *const NativeKeyState)
    }

    unsafe extern "C" fn native_is_key(object: Id, _: Id) -> u8 {
        key_state(object).active.load(Ordering::SeqCst) as u8
    }

    unsafe extern "C" fn native_resign(object: Id, _: Id) {
        key_state(object).native_resigns.fetch_add(1, Ordering::SeqCst);
        // A real Objective-C message synchronously re-enters the registered delegate.
        let send: unsafe extern "C" fn(Id, Id, Id) = msg();
        send(object, sel(c"windowDidResignKey:"), object);
    }

    unsafe fn record_key(object: Id, selector: Id, notification: Id, original: usize) {
        let state = key_state(object);
        state.original.store(original, Ordering::SeqCst);
        state.receiver.store(object as usize, Ordering::SeqCst);
        state.selector.store(selector as usize, Ordering::SeqCst);
        state.notification.store(notification as usize, Ordering::SeqCst);
        if selector == sel(c"windowDidBecomeKey:") {
            state.becomes.fetch_add(1, Ordering::SeqCst);
        } else {
            state.resigns.fetch_add(1, Ordering::SeqCst);
        }
    }

    unsafe extern "C" fn original_a(object: Id, selector: Id, notification: Id) {
        record_key(object, selector, notification, 1);
    }

    unsafe extern "C" fn original_b(object: Id, selector: Id, notification: Id) {
        record_key(object, selector, notification, 2);
    }

    static ORIGINAL_A: OnceLock<KeyNotification> = OnceLock::new();
    static ORIGINAL_B: OnceLock<KeyNotification> = OnceLock::new();

    unsafe extern "C" fn guarded_a(object: Id, selector: Id, notification: Id) {
        dispatch_key_notification(object, selector, notification, &ORIGINAL_A);
    }

    unsafe extern "C" fn guarded_b(object: Id, selector: Id, notification: Id) {
        dispatch_key_notification(object, selector, notification, &ORIGINAL_B);
    }

    fn allocate_test_class(superclass: Id) -> (CString, Id) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = CString::new(format!("OrloKeyTest_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)))
            .unwrap();
        let class = unsafe { objc_allocateClassPair(superclass, name.as_ptr(), 0) };
        assert!(!class.is_null());
        (name, class)
    }

    fn native_key_fixture(original: KeyNotification, encoding: &CStr) -> NativeKeyFixture {
        let (name, class) = allocate_test_class(class(c"NSObject"));
        unsafe {
            assert_ne!(
                class_addIvar(
                    class,
                    c"orloKeyState".as_ptr(),
                    std::mem::size_of::<Id>(),
                    std::mem::align_of::<Id>().trailing_zeros() as u8,
                    c"^v".as_ptr(),
                ),
                0
            );
            for (selector, implementation, encoding) in [
                (c"isKeyWindow", native_is_key as *const (), c"c@:"),
                (c"resignKeyWindow", native_resign as *const (), c"v@:"),
                (c"windowDidBecomeKey:", original as *const (), encoding),
                (c"windowDidResignKey:", original as *const (), c"v@:@"),
            ] {
                assert_ne!(class_addMethod(class, sel(selector), implementation, encoding.as_ptr()), 0);
            }
            objc_registerClassPair(class);
            let send: unsafe extern "C" fn(Id, Id) -> Id = msg();
            let object = send(class, sel(c"new"));
            assert!(!object.is_null());
            let state = Box::<NativeKeyState>::default();
            object_setInstanceVariable(
                object,
                c"orloKeyState".as_ptr(),
                (&*state as *const NativeKeyState).cast_mut().cast(),
            );
            NativeKeyFixture { name, object, state }
        }
    }

    #[test]
    fn native_key_guard_balances_false_notifications_and_forwards_real_ones() {
        let first = native_key_fixture(original_a, c"v24@0:8@16");
        let second = native_key_fixture(original_b, c"v@:@");
        install_key_guard(&first.name, &ORIGINAL_A, guarded_a).unwrap();
        install_key_guard(&second.name, &ORIGINAL_B, guarded_b).unwrap();
        let send: unsafe extern "C" fn(Id, Id, Id) = unsafe { msg() };
        unsafe { send(first.object, sel(c"windowDidBecomeKey:"), second.object) };
        assert_eq!(
            first.state.becomes.load(Ordering::SeqCst),
            0,
            "false key notification must not enter original handler"
        );
        assert_eq!(first.state.native_resigns.load(Ordering::SeqCst), 1);
        assert_eq!(
            first.state.resigns.load(Ordering::SeqCst),
            1,
            "native resign synchronously reaches the original delegate"
        );
        assert_eq!(first.state.original.load(Ordering::SeqCst), 1);
        assert_eq!(first.state.selector.load(Ordering::SeqCst), sel(c"windowDidResignKey:") as usize);
        first.state.active.store(true, Ordering::SeqCst);
        second.state.active.store(true, Ordering::SeqCst);
        for (fixture, other, original) in [(&first, &second, 1), (&second, &first, 2)] {
            unsafe { send(fixture.object, sel(c"windowDidBecomeKey:"), other.object) };
            assert_eq!(fixture.state.becomes.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.state.original.load(Ordering::SeqCst), original);
            assert_eq!(fixture.state.receiver.load(Ordering::SeqCst), fixture.object as usize);
            assert_eq!(fixture.state.selector.load(Ordering::SeqCst), sel(c"windowDidBecomeKey:") as usize);
            assert_eq!(fixture.state.notification.load(Ordering::SeqCst), other.object as usize);
        }
        install_key_guard(&first.name, &ORIGINAL_A, guarded_a).unwrap();
        first.state.active.store(false, Ordering::SeqCst);
        unsafe { send(first.object, sel(c"windowDidBecomeKey:"), second.object) };
        assert_eq!(first.state.becomes.load(Ordering::SeqCst), 1);
        assert_eq!(first.state.resigns.load(Ordering::SeqCst), 2, "reinstall is idempotent");
        second.state.active.store(false, Ordering::SeqCst);
        unsafe { send(second.object, sel(c"windowDidBecomeKey:"), first.object) };
        assert_eq!(second.state.becomes.load(Ordering::SeqCst), 1);
        assert_eq!(second.state.resigns.load(Ordering::SeqCst), 1);
        assert_eq!(second.state.original.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_key_guard_rejects_missing_inherited_and_incompatible_methods() {
        let original = OnceLock::new();
        assert!(install_key_guard(c"OrloMissingKeyClass", &original, guarded_a).is_err());
        let parent = native_key_fixture(original_a, c"v@:@");
        let before = unsafe {
            method_getImplementation(class_getInstanceMethod(class(&parent.name), sel(c"windowDidBecomeKey:")))
        };
        let (name, subclass) = allocate_test_class(class(&parent.name));
        unsafe { objc_registerClassPair(subclass) };
        assert!(install_key_guard(&name, &original, guarded_a).is_err());
        let after = unsafe {
            method_getImplementation(class_getInstanceMethod(class(&parent.name), sel(c"windowDidBecomeKey:")))
        };
        assert_eq!(before as usize, after as usize, "never replace a superclass method");
        let incompatible = native_key_fixture(original_b, c"v@:");
        assert!(install_key_guard(&incompatible.name, &original, guarded_b).is_err());
        assert!(original.get().is_none());
    }

    #[test]
    fn reduce_motion_answers_without_crashing() {
        // the machine's setting, either way
        let _ = reduce_motion();
    }

    /// A pasteboard of this test's own, released when the test is done: the user's is never touched.
    struct Private(String);

    impl Private {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::SeqCst);
            Self(format!("orlo-test-pasteboard-{}-{n}", std::process::id()))
        }

        fn name(&self) -> Option<&str> {
            Some(&self.0)
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            pool(|| {
                let board = pasteboard(self.name());
                // SAFETY: the pasteboard is this test's own and is not used again.
                unsafe {
                    let release: unsafe extern "C" fn(Id, Id) = msg();
                    release(board, sel(c"releaseGlobally"));
                }
            });
        }
    }

    /// What another app would read from `name` for `uti`: the type's data, if it is there.
    fn read_type(name: Option<&str>, uti: &str) -> Option<String> {
        pool(|| {
            let (board, uti) = (pasteboard(name), NsString::new(uti));
            // SAFETY: each objc_msgSend is cast to the message's signature; the NSData is
            // autoreleased by the pool and read before it ends.
            unsafe {
                let id_for: unsafe extern "C" fn(Id, Id, Id) -> Id = msg();
                let bytes: unsafe extern "C" fn(Id, Id) -> *const u8 = msg();
                let length: unsafe extern "C" fn(Id, Id) -> usize = msg();
                let data = id_for(board, sel(c"dataForType:"), uti.0);
                if data.is_null() {
                    return None;
                }
                let n = length(data, sel(c"length"));
                let data = bytes(data, sel(c"bytes"));
                let all = if n == 0 { &[][..] } else { std::slice::from_raw_parts(data, n) };
                Some(String::from_utf8(all.to_vec()).expect("UTF-8 on the pasteboard"))
            }
        })
    }

    /// Whether `name` lists `uti` among its types: what gpui's paste looks for first.
    fn declares(name: Option<&str>, uti: &str) -> bool {
        pool(|| {
            let (board, uti) = (pasteboard(name), NsString::new(uti));
            // SAFETY: each objc_msgSend is cast to the message's signature; the types array is
            // autoreleased by the pool and used inside it.
            unsafe {
                let types: unsafe extern "C" fn(Id, Id) -> Id = msg();
                let contains: unsafe extern "C" fn(Id, Id, Id) -> u8 = msg();
                contains(types(board, sel(c"types")), sel(c"containsObject:"), uti.0) != 0
            }
        })
    }

    /// Put `text` on `name` under `uti` alone, as another app might.
    fn set_type(name: Option<&str>, uti: &str, text: &str) {
        pool(|| {
            let (board, uti, text) = (pasteboard(name), NsString::new(uti), NsString::new(text));
            // SAFETY: each objc_msgSend is cast to the message's signature.
            unsafe {
                let set: unsafe extern "C" fn(Id, Id, Id, Id) -> u8 = msg();
                assert_ne!(set(board, sel(c"setString:forType:"), text.0, uti.0), 0);
            }
        })
    }

    const HTML: &str = "public.html";
    const TEXT: &str = "public.utf8-plain-text";

    #[test]
    fn pasteboard_generation_tracks_later_copies_even_of_identical_text() {
        let board = Private::new();
        let initial = pasteboard_change_count(board.name()).expect("a private pasteboard");
        assert!(write_html_and_text(board.name(), "<pre>x</pre>", "x"));
        let copied = pasteboard_change_count(board.name()).unwrap();
        assert!(copied > initial);
        assert_eq!(pasteboard_change_count(board.name()), Some(copied));
        assert!(write_html_and_text(board.name(), "<pre>x</pre>", "x"));
        assert!(pasteboard_change_count(board.name()).unwrap() > copied);
    }

    #[test]
    fn one_write_puts_html_and_plain_text_on_a_private_pasteboard() {
        let board = Private::new();
        let html = "<pre style=\"color:#fff\">let <span>x</span> = 1;</pre>";
        assert_eq!(read_type(board.name(), HTML), None, "nothing before");
        assert!(!declares(board.name(), HTML) && !declares(board.name(), TEXT));
        assert!(write_html_and_text(board.name(), html, "let x = 1;"));
        assert!(declares(board.name(), HTML) && declares(board.name(), TEXT));
        assert_eq!(read_type(board.name(), HTML).as_deref(), Some(html));
        // the type gpui's paste reads, as UTF-8
        assert_eq!(read_type(board.name(), TEXT).as_deref(), Some("let x = 1;"));
    }

    #[test]
    fn a_write_replaces_whatever_was_there_in_every_type() {
        let board = Private::new();
        set_type(board.name(), "public.rtf", "{\\rtf1 old}");
        set_type(board.name(), TEXT, "old text");
        assert!(write_html_and_text(board.name(), "<pre>1</pre>", "1"));
        assert!(write_html_and_text(board.name(), "<pre>2</pre>", "2"));
        assert_eq!(read_type(board.name(), "public.rtf"), None, "cleared");
        assert_eq!(read_type(board.name(), HTML).as_deref(), Some("<pre>2</pre>"));
        assert_eq!(read_type(board.name(), TEXT).as_deref(), Some("2"));
    }

    #[test]
    fn text_with_a_nul_and_beyond_ascii_comes_back_whole() {
        let board = Private::new();
        let (html, text) = ("<pre>a\0b é 中 😀</pre>", "a\0b\nä 中 😀\0");
        assert!(write_html_and_text(board.name(), html, text));
        assert_eq!(read_type(board.name(), HTML).as_deref(), Some(html));
        assert_eq!(read_type(board.name(), TEXT).as_deref(), Some(text));
    }

    #[test]
    fn empty_text_and_a_big_page_are_written_too() {
        let board = Private::new();
        assert!(write_html_and_text(board.name(), "", ""));
        assert_eq!(read_type(board.name(), HTML).as_deref(), Some(""));
        assert_eq!(read_type(board.name(), TEXT).as_deref(), Some(""));
        let big = "<span>x</span>\n".repeat(200_000);
        assert!(write_html_and_text(board.name(), &big, &big[..big.len() / 2]));
        assert_eq!(read_type(board.name(), HTML).as_deref(), Some(big.as_str()));
        assert_eq!(read_type(board.name(), TEXT).map(|t| t.len()), Some(big.len() / 2));
    }

    #[test]
    fn the_general_pasteboard_is_found_without_being_written() {
        let board = Private::new();
        pool(|| {
            let (general, private) = (pasteboard(None), pasteboard(board.name()));
            assert!(!general.is_null() && !private.is_null());
            assert_ne!(general, private);
        });
    }
}
