//! Floating Settings tab for mirror mode (`screengood title-bar`,
//! spawned by the mirror process): a vertical Settings button attached
//! to the left edge of the scrcpy window, following it at display
//! rate. The mirror window belongs to scrcpy, so macOS cannot put a
//! real button in its title bar; this attached panel is the in-app
//! equivalent, and it leaves the whole title bar free for dragging.
//! The button opens the separate `screengood settings` window.

use crate::ax;
use crate::listener::{scrcpy_window_bounds_for_pid, scrcpy_windows, Bounds};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSBezelStyle, NSButton, NSCellImagePosition, NSColor, NSFloatingWindowLevel, NSFont, NSImage,
    NSPanel, NSScreen, NSWindowAnimationBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSActivityOptions, NSNotification, NSObject, NSObjectProtocol,
    NSPoint, NSProcessInfo, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::process::{Child, Command};

/// Vertical side tab centered on the mirror's left edge. It hangs
/// mostly outside the window, overlapping just enough to read as
/// attached, so it never covers phone content or the title bar.
const BAR_WIDTH: f64 = 72.0;
const BAR_HEIGHT: f64 = 104.0;
const ATTACH_OVERLAP: f64 = 10.0;
const SETTINGS_BUTTON_WIDTH: f64 = 64.0;
const SETTINGS_BUTTON_HEIGHT: f64 = 88.0;
/// Follow above display rate so dragging the mirror looks attached,
/// not stepped. Accessibility events land between these ticks too;
/// both paths read current geometry and no-op when unchanged.
const FOLLOW_INTERVAL_SECS: f64 = 1.0 / 120.0;

// Accessibility (AX) tracking (bindings in `crate::ax`): the mirror
// window reports its own moves and resizes, so the button is
// repositioned by the same event stream that moves the window
// instead of only chasing it on a poll.
type AxObserverRef = ax::ObserverRef;
type AxElementRef = ax::ElementRef;

/// AppKit frame for the side tab on a Quartz-coordinate mirror
/// window. Quartz origin is the primary display's top-left; AppKit's
/// is its bottom-left, so `primary_height` flips Y. The tab is
/// vertically centered on the left edge and clamped on-screen.
/// `None` hides it (fullscreen or too small to carry it).
fn title_bar_frame(mirror: Bounds, primary_height: f64) -> Option<NSRect> {
    let (x, y, w, h) = mirror;
    if w <= 0.0 || h <= 0.0 || primary_height <= 0.0 {
        return None;
    }
    if y <= 0.0 && h >= primary_height - 2.0 {
        return None; // fullscreen: no edge to hang the tab on
    }
    if w < BAR_WIDTH || h < BAR_HEIGHT + 20.0 {
        return None;
    }
    let center_y = primary_height - y - h / 2.0;
    Some(NSRect::new(
        NSPoint::new(
            (x - BAR_WIDTH + ATTACH_OVERLAP).max(0.0),
            center_y - BAR_HEIGHT / 2.0,
        ),
        NSSize::new(BAR_WIDTH, BAR_HEIGHT),
    ))
}

/// Largest plausible mirror window (with owner pid) from a bounds query.
fn pick_bounds(windows: Vec<(Bounds, i32)>) -> Option<(Bounds, i32)> {
    windows
        .into_iter()
        .filter(|&((_, _, w, h), _)| w > 0.0 && h > 0.0)
        .max_by(|a, b| (a.0 .2 * a.0 .3).total_cmp(&(b.0 .2 * b.0 .3)))
}

/// True when two frames already agree closely enough that moving the
/// panel again would be a wasted redraw.
fn frames_match(a: NSRect, b: NSRect) -> bool {
    let close = |u: f64, v: f64| (u - v).abs() < 0.25;
    close(a.origin.x, b.origin.x)
        && close(a.origin.y, b.origin.y)
        && close(a.size.width, b.size.width)
        && close(a.size.height, b.size.height)
}

