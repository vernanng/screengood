//! Native Settings window for `screengood settings`: three sliders for
//! scroll feel (step, cooldown, swipe duration), saved via
//! `crate::settings` and applied on next launch. Runs its own
//! `NSApplication` on the main thread; it never touches the phone, so
//! it can be opened while a mirror is running. AppKit via `objc2`,
//! following the upstream `hello_world_app` delegate pattern.

use crate::settings::{
    format_step, save_to, ScrollSettings, COOLDOWN_MAX_MS, COOLDOWN_MIN_MS, STEP_MAX_PX,
    STEP_MIN_PX, SWIPE_MAX_MS, SWIPE_MIN_MS,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSButton, NSSlider, NSTextField, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString,
};
use std::cell::OnceCell;
use std::path::PathBuf;

#[derive(Debug)]
struct SettingsIvars {
    window: OnceCell<Retained<NSWindow>>,
    step_slider: OnceCell<Retained<NSSlider>>,
    cooldown_slider: OnceCell<Retained<NSSlider>>,
    swipe_slider: OnceCell<Retained<NSSlider>>,
    step_value: OnceCell<Retained<NSTextField>>,
    cooldown_value: OnceCell<Retained<NSTextField>>,
    swipe_value: OnceCell<Retained<NSTextField>>,
    status: OnceCell<Retained<NSTextField>>,
    initial: ScrollSettings,
    path: PathBuf,
}

