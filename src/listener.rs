//! Global SHIFT+wheel / SHIFT+click listener (plan Q3: direct `CGEventTap`
//! via `core-graphics`). One tap sees scroll + mouse-down events; the SHIFT
//! state comes from each event's flags, so no separate keyboard listener.
//!
//! SHIFT+wheel functions exactly like wheeling while the mouse is on the
//! mirror screen: the original event is suppressed and a plain (no-SHIFT)
//! copy is posted straight to the scrcpy process at the cursor-or-center
//! point, so scrcpy scrolls the phone natively. Plain wheel is never
//! intercepted. Only when no mirror window exists does SHIFT+wheel fall
//! back to the accumulated NEXT/PREV adb swipes.

use crate::adb::Adb;
use crate::logic::{wheel_route, Action, Cooldown, Geometry, PixelAccumulator, WheelRoute};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::CFString;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, CallbackResult, EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowOwnerName, kCGWindowOwnerPID,
};
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ListenerConfig {
    pub adb: Adb,
    pub geo: Geometry,
    /// click-probe mode: log middle/right presses for 20s, then exit.
    pub probe: bool,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
}

/// Input Monitoring + Accessibility preflight for this binary.
pub fn permissions_ok() -> bool {
    unsafe { AXIsProcessTrusted() && CGPreflightListenEventAccess() }
}

/// (x, y, width, height) of one scrcpy window, as in `scrcpy_bounds()`.
type Bounds = (f64, f64, f64, f64);

/// One scrcpy window plus the pid that owns it (wheel forwarding target).
#[derive(Debug, Clone, Copy)]
struct MirrorTarget {
    pid: i32,
    bounds: Bounds,
}

static BOUNDS_CACHE: Mutex<Option<(Instant, Vec<MirrorTarget>)>> = Mutex::new(None);

fn lock_bounds_cache() -> MutexGuard<'static, Option<(Instant, Vec<MirrorTarget>)>> {
    BOUNDS_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks wheel events we synthesize, so they can never be re-intercepted.
const SYNTHETIC_WHEEL_MAGIC: i64 = 0x5347_5757; // "SGWW"

fn cf_type(ptr: *const c_void) -> Option<CFType> {
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { CFType::wrap_under_get_rule(ptr as CFTypeRef) })
    }
}

fn dict_value_string(dict: &CFDictionary, key: *const c_void) -> Option<String> {
    let value = dict.find(key)?;
    let s = cf_type(*value)?.downcast::<CFString>()?;
    Some(s.to_string())
}

fn dict_value_dict(dict: &CFDictionary, key: *const c_void) -> Option<CFDictionary> {
    let value = dict.find(key)?;
    cf_type(*value)?.downcast::<CFDictionary>()
}

fn dict_value_f64(dict: &CFDictionary, key: &CFString) -> f64 {
    let found = dict.find(key.as_concrete_TypeRef() as *const c_void);
    let number = found
        .and_then(|value| cf_type(*value))
        .and_then(|t| t.downcast::<CFNumber>());
    number.and_then(|n| n.to_f64()).unwrap_or(0.0)
}

fn dict_value_i32(dict: &CFDictionary, key: *const c_void) -> i32 {
    let found = dict.find(key);
    let number = found
        .and_then(|value| cf_type(*value))
        .and_then(|t| t.downcast::<CFNumber>());
    number.and_then(|n| n.to_i32()).unwrap_or(0)
}

/// Port of `scrcpy_bounds()`'s window query; empty on any failure.
fn query_scrcpy_windows() -> Vec<MirrorTarget> {
    let windows = match copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) {
        Some(w) => w,
        None => return Vec::new(),
    };
    let key_x = CFString::new("X");
    let key_y = CFString::new("Y");
    let key_w = CFString::new("Width");
    let key_h = CFString::new("Height");
    let mut out = Vec::new();
    for ptr in windows.get_all_values() {
        let dict = match cf_type(ptr).and_then(|t| t.downcast::<CFDictionary>()) {
            Some(d) => d,
            None => continue,
        };
        let owner = dict_value_string(&dict, unsafe { kCGWindowOwnerName } as *const c_void)
            .unwrap_or_default();
        if !owner.to_lowercase().contains("scrcpy") {
            continue;
        }
        let pid = dict_value_i32(&dict, unsafe { kCGWindowOwnerPID } as *const c_void);
        let bounds = match dict_value_dict(&dict, unsafe { kCGWindowBounds } as *const c_void) {
            Some(b) => b,
            None => {
                out.push(MirrorTarget {
                    pid,
                    bounds: (0.0, 0.0, 0.0, 0.0),
                });
                continue;
            }
        };
        out.push(MirrorTarget {
            pid,
            bounds: (
                dict_value_f64(&bounds, &key_x),
                dict_value_f64(&bounds, &key_y),
                dict_value_f64(&bounds, &key_w),
                dict_value_f64(&bounds, &key_h),
            ),
        });
    }
    out
}

