//! Minimal Accessibility (AX) bindings for following another app's
//! window: create an observer for its move/resize notifications and
//! read window geometry. Raw CoreFoundation/ApplicationServices FFI
//! lives here so `title_bar` only deals in higher-level calls.
//! Pointers returned by `windows` are retained (+1) for the caller.

use core_foundation::base::TCFType;
use core_foundation::runloop::kCFRunLoopCommonModes;
use core_foundation::string::{CFString, CFStringRef};
use objc2_foundation::{ns_string, NSString};
use std::ffi::c_void;

pub type ObserverRef = *mut c_void;
pub type ElementRef = *mut c_void;
pub type ObserverCallback = unsafe extern "C" fn(ObserverRef, ElementRef, CFStringRef, *mut c_void);

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXObserverCreate(
        application: i32,
        callback: ObserverCallback,
        out_observer: *mut ObserverRef,
    ) -> i32;
    fn AXObserverAddNotification(
        observer: ObserverRef,
        element: ElementRef,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverGetRunLoopSource(observer: ObserverRef) -> *mut c_void;
    fn AXUIElementCreateApplication(pid: i32) -> ElementRef;
    fn AXUIElementCopyAttributeValue(
        element: ElementRef,
        attribute: CFStringRef,
        value: *mut *const c_void,
    ) -> i32;
    fn AXValueGetValue(value: *const c_void, the_type: u32, value_ptr: *mut c_void) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: CFStringRef);
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct AxPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct AxSize {
    width: f64,
    height: f64,
}

fn constant(name: &'static NSString) -> CFStringRef {
    name as *const NSString as CFStringRef
}

pub fn retain_ptr(ptr: *const c_void) {
    if !ptr.is_null() {
        unsafe { CFRetain(ptr) };
    }
}

pub fn release_ptr(ptr: *const c_void) {
    if !ptr.is_null() {
        unsafe { CFRelease(ptr) };
    }
}

/// Create an observer for `pid`; on error returns the AX error code.
pub fn create_observer(pid: i32, callback: ObserverCallback) -> Result<ObserverRef, i32> {
    let mut observer: ObserverRef = std::ptr::null_mut();
    let err = unsafe { AXObserverCreate(pid, callback, &mut observer) };
    if err == 0 && !observer.is_null() {
        Ok(observer)
    } else {
        Err(err)
    }
}

/// Application element for `pid` (retained), or null on failure.
pub fn create_application(pid: i32) -> ElementRef {
    unsafe { AXUIElementCreateApplication(pid) }
}

/// Add the observer's source to the main run loop (common modes, so
/// notifications keep arriving during event tracking).
pub fn add_observer_to_main_run_loop(observer: ObserverRef) -> bool {
    if observer.is_null() {
        return false;
    }
    let source = unsafe { AXObserverGetRunLoopSource(observer) };
    if source.is_null() {
        return false;
    }
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    }
    true
}

pub fn add_notification(
    observer: ObserverRef,
    element: ElementRef,
    notification: &'static NSString,
    refcon: *mut c_void,
) {
    if observer.is_null() || element.is_null() {
        return;
    }
    unsafe {
        AXObserverAddNotification(observer, element, constant(notification), refcon);
    }
}

fn copy_attribute(element: ElementRef, attribute: &'static NSString) -> Option<*const c_void> {
    if element.is_null() {
        return None;
    }
    let mut value: *const c_void = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, constant(attribute), &mut value) };
    (err == 0 && !value.is_null()).then_some(value)
}

fn point(element: ElementRef) -> Option<(f64, f64)> {
    let value = copy_attribute(element, ns_string!("AXPosition"))?;
    let mut point = AxPoint::default();
    let ok = unsafe { AXValueGetValue(value, 1, &mut point as *mut AxPoint as *mut c_void) };
    release_ptr(value);
    (ok != 0).then_some((point.x, point.y))
}

fn size(element: ElementRef) -> Option<(f64, f64)> {
    let value = copy_attribute(element, ns_string!("AXSize"))?;
    let mut size = AxSize::default();
    let ok = unsafe { AXValueGetValue(value, 2, &mut size as *mut AxSize as *mut c_void) };
    release_ptr(value);
    (ok != 0).then_some((size.width, size.height))
}

/// Window title (`AXTitle`), when readable. Distinguishes the mirror
/// window from auxiliary windows of the same app (its About panel).
pub fn title(element: ElementRef) -> Option<String> {
    let value = copy_attribute(element, ns_string!("AXTitle"))?;
    // `copy_attribute` hands back a +1 CFString; the create rule takes
    // ownership of that retain.
    let text = unsafe { CFString::wrap_under_create_rule(value as CFStringRef) };
    Some(text.to_string())
}

/// `(x, y, width, height)` in global top-left coordinates.
pub fn bounds(element: ElementRef) -> Option<(f64, f64, f64, f64)> {
    let (x, y) = point(element)?;
    let (w, h) = size(element)?;
    (w > 0.0 && h > 0.0).then_some((x, y, w, h))
}

/// The app's windows, each retained (+1) for the caller. The array
/// itself is released before returning, so retaining here is what
/// keeps the elements alive for the caller's attribute reads.
pub fn windows(app: ElementRef) -> Vec<ElementRef> {
    let Some(array) = copy_attribute(app, ns_string!("AXWindows")) else {
        return Vec::new();
    };
    let count = unsafe { CFArrayGetCount(array) };
    let mut out = Vec::new();
    for i in 0..count.max(0) {
        let item = unsafe { CFArrayGetValueAtIndex(array, i) };
        if !item.is_null() {
            retain_ptr(item);
            out.push(item as ElementRef);
        }
    }
    release_ptr(array);
    out
}