#[derive(Debug)]
struct TitleBarIvars {
    panel: OnceCell<Retained<NSPanel>>,
    timer: OnceCell<Retained<NSTimer>>,
    settings_child: RefCell<Option<Child>>,
    /// scrcpy owner pid once identified (0 = not yet), for the fast
    /// per-tick bounds query.
    scrcpy_pid: Cell<i32>,
    /// Retained Accessibility objects as raw addresses (0 = none);
    /// they live for this short-lived helper process.
    ax_observer: Cell<usize>,
    ax_app: Cell<usize>,
    ax_window: Cell<usize>,
    ax_failed: Cell<bool>,
    /// Cached primary-display height for the Quartz/AppKit Y flip.
    primary_height: Cell<f64>,
    parent_pid: i32,
}

define_class!(
    // SAFETY:
    // - The superclass NSObject does not have any subclassing requirements.
    // - `Delegate` does not implement `Drop`.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = TitleBarIvars]
    struct Delegate;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        #[unsafe(method(titleBarTick:))]
        fn title_bar_tick(&self, _timer: &NSObject) {
            self.sync_panel();
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &NSObject) {
            self.launch_settings();
        }
    }

    // SAFETY: `NSApplicationDelegate` has no safety requirements.
    unsafe impl NSApplicationDelegate for Delegate {
        // SAFETY: The signature is correct.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            self.build_panel();
            self.sync_panel();
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, parent_pid: i32) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TitleBarIvars {
            panel: OnceCell::new(),
            timer: OnceCell::new(),
            settings_child: RefCell::new(None),
            scrcpy_pid: Cell::new(0),
            ax_observer: Cell::new(0),
            ax_app: Cell::new(0),
            ax_window: Cell::new(0),
            ax_failed: Cell::new(false),
            primary_height: Cell::new(0.0),
            parent_pid,
        });
        // SAFETY: The signature of `NSObject`'s `init` method is correct.
        unsafe { msg_send![super(this), init] }
    }

    fn quit(&self) {
        NSApplication::sharedApplication(self.mtm()).terminate(None);
    }

    /// This delegate as a target/action receiver.
    fn as_target(&self) -> &AnyObject {
        let proto: &ProtocolObject<dyn NSObjectProtocol> = ProtocolObject::from_ref(self);
        proto.as_ref()
    }

    fn build_panel(&self) {
        let mtm = self.mtm();
        let target = self.as_target();
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(BAR_WIDTH, BAR_HEIGHT)),
            NSWindowStyleMask::Borderless
                | NSWindowStyleMask::NonactivatingPanel
                | NSWindowStyleMask::UtilityWindow,
            NSBackingStoreType::Buffered,
            false,
        );
        // SAFETY: Disable auto-release when closing windows; required
        // when creating a window outside a window controller.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setLevel(NSFloatingWindowLevel);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setMovable(false);
        // Transparent around the button: the button itself is the tab.
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        // No shadow, no implicit animation: both make a dragged mirror
        // look like the button is trailing behind it.
        panel.setHasShadow(false);
        panel.setAnimationBehavior(NSWindowAnimationBehavior::None);

        let view = panel.contentView().expect("panel must have content view");
        // Settings only: no title text beside the button. The subview is
        // retained by its superview, so no ivar needs to hold it.
        // SAFETY: `target` is the delegate, which implements `openSettings:`.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Settings"),
                Some(target),
                Some(sel!(openSettings:)),
                mtm,
            )
        };
        button.setFrame(rect(
            4.0,
            8.0,
            SETTINGS_BUTTON_WIDTH,
            SETTINGS_BUTTON_HEIGHT,
        ));
        button.setBezelStyle(NSBezelStyle::FlexiblePush);
        button.setBezelColor(Some(&NSColor::controlAccentColor()));
        button.setContentTintColor(Some(&NSColor::whiteColor()));
        button.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
        if let Some(gear) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            ns_string!("gearshape.fill"),
            Some(ns_string!("Settings")),
        ) {
            button.setImage(Some(&gear));
            button.setImagePosition(NSCellImagePosition::ImageAbove);
            button.setImageHugsTitle(true);
        }
        view.addSubview(&button);

        panel.orderOut(None);
        let _ = self.ivars().panel.set(panel);

        // SAFETY: `target` is the delegate, which implements `titleBarTick:`.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                FOLLOW_INTERVAL_SECS,
                target,
                sel!(titleBarTick:),
                None,
                true,
            )
        };
        timer.setTolerance(0.0005);
        // Keep following through event-tracking loops (menus, drags),
        // not just the default run-loop mode.
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
        let _ = self.ivars().timer.set(timer);

        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }

    /// Current mirror bounds: fast known-pid query per tick, falling
    /// back to full identification until scrcpy's pid is known.
    fn current_mirror_bounds(&self) -> Option<Bounds> {
        let pid = self.ivars().scrcpy_pid.get();
        if pid > 0 {
            self.ensure_ax_observer(pid);
            if let Some(bounds) =
                scrcpy_window_bounds_for_pid(pid).filter(|&(_, _, w, h)| w > 0.0 && h > 0.0)
            {
                return Some(bounds);
            }
        }
        let (bounds, pid) = pick_bounds(scrcpy_windows())?;
        if pid > 0 {
            self.ivars().scrcpy_pid.set(pid);
            self.ensure_ax_observer(pid);
        }
        Some(bounds)
    }

    /// Register for the mirror window's own move/resize notifications
    /// once. Failure only means the 60 Hz fallback poll keeps working.
    fn ensure_ax_observer(&self, pid: i32) {
        let ivars = self.ivars();
        if ivars.ax_observer.get() != 0 || ivars.ax_failed.get() || pid <= 0 {
            return;
        }
        let observer = match ax::create_observer(pid, ax_observer_callback) {
            Ok(observer) => observer,
            Err(err) => {
                eprintln!("title bar: Accessibility tracking unavailable ({err}); using fallback");
                ivars.ax_failed.set(true);
                return;
            }
        };
        let app = ax::create_application(pid);
        if app.is_null() || !ax::add_observer_to_main_run_loop(observer) {
            eprintln!("title bar: Accessibility tracking unavailable; using fallback");
            ax::release_ptr(app as *const c_void);
            ax::release_ptr(observer as *const c_void);
            ivars.ax_failed.set(true);
            return;
        }
        ivars.ax_observer.set(observer as usize);
        ivars.ax_app.set(app as usize);
        self.add_ax_notification(app, ns_string!("AXWindowCreated"));
        self.register_largest_ax_window();
        println!("Title bar tracking: Accessibility move/resize events");
    }

    /// Track the largest current AX window of the mirror app (its main
    /// window), returning its bounds when readable.
    fn register_largest_ax_window(&self) -> Option<Bounds> {
        let app = self.ivars().ax_app.get() as AxElementRef;
        if app.is_null() {
            return None;
        }
        let windows = ax::windows(app);
        let mut best: Option<(AxElementRef, Bounds)> = None;
        for &window in &windows {
            if let Some(bounds) = ax::bounds(window) {
                let replace = match best {
                    None => true,
                    Some((_, current)) => bounds.2 * bounds.3 > current.2 * current.3,
                };
                if replace {
                    best = Some((window, bounds));
                }
            }
        }
        let picked = best.map(|(window, bounds)| {
            // register_ax_window retains the window it stores; every
            // element here then drops the +1 from ax_windows.
            self.register_ax_window(window);
            bounds
        });
        for window in windows {
            ax::release_ptr(window as *const c_void);
        }
        picked
    }

    fn add_ax_notification(&self, element: AxElementRef, notification: &'static NSString) {
        let observer = self.ivars().ax_observer.get() as AxObserverRef;
        if observer.is_null() || element.is_null() {
            return;
        }
        let refcon = self as *const Delegate as *mut c_void;
        ax::add_notification(observer, element, notification, refcon);
    }

    fn register_ax_window(&self, window: AxElementRef) {
        if window.is_null() {
            return;
        }
        let ivars = self.ivars();
        if ivars.ax_window.get() != window as usize {
            let old = ivars.ax_window.get();
            if old != 0 {
                ax::release_ptr(old as *const c_void);
            }
            ax::retain_ptr(window as *const c_void);
            ivars.ax_window.set(window as usize);
            self.add_ax_notification(window, ns_string!("AXMoved"));
            self.add_ax_notification(window, ns_string!("AXResized"));
            self.add_ax_notification(window, ns_string!("AXUIElementDestroyed"));
        }
    }

    fn handle_ax_notification(&self, element: AxElementRef, notification: CFStringRef) {
        if element.is_null() {
            return;
        }
        let name = if notification.is_null() {
            String::new()
        } else {
            unsafe { CFString::wrap_under_get_rule(notification) }.to_string()
        };
        if name == "AXWindowCreated" {
            // The callback element is the application, not the new
            // window: re-pick the largest window and follow that.
            if let Some(bounds) = self.register_largest_ax_window() {
                self.apply_bounds(bounds);
            }
            return;
        }
        if element as usize == self.ivars().ax_app.get() {
            return;
        }
        if name == "AXUIElementDestroyed" {
            if self.ivars().ax_window.get() == element as usize {
                self.ivars().ax_window.set(0);
                ax::release_ptr(element as *const c_void);
            }
            if let Some(panel) = self.ivars().panel.get() {
                if panel.isVisible() {
                    panel.orderOut(None);
                }
            }
            return;
        }
        self.register_ax_window(element);
        if let Some(bounds) = ax::bounds(element) {
            self.apply_bounds(bounds);
        }
    }

    /// Position the button from mirror bounds. Shared by the AX event
    /// path and the timer fallback; moves use `setFrameOrigin` only, so
    /// a drag does not pay for a resize/redraw on every frame.
    fn primary_height(&self) -> f64 {
        let cached = self.ivars().primary_height.get();
        if cached > 0.0 {
            return cached;
        }
        let height = primary_screen_height(self.mtm());
        if height > 0.0 {
            self.ivars().primary_height.set(height);
        }
        height
    }

    fn apply_bounds(&self, bounds: Bounds) {
        let Some(panel) = self.ivars().panel.get() else {
            return;
        };
        let Some(frame) = title_bar_frame(bounds, self.primary_height()) else {
            // Minimized, covered, or fullscreen: stay alive hidden. The
            // mirror parent ends this process when scrcpy exits.
            if panel.isVisible() {
                panel.orderOut(None);
            }
            return;
        };
        let current = panel.frame();
        if !frames_match(current, frame) {
            let same_size = (current.size.width - frame.size.width).abs() < 0.25
                && (current.size.height - frame.size.height).abs() < 0.25;
            if same_size {
                panel.setFrameOrigin(frame.origin);
            } else {
                panel.setFrame_display(frame, true);
            }
        }
        if !panel.isVisible() {
            panel.orderFrontRegardless();
        }
    }

    fn sync_panel(&self) {
        if !pid_alive(self.ivars().parent_pid) {
            self.quit();
            return;
        }
        match self.current_mirror_bounds() {
            Some(bounds) => self.apply_bounds(bounds),
            None => {
                if let Some(panel) = self.ivars().panel.get() {
                    if panel.isVisible() {
                        panel.orderOut(None);
                    }
                }
            }
        }
    }

    fn launch_settings(&self) {
        let mut child_slot = self.ivars().settings_child.borrow_mut();
        if let Some(child) = child_slot.as_mut() {
            match child.try_wait() {
                Ok(None) => return, // already open
                _ => *child_slot = None,
            }
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("screengood"));
        match Command::new(exe).arg("settings").spawn() {
            Ok(child) => *child_slot = Some(child),
            Err(e) => eprintln!("title bar: could not open settings: {e}"),
        }
    }
}

