//! Pasting into the app in front, as Raycast does after a pick: a synthetic ⌘V. Posting key
//! events needs the Accessibility permission (System Settings › Privacy & Security ›
//! Accessibility); the first try asks for it, and until it's granted the pick is only copied.

use std::ffi::c_void;

type CFTypeRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFTypeRef;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn CGEventCreateKeyboardEvent(source: CFTypeRef, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize,
                          key_cb: *const c_void, value_cb: *const c_void) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
}

/// `kVK_ANSI_V`, `kCGEventFlagMaskCommand`, `kCGHIDEventTap`.
const KEY_V: u16 = 9;
const FLAG_COMMAND: u64 = 1 << 20;
const HID_TAP: u32 = 0;

/// Whether tomboy may post key events; `prompt` shows the system's "allow" dialog (once per
/// process, as macOS decides) and lists tomboy under Accessibility.
pub fn trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt { return AXIsProcessTrustedWithOptions(std::ptr::null()); }
        let (keys, values) = ([kAXTrustedCheckOptionPrompt], [kCFBooleanTrue]);
        let opts = CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), 1,
            &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
        let ok = AXIsProcessTrustedWithOptions(opts);
        if !opts.is_null() { CFRelease(opts); }
        ok
    }
}

/// Press ⌘V in whatever app has the keyboard, after `delay` (so the launcher's panel has
/// handed it back). Returns at once; false when the permission is missing.
pub fn paste_soon(delay: std::time::Duration) -> bool {
    if !trusted(true) { return false; }
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        unsafe {
            for down in [true, false] {
                let ev = CGEventCreateKeyboardEvent(std::ptr::null(), KEY_V, down);
                if ev.is_null() { return; }
                CGEventSetFlags(ev, FLAG_COMMAND);
                CGEventPost(HID_TAP, ev);
                CFRelease(ev);
            }
        }
    });
    true
}