define_class!(
    // SAFETY:
    // - The superclass NSObject does not have any subclassing requirements.
    // - `Delegate` does not implement `Drop`.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = SettingsIvars]
    struct Delegate;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        #[unsafe(method(sliderChanged:))]
        fn slider_changed(&self, _sender: &NSObject) {
            self.refresh_values();
        }

        #[unsafe(method(saveSettings:))]
        fn save_settings(&self, _sender: &NSObject) {
            let settings = self.current_settings();
            match save_to(&settings, &self.ivars().path) {
                Ok(()) => {
                    println!(
                        "Saved scroll settings ({}) to {} - applies on next launch.",
                        settings.summary(),
                        self.ivars().path.display()
                    );
                    self.quit();
                }
                Err(e) => self.set_status(&format!("Could not save: {e}")),
            }
        }

        #[unsafe(method(cancelSettings:))]
        fn cancel_settings(&self, _sender: &NSObject) {
            self.quit();
        }

        #[unsafe(method(resetDefaults:))]
        fn reset_defaults(&self, _sender: &NSObject) {
            let d = ScrollSettings::default();
            if let Some(s) = self.ivars().step_slider.get() {
                s.setDoubleValue(d.scroll_step_px);
            }
            if let Some(s) = self.ivars().cooldown_slider.get() {
                s.setDoubleValue(d.cooldown_ms as f64);
            }
            if let Some(s) = self.ivars().swipe_slider.get() {
                s.setDoubleValue(d.swipe_ms as f64);
            }
            self.refresh_values();
        }
    }

    // SAFETY: `NSApplicationDelegate` has no safety requirements.
    unsafe impl NSApplicationDelegate for Delegate {
        // SAFETY: The signature is correct.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            self.build_window();
        }
    }

    // SAFETY: `NSWindowDelegate` has no safety requirements.
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            // Quit the application when the window is closed.
            self.quit();
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, initial: ScrollSettings, path: PathBuf) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SettingsIvars {
            window: OnceCell::new(),
            step_slider: OnceCell::new(),
            cooldown_slider: OnceCell::new(),
            swipe_slider: OnceCell::new(),
            step_value: OnceCell::new(),
            cooldown_value: OnceCell::new(),
            swipe_value: OnceCell::new(),
            status: OnceCell::new(),
            initial,
            path,
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

    fn current_settings(&self) -> ScrollSettings {
        let ivars = self.ivars();
        let get = |cell: &OnceCell<Retained<NSSlider>>, fallback: f64| {
            cell.get().map(|s| s.doubleValue()).unwrap_or(fallback)
        };
        ScrollSettings {
            scroll_step_px: get(&ivars.step_slider, ivars.initial.scroll_step_px),
            cooldown_ms: get(&ivars.cooldown_slider, ivars.initial.cooldown_ms as f64) as u64,
            swipe_ms: get(&ivars.swipe_slider, ivars.initial.swipe_ms as f64) as u32,
        }
        .validated()
    }

    fn refresh_values(&self) {
        let s = self.current_settings();
        let ivars = self.ivars();
        if let Some(v) = ivars.step_value.get() {
            v.setStringValue(&NSString::from_str(&format!(
                "{} px",
                format_step(s.scroll_step_px)
            )));
        }
        if let Some(v) = ivars.cooldown_value.get() {
            v.setStringValue(&NSString::from_str(&format!("{} ms", s.cooldown_ms)));
        }
        if let Some(v) = ivars.swipe_value.get() {
            v.setStringValue(&NSString::from_str(&format!("{} ms", s.swipe_ms)));
        }
    }

    fn set_status(&self, text: &str) {
        if let Some(v) = self.ivars().status.get() {
            v.setStringValue(&NSString::from_str(text));
        }
    }

    fn build_window(&self) {
        let mtm = self.mtm();
        let ivars = self.ivars();
        let initial = ivars.initial;
        let target = self.as_target();

        // SAFETY: We disable releasing when closed below.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 300.0)),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: Disable auto-release when closing windows; required
        // when creating an `NSWindow` outside a window controller.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(ns_string!("ScreenGood Settings"));
        let view = window.contentView().expect("window must have content view");

        add_label(
            &view,
            mtm,
            "Scroll feel - changes apply on next launch.",
            rect(20.0, 268.0, 440.0, 20.0),
        );
        let (step_slider, step_value) = add_row(
            &view,
            mtm,
            target,
            "Scroll step",
            "Wheel movement per up/down step. Lower = more sensitive.",
            initial.scroll_step_px,
            STEP_MIN_PX,
            STEP_MAX_PX,
            232.0,
        );
        let (cooldown_slider, cooldown_value) = add_row(
            &view,
            mtm,
            target,
            "Cooldown",
            "Pause between steps. Higher = fewer accidental double steps.",
            initial.cooldown_ms as f64,
            COOLDOWN_MIN_MS as f64,
            COOLDOWN_MAX_MS as f64,
            168.0,
        );
        let (swipe_slider, swipe_value) = add_row(
            &view,
            mtm,
            target,
            "Swipe duration",
            "Phone swipe duration per step. Lower = faster flick.",
            initial.swipe_ms as f64,
            SWIPE_MIN_MS as f64,
            SWIPE_MAX_MS as f64,
            104.0,
        );
        let _ = ivars.step_slider.set(step_slider);
        let _ = ivars.cooldown_slider.set(cooldown_slider);
        let _ = ivars.swipe_slider.set(swipe_slider);
        let _ = ivars.step_value.set(step_value);
        let _ = ivars.cooldown_value.set(cooldown_value);
        let _ = ivars.swipe_value.set(swipe_value);

        let status = add_label(
            &view,
            mtm,
            &format!("Saved to {}", display_path(&ivars.path)),
            rect(20.0, 52.0, 440.0, 18.0),
        );
        let _ = ivars.status.set(status);
        add_button(&view, mtm, target, "Defaults", sel!(resetDefaults:), 20.0);
        add_button(&view, mtm, target, "Cancel", sel!(cancelSettings:), 252.0);
        add_button(&view, mtm, target, "Save", sel!(saveSettings:), 360.0);

        window.setDelegate(Some(ProtocolObject::from_ref(self)));
        window.center();
        window.makeKeyAndOrderFront(None);
        let _ = ivars.window.set(window);
        self.refresh_values();

        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        // Activate the application; required when launching unbundled.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn add_label(
    view: &NSView,
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    view.addSubview(&label);
    label
}

/// One settings row (name, slider, value, hint); returns slider + value.
#[allow(clippy::too_many_arguments)]
fn add_row(
    view: &NSView,
    mtm: MainThreadMarker,
    target: &AnyObject,
    name: &str,
    hint: &str,
    value: f64,
    min: f64,
    max: f64,
    y: f64,
) -> (Retained<NSSlider>, Retained<NSTextField>) {
    add_label(view, mtm, name, rect(20.0, y + 2.0, 150.0, 20.0));
    // SAFETY: `target` is the delegate, which implements `sliderChanged:`.
    let slider = unsafe {
        NSSlider::sliderWithValue_minValue_maxValue_target_action(
            value,
            min,
            max,
            Some(target),
            Some(sel!(sliderChanged:)),
            mtm,
        )
    };
    slider.setFrame(rect(178.0, y, 204.0, 24.0));
    view.addSubview(&slider);
    let value_label = add_label(view, mtm, "", rect(394.0, y + 2.0, 66.0, 20.0));
    add_label(view, mtm, hint, rect(20.0, y - 20.0, 440.0, 16.0));
    (slider, value_label)
}

fn add_button(
    view: &NSView,
    mtm: MainThreadMarker,
    target: &AnyObject,
    title: &str,
    action: Sel,
    x: f64,
) {
    // SAFETY: `target` is the delegate, which implements `action`.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setFrame(rect(x, 16.0, 100.0, 28.0));
    view.addSubview(&button);
}

/// `~/...` form of `path` when it lives under the home dir.
fn display_path(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = shown.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    shown
}

/// Open the Settings window and run until it closes. Main thread only.
pub fn run_settings_window() -> i32 {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("settings: must run on the main thread");
        return 1;
    };
    let path = crate::settings::settings_path();
    let initial = crate::settings::load_from(&path);
    let app = NSApplication::sharedApplication(mtm);
    let delegate = Delegate::new(mtm, initial, path);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    0
}