fn scrcpy_windows() -> Vec<MirrorTarget> {
    let now = Instant::now();
    {
        let cache = lock_bounds_cache();
        if let Some((at, windows)) = &*cache {
            if now.duration_since(*at) < Duration::from_millis(500) {
                return windows.clone();
            }
        }
    }
    let windows: Vec<MirrorTarget> =
        std::panic::catch_unwind(query_scrcpy_windows).unwrap_or_default();
    *lock_bounds_cache() = Some((now, windows.clone()));
    windows
}

fn point_in_bounds(x: f64, y: f64, bounds: Bounds) -> bool {
    bounds.0 <= x && x <= bounds.0 + bounds.2 && bounds.1 <= y && y <= bounds.1 + bounds.3
}

/// True when the Mac cursor (x, y) is over the scrcpy mirror window.
/// Bounds are cached for 0.5s, as in the Python code.
pub fn scrcpy_at(x: f64, y: f64) -> bool {
    scrcpy_windows()
        .iter()
        .any(|w| point_in_bounds(x, y, w.bounds))
}

/// The mirror window under (x, y), else the first mirror window, else none.
fn mirror_target(x: f64, y: f64) -> Option<MirrorTarget> {
    let windows = scrcpy_windows();
    windows
        .iter()
        .find(|w| point_in_bounds(x, y, w.bounds))
        .or_else(|| windows.first())
        .copied()
        .filter(|w| w.pid > 0)
}

struct ListenerState {
    cfg: ListenerConfig,
    acc: PixelAccumulator,
    cooldown: Cooldown,
}

static STATE: OnceLock<Mutex<ListenerState>> = OnceLock::new();
/// Raw `CFMachPortRef` of the installed tap, for re-enabling on timeout.
static TAP_PORT: AtomicUsize = AtomicUsize::new(0);

fn lock_state() -> Option<MutexGuard<'static, ListenerState>> {
    STATE
        .get()
        .map(|m| m.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `HH:MM:SS` local time (Python `time.strftime('%H:%M:%S')`); never panics.
fn now_hms() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return "00:00:00".to_string();
        }
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Next => "NEXT",
        Action::Prev => "PREV",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseKind {
    Right,
    Other,
}

/// pynput `str(button)` for the clicks we handle; `Other` is middle (2) only.
fn button_name(kind: MouseKind, button_number: i64) -> Option<&'static str> {
    match kind {
        MouseKind::Right => Some("Button.right"),
        MouseKind::Other => {
            if button_number == 2 {
                Some("Button.middle")
            } else {
                None
            }
        }
    }
}

fn shift_held(event: &CGEvent) -> bool {
    event.get_flags().contains(CGEventFlags::CGEventFlagShift)
}

/// Integer wheel deltas for a synthetic PIXEL scroll event: point deltas
/// pass through untouched; fixed-point deltas round to the nearest pixel
/// (a nonzero sub-pixel nudge keeps its direction as ±1).
fn wheel_ints(pdx: i64, pdy: i64, fdx: f64, fdy: f64) -> (i32, i32) {
    fn one(pi: i64, fd: f64) -> i32 {
        if pi != 0 {
            return pi.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        }
        let r = fd.round();
        if r == 0.0 && fd != 0.0 {
            if fd > 0.0 {
                1
            } else {
                -1
            }
        } else {
            r.clamp(i32::MIN as f64, i32::MAX as f64) as i32
        }
    }
    (one(pdx, fdx), one(pdy, fdy))
}