/// AXObserver entry point. `refcon` is the `Delegate`, retained by
/// `run_title_bar` for the life of this process. A panic must never
/// cross the FFI boundary.
unsafe extern "C" fn ax_observer_callback(
    _observer: AxObserverRef,
    element: AxElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    if refcon.is_null() {
        return;
    }
    let delegate = unsafe { &*(refcon as *const Delegate) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        delegate.handle_ax_notification(element, notification)
    }));
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// Height of the primary display (the one at AppKit origin 0,0), which
/// anchors the Quartz-to-AppKit Y flip in `title_bar_frame`.
fn primary_screen_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm)
        .firstObject()
        .map(|screen| screen.frame().size.height)
        .unwrap_or(0.0)
}

fn pid_alive(pid: i32) -> bool {
    pid > 1 && unsafe { libc::kill(pid, 0) } == 0
}

/// Run the floating title bar until the mirror (or our parent) goes
/// away. Spawned by mirror mode; main thread only.
pub fn run_title_bar() -> i32 {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("title-bar: must run on the main thread");
        return 1;
    };
    let parent_pid = std::env::var("SCREENGOOD_TITLEBAR_PARENT_PID")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or_else(|| unsafe { libc::getppid() });
    // Keep timers and AX notifications at full rate: an accessory
    // helper with no regular window is otherwise an App Nap candidate.
    let _activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
            | NSActivityOptions::LatencyCritical,
        &NSString::from_str("Follow ScreenGood mirror window"),
    );
    let app = NSApplication::sharedApplication(mtm);
    let delegate = Delegate::new(mtm, parent_pid);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_left_side_tab_centered_on_mirror() {
        // Mirror at Quartz (100, 50) sized 400x800, primary height 1000:
        // the tab hangs on the left edge (100 - 72 + 10 overlap) and is
        // vertically centered (AppKit center 550, origin 550 - 52).
        let frame = title_bar_frame((100.0, 50.0, 400.0, 800.0), 1000.0).unwrap();
        assert_eq!(frame.origin.x, 38.0);
        assert_eq!(frame.origin.y, 498.0);
        assert_eq!(frame.size.width, BAR_WIDTH);
        assert_eq!(frame.size.height, BAR_HEIGHT);
        assert_eq!(BAR_WIDTH, SETTINGS_BUTTON_WIDTH + 8.0);
        // At the screen's left edge the tab clamps on-screen instead
        // of hanging off it.
        let clamped = title_bar_frame((0.0, 50.0, 400.0, 800.0), 1000.0).unwrap();
        assert_eq!(clamped.origin.x, 0.0);
    }

    #[test]
    fn frames_match_skips_unchanged_moves() {
        let a = rect(384.0, 916.0, BAR_WIDTH, BAR_HEIGHT);
        assert!(frames_match(a, rect(384.1, 916.0, BAR_WIDTH, BAR_HEIGHT)));
        assert!(!frames_match(a, rect(390.0, 916.0, BAR_WIDTH, BAR_HEIGHT)));
    }

    #[test]
    fn frame_hides_for_fullscreen_and_narrow_windows() {
        assert_eq!(title_bar_frame((0.0, 0.0, 1600.0, 1000.0), 1000.0), None);
        assert_eq!(title_bar_frame((0.0, 30.0, 60.0, 100.0), 1000.0), None);
        assert_eq!(title_bar_frame((0.0, 0.0, 0.0, 0.0), 1000.0), None);
    }

    #[test]
    fn pick_bounds_takes_largest_valid_window() {
        let picked = pick_bounds(vec![
            ((0.0, 0.0, 0.0, 0.0), 111),
            ((10.0, 10.0, 300.0, 500.0), 222),
            ((20.0, 20.0, 400.0, 800.0), 333),
        ]);
        assert_eq!(picked, Some(((20.0, 20.0, 400.0, 800.0), 333)));
        assert_eq!(pick_bounds(Vec::new()), None);
    }
}
