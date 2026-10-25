//! Global SHIFT+wheel / SHIFT+click listener: a 1:1 port of the Quartz
//! `CGEventTap` in `fb-reels-scroll.py` (plan Q3: direct tap via
//! `core-graphics`). One tap sees scroll + mouse-down events; the SHIFT
//! state comes from each event's flags, so no separate keyboard listener.

use crate::adb::Adb;
use crate::logic::{
    wheel_route, Action, Cooldown, Geometry, PixelAccumulator, WheelRoute, PIXEL_THRESHOLD,
};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::CFString;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, CallbackResult, EventField,
};
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerName, kCGWindowOwnerPID,
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

static BOUNDS_CACHE: Mutex<Option<(Instant, Vec<Bounds>)>> = Mutex::new(None);

fn lock_bounds_cache() -> MutexGuard<'static, Option<(Instant, Vec<Bounds>)>> {
    BOUNDS_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

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
    dict.find(key)
        .and_then(|value| cf_type(*value))
        .and_then(|t| t.downcast::<CFNumber>())
        .and_then(|n| n.to_i32())
        .unwrap_or(0)
}

fn process_basename(pid: i32) -> String {
    if pid <= 0 {
        return String::new();
    }
    let mut buf = vec![0u8; 4096];
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if len <= 0 {
        return String::new();
    }
    buf.truncate(len as usize);
    let path = String::from_utf8_lossy(&buf);
    path.rsplit('/').next().unwrap_or_default().to_lowercase()
}

fn is_scrcpy_window(dict: &CFDictionary) -> bool {
    let owner = dict_value_string(dict, unsafe { kCGWindowOwnerName } as *const c_void)
        .unwrap_or_default()
        .to_lowercase();
    if owner.contains("scrcpy") {
        return true;
    }
    let title = dict_value_string(dict, unsafe { kCGWindowName } as *const c_void)
        .unwrap_or_default()
        .to_lowercase();
    if title.contains("scrcpy") || title.contains("android - click=tap") {
        return true;
    }
    let pid = dict_value_i32(dict, unsafe { kCGWindowOwnerPID } as *const c_void);
    process_basename(pid) == "scrcpy"
}

fn point_in_bounds(x: f64, y: f64, bounds: Bounds) -> bool {
    let (bx, by, bw, bh) = bounds;
    bx <= x && x <= bx + bw && by <= y && y <= by + bh
}

/// Port of `scrcpy_bounds()`'s window query; empty on any failure.
fn query_scrcpy_bounds() -> Vec<Bounds> {
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
        if !is_scrcpy_window(&dict) {
            continue;
        }
        let bounds = match dict_value_dict(&dict, unsafe { kCGWindowBounds } as *const c_void) {
            Some(b) => b,
            None => {
                out.push((0.0, 0.0, 0.0, 0.0));
                continue;
            }
        };
        out.push((
            dict_value_f64(&bounds, &key_x),
            dict_value_f64(&bounds, &key_y),
            dict_value_f64(&bounds, &key_w),
            dict_value_f64(&bounds, &key_h),
        ));
    }
    out
}

fn scrcpy_bounds() -> Vec<Bounds> {
    let now = Instant::now();
    {
        let cache = lock_bounds_cache();
        if let Some((at, bounds)) = &*cache {
            if now.duration_since(*at) < Duration::from_millis(500) {
                return bounds.clone();
            }
        }
    }
    let bounds: Vec<(f64, f64, f64, f64)> =
        std::panic::catch_unwind(query_scrcpy_bounds).unwrap_or_default();
    *lock_bounds_cache() = Some((now, bounds.clone()));
    bounds
}

/// True when the Mac cursor (x, y) is over the scrcpy mirror window.
/// Bounds are cached for 0.5s, as in the Python code.
pub fn scrcpy_at(x: f64, y: f64) -> bool {
    scrcpy_bounds()
        .into_iter()
        .any(|bounds| point_in_bounds(x, y, bounds))
}

/// Normalize every wheel source before the one shared reel-step path.
/// Discrete mice may report only line deltas; scale one line to the exact
/// pixel threshold so one notch becomes one NEXT/PREV step.
fn scroll_deltas(
    point_dx: i64,
    point_dy: i64,
    fixed_dx: f64,
    fixed_dy: f64,
    line_dx: i64,
    line_dy: i64,
) -> (f64, f64) {
    if point_dx != 0 || point_dy != 0 {
        (point_dx as f64, point_dy as f64)
    } else if fixed_dx != 0.0 || fixed_dy != 0.0 {
        (fixed_dx, fixed_dy)
    } else {
        (
            line_dx as f64 * PIXEL_THRESHOLD,
            line_dy as f64 * PIXEL_THRESHOLD,
        )
    }
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

/// Port of the scroll-wheel half of `_quartz_cb` + `wheel_decision`.
fn handle_scroll(event: &CGEvent) -> CallbackResult {
    let loc = event.location();
    let (x, y) = (loc.x, loc.y);
    let shift = shift_held(event);
    let pdx = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2);
    let pdy = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1);
    let fdx = event.get_double_value_field(EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2);
    let fdy = event.get_double_value_field(EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1);
    let ldx = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2);
    let ldy = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1);
    // SHIFT+wheel and wheeling on the mirror deliberately share this exact
    // normalization, accumulator, cooldown, and adb swipe below.
    let (dx, dy) = scroll_deltas(pdx, pdy, fdx, fdy, ldx, ldy);
    match wheel_route(shift, scrcpy_at(x, y)) {
        WheelRoute::Pass => CallbackResult::Keep,
        WheelRoute::Consume => {
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
    }
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
        println!("Hold SHIFT + wheel DOWN = NEXT | UP = PREV (light scroll ok) | SHIFT+middle-click (or SHIFT+right-click) = VIDEO pause/resume. Plain wheel while the cursor is on the mirror screen = NEXT/PREV too. Ctrl+C stops.");
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
    println!("Wheel tap ON: light SHIFT+wheel = NEXT/PREV | wheel on mirror screen = NEXT/PREV");
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
    fn scroll_deltas_cover_point_fixed_and_line_wheels() {
        assert_eq!(scroll_deltas(2, 0, 9.0, 9.0, 9, 9), (2.0, 0.0));
        assert_eq!(scroll_deltas(0, 0, -0.25, 0.0, 9, 9), (-0.25, 0.0));
        assert_eq!(scroll_deltas(0, 0, 0.0, 0.0, 0, -1), (0.0, -6.0));
        assert_eq!(scroll_deltas(0, 0, 0.0, 0.0, 2, 0), (12.0, 0.0));
    }

    #[test]
    fn mirror_bounds_include_edges() {
        let bounds = (10.0, 20.0, 100.0, 200.0);
        assert!(point_in_bounds(10.0, 20.0, bounds));
        assert!(point_in_bounds(110.0, 220.0, bounds));
        assert!(!point_in_bounds(110.1, 220.0, bounds));
        assert!(!point_in_bounds(50.0, 19.9, bounds));
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
    fn time_format_shape() {
        let t = now_hms();
        assert_eq!(t.len(), 8);
        assert_eq!(t.as_bytes()[2], b':');
        assert_eq!(t.as_bytes()[5], b':');
    }
}