/// Forward a SHIFT+wheel tick into the mirror as a plain wheel event, so
/// scrcpy scrolls the phone exactly as if the mouse were on the screen.
/// Returns false when there is no mirror window (caller falls back).
fn forward_wheel_to_mirror(event: &CGEvent, x: f64, y: f64, wdx: i32, wdy: i32) -> bool {
    let target = match mirror_target(x, y) {
        Some(t) => t,
        None => return false,
    };
    if wdx == 0 && wdy == 0 {
        return true; // consumed; nothing to forward
    }
    let (bx, by, bw, bh) = target.bounds;
    let (lx, ly) = if point_in_bounds(x, y, target.bounds) {
        (x, y)
    } else {
        (bx + bw / 2.0, by + bh / 2.0)
    };
    let source = match CGEventSource::new(CGEventSourceStateID::Private) {
        Ok(s) => s,
        Err(()) => return false,
    };
    let copy = match CGEvent::new_scroll_event(source, ScrollEventUnit::PIXEL, 2, wdy, wdx, 0) {
        Ok(e) => e,
        Err(()) => return false,
    };
    copy.set_location(CGPoint { x: lx, y: ly });
    // Plain wheel, not SHIFT+wheel: scrcpy would read SHIFT as horizontal.
    copy.set_flags(event.get_flags() - CGEventFlags::CGEventFlagShift);
    copy.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, SYNTHETIC_WHEEL_MAGIC);
    copy.post_to_pid(target.pid);
    true
}

/// SHIFT+wheel enters here; plain wheel is never intercepted, so wheeling
/// on the mirror screen stays scrcpy-native.
fn handle_scroll(event: &CGEvent) -> CallbackResult {
    if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == SYNTHETIC_WHEEL_MAGIC {
        return CallbackResult::Keep;
    }
    let loc = event.location();
    let (x, y) = (loc.x, loc.y);
    let shift = shift_held(event);
    if wheel_route(shift, false) == WheelRoute::Pass {
        return CallbackResult::Keep;
    }
    let pdx = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2);
    let pdy = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1);
    let fdx = event.get_double_value_field(EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2);
    let fdy = event.get_double_value_field(EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1);
    let (wdx, wdy) = wheel_ints(pdx, pdy, fdx, fdy);
    if forward_wheel_to_mirror(event, x, y, wdx, wdy) {
        return CallbackResult::Drop;
    }
    // No mirror window: accumulated NEXT/PREV adb swipes (listen-only mode).
    let (dx, dy) = if pdx == 0 && pdy == 0 {
        (fdx, fdy)
    } else {
        (pdx as f64, pdy as f64)
    };
    let fire = match lock_state() {
        Some(mut st) => {
            let now = Instant::now();
            match st.acc.accumulate(dx, dy, now) {
                Some(action) if st.cooldown.try_fire(now) => {
                    let cmd = st.cfg.adb.swipe_action_cmd(st.cfg.geo, action, 400);
                    Some((st.cfg.adb.clone(), cmd, action))
                }
                _ => None,
            }
        }
        None => None,
    };
    if let Some((adb, cmd, action)) = fire {
        println!("[{}] wheel -> {}", now_hms(), action_name(action));
        adb.run_detached(&cmd);
    }
    CallbackResult::Drop
}

/// Port of `on_click` (pressed only); clicks are never suppressed.
fn handle_click(kind: MouseKind, event: &CGEvent) -> CallbackResult {
    let button_number = event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER);
    let name = match button_name(kind, button_number) {
        Some(n) => n,
        None => return CallbackResult::Keep,
    };
    let loc = event.location();
    if scrcpy_at(loc.x, loc.y) {
        // Cursor over mirror: scrcpy handles the click, else double-tap.
        return CallbackResult::Keep;
    }
    let shift = shift_held(event);
    println!(
        "[{}] {} pressed shift={} -> {}",
        now_hms(),
        name,
        shift,
        if shift {
            "VIDEO pause/resume"
        } else {
            "ignored (hold SHIFT)"
        }
    );
    if shift {
        let pair = lock_state().map(|st| (st.cfg.adb.clone(), st.cfg.geo));
        if let Some((adb, geo)) = pair {
            let cmd = adb.video_cmd(geo);
            let rc = adb.run(&cmd);
            if rc == 0 {
                let target = if adb.serial.is_empty() {
                    "phone"
                } else {
                    adb.serial.as_str()
                };
                println!(
                    "[{}] sent tap {} {} to {}",
                    now_hms(),
                    geo.cx,
                    geo.tap_y,
                    target
                );
            }
        }
    }
    CallbackResult::Keep
}

fn handle_event(etype: CGEventType, event: &CGEvent) -> CallbackResult {
    match etype {
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
            let port = TAP_PORT.load(Ordering::SeqCst);
            if port != 0 {
                unsafe { CGEventTapEnable(port as *mut c_void, true) };
            }
            CallbackResult::Keep
        }
        CGEventType::ScrollWheel => handle_scroll(event),
        CGEventType::RightMouseDown => handle_click(MouseKind::Right, event),
        CGEventType::OtherMouseDown => handle_click(MouseKind::Other, event),
        _ => CallbackResult::Keep,
    }
}

/// Tap callback; a panic must never cross the FFI boundary.
fn tap_callback(_proxy: CGEventTapProxy, etype: CGEventType, event: &CGEvent) -> CallbackResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_event(etype, event))) {
        Ok(result) => result,
        Err(_) => CallbackResult::Keep,
    }
}

/// Run the tap on the current thread's CFRunLoop. Blocks forever
/// (probe mode exits the process after 20s).
pub fn run(cfg: ListenerConfig) {
    let probe = cfg.probe;
    let new_state = Mutex::new(ListenerState {
        cfg,
        acc: PixelAccumulator::new(),
        cooldown: Cooldown::new(),
    });
    if let Err(returned) = STATE.set(new_state) {
        if let Some(existing) = STATE.get() {
            let fresh = returned.into_inner().unwrap_or_else(|e| e.into_inner());
            *existing.lock().unwrap_or_else(|e| e.into_inner()) = fresh;
        }
    }

    if probe {
        println!("CLICK-PROBE: press SHIFT+middle-click and SHIFT+right-click. Logging every middle/right press for 20s...");
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(20));
            println!("PROBE DONE - if no lines appeared, macOS blocked the listener or the mouse has no middle button (use SHIFT+right-click instead).");
            std::process::exit(0);
        });
    } else {
        println!("SHIFT + wheel = same as wheeling on the mirror screen (forwarded into the mirror; NEXT/PREV swipe when no mirror). SHIFT+middle-click (or SHIFT+right-click) = VIDEO pause/resume. Ctrl+C stops.");
    }

    let tap = match CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![
            CGEventType::ScrollWheel,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
        ],
        tap_callback,
    ) {
        Ok(t) => t,
        Err(()) => {
            println!("Quartz wheel tap unavailable - fallback to line scroll (allow Accessibility for Terminal)");
            return;
        }
    };
    TAP_PORT.store(
        tap.mach_port().as_concrete_TypeRef() as usize,
        Ordering::SeqCst,
    );
    let loop_source = match tap.mach_port().create_runloop_source(0) {
        Ok(s) => s,
        Err(()) => {
            println!("Quartz wheel tap unavailable - fallback to line scroll (allow Accessibility for Terminal)");
            return;
        }
    };
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    println!("Wheel tap ON: SHIFT+wheel forwards into the mirror as an on-screen wheel | plain wheel untouched");
    CFRunLoop::run_current();
}

/// Run the listener on a background thread (mirror mode).
pub fn spawn(cfg: ListenerConfig) {
    std::thread::spawn(move || run(cfg));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrcpy_at_origin_returns_bool_without_panicking() {
        let first: bool = scrcpy_at(0.0, 0.0);
        // Second call hits the 500ms cache, so it must agree with the first.
        let second: bool = scrcpy_at(0.0, 0.0);
        assert_eq!(first, second);
    }

    #[test]
    fn button_classification() {
        assert_eq!(button_name(MouseKind::Right, 1), Some("Button.right"));
        assert_eq!(button_name(MouseKind::Other, 2), Some("Button.middle"));
        assert_eq!(button_name(MouseKind::Other, 3), None);
        assert_eq!(button_name(MouseKind::Other, 0), None);
    }

    #[test]
    fn action_names_match_python() {
        assert_eq!(action_name(Action::Next), "NEXT");
        assert_eq!(action_name(Action::Prev), "PREV");
    }

    #[test]
    fn wheel_ints_forwarding_deltas() {
        // Point deltas pass through untouched (same as on-screen wheel).
        assert_eq!(wheel_ints(0, -12, 0.0, 0.0), (0, -12));
        assert_eq!(wheel_ints(3, 0, 9.9, 9.9), (3, 10));
        // Fixed-point rounds; sub-pixel nudges keep their direction.
        assert_eq!(wheel_ints(0, 0, 0.0, -2.4), (0, -2));
        assert_eq!(wheel_ints(0, 0, 0.3, -0.2), (1, -1));
        assert_eq!(wheel_ints(0, 0, 0.0, 0.0), (0, 0));
    }

    #[test]
    fn time_format_shape() {
        let t = now_hms();
        assert_eq!(t.len(), 8);
        assert_eq!(t.as_bytes()[2], b':');
        assert_eq!(t.as_bytes()[5], b':');
    }
}
