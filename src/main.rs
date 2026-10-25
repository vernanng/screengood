//! screengood — control Facebook Reels on an Android phone from a Mac
//! mouse (Rust port of Android-Mouse.command + Reels-Scroll.command).
//!
//! Subcommands: `mirror` (default, scrcpy + listener), `listen`,
//! `test`, `click-probe`, `make-app`. Legacy `--mirror`/`--test`/
//! `--click-probe` flags are accepted as aliases. Mirror is the only
//! screen mode (pad mode was removed by user decision, 2026-10-25).

mod adb;
mod discover;
mod listener;
mod logic;

use adb::Adb;
use logic::{Action, Geometry};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("mirror");
    let code = match mode {
        "test" | "--test" => run_test(),
        "make-app" => make_app(),
        "listen" | "click-probe" | "--click-probe" => {
            run_listener_mode(mode == "click-probe" || mode == "--click-probe")
        }
        "mirror" | "--mirror" => run_mirror_mode(),
        other => {
            eprintln!("unknown mode: {other}\nusage: screengood [mirror|listen|test|click-probe|make-app]");
            2
        }
    };
    std::process::exit(code);
}

/// Port of the Python listener `--test` selftest (wheel mapping, coords
/// survive the `-s SERIAL` prefix, light scroll, mirror routing).
/// No phone needed.
fn run_test() -> i32 {
    println!("== screengood selftest ==");
    let mut ok = true;
    let geo = Geometry::default();
    let adb = Adb::new(PathBuf::from("adb"), "FAKE:1".to_string());

    for (dx, dy, exp) in [
        (0.0, -1.0, Some(Action::Next)),
        (0.0, 1.0, Some(Action::Prev)),
        (-1.0, 0.0, Some(Action::Next)),
        (1.0, 0.0, Some(Action::Prev)),
        (0.0, 0.0, None),
    ] {
        let got = logic::decide(dx, dy);
        let pass = got == exp;
        ok &= pass;
        println!(
            "  dx={dx:+} dy={dy:+} -> {got:?} (expect {exp:?}) [{}]",
            pass_str(pass)
        );
    }

    let next = adb.swipe_action_cmd(geo, Action::Next, 400);
    let prev = adb.swipe_action_cmd(geo, Action::Prev, 400);
    let tail = |c: &[String]| c[c.len() - 5..c.len() - 1].to_vec();
    let cx = geo.cx.to_string();
    let pass = tail(&next)
        == vec![
            cx.clone(),
            geo.swipe_bot.to_string(),
            cx.clone(),
            geo.swipe_top.to_string(),
        ];
    ok &= pass;
    println!(
        "  NEXT coords bottom->top survive -s prefix [{}]",
        pass_str(pass)
    );
    let pass = tail(&prev)
        == vec![
            cx.clone(),
            geo.swipe_top.to_string(),
            cx.clone(),
            geo.swipe_bot.to_string(),
        ];
    ok &= pass;
    println!(
        "  PREV coords top->bottom survive -s prefix [{}]",
        pass_str(pass)
    );
    let pass = geo.swipe_bot > geo.swipe_top
        && geo.swipe_top > 0
        && geo.tap_y > 0
        && geo.tap_y < geo.ph
        && geo.cx > 0
        && geo.cx < geo.pw;
    ok &= pass;
    println!(
        "  points on screen ({}x{}) [{}]",
        geo.pw,
        geo.ph,
        pass_str(pass)
    );
    let video = adb.video_cmd(geo);
    let pass = video[video.len() - 3..] == ["tap", cx.as_str(), geo.tap_y.to_string().as_str()];
    ok &= pass;
    println!("  video pause/resume tap center [{}]", pass_str(pass));

    // Light scroll: three tiny -2px ticks add up to one NEXT.
    let mut acc = logic::PixelAccumulator::new();
    let t0 = std::time::Instant::now();
    let acts: Vec<Option<Action>> = (0..3)
        .map(|i| acc.accumulate(0.0, -2.0, t0 + std::time::Duration::from_millis(50 * i)))
        .collect();
    let pass = acts == vec![None, None, Some(Action::Next)];
    ok &= pass;
    println!(
        "  light scroll 3x-2px -> NEXT [{pass} acts={acts:?}]",
        pass = pass_str(pass)
    );

    // Mirror routing: SHIFT anywhere + plain wheel on mirror consume; elsewhere passes.
    let pass = logic::wheel_route(true, false) == logic::WheelRoute::Consume
        && logic::wheel_route(false, true) == logic::WheelRoute::Consume
        && logic::wheel_route(false, false) == logic::WheelRoute::Pass;
    ok &= pass;
    println!("  wheel route shift/mirror/elsewhere [{}]", pass_str(pass));

    println!(
        "SELFTEST {}",
        if ok {
            "OK - DOWN=NEXT, UP=PREV"
        } else {
            "FAILED"
        }
    );
    if ok {
        0
    } else {
        1
    }
}

fn pass_str(p: bool) -> &'static str {
    if p {
        "PASS"
    } else {
        "FAIL"
    }
}

fn ensure_tool(bin: &Path, brew_pkg: &str) {
    if bin.is_absolute() && !bin.exists() {
        println!("Installing {brew_pkg} via Homebrew...");
        let _ = Command::new("brew").args(["install", brew_pkg]).status();
    }
}

fn open_permission_panes() {
    for pane in [
        "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
    ] {
        let _ = Command::new("open").arg(pane).status();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Start the SHIFT+wheel listener in the background if permissions allow;
/// the mirror keeps working without it.
fn spawn_listener_if_allowed(phone: &discover::Phone) {
    if listener::permissions_ok() {
        listener::spawn(listener::ListenerConfig {
            adb: phone.adb.clone(),
            geo: phone.geo,
            probe: false,
        });
    } else {
        println!(
            "macOS blocked the SHIFT+wheel listener (Input Monitoring). Pad/mirror still work."
        );
        println!("Allow screengood in Settings > Privacy > Input Monitoring + Accessibility, then reopen.");
        open_permission_panes();
    }
}

fn run_listener_mode(probe: bool) -> i32 {
    let tools = discover::resolve_tools();
    ensure_tool(&tools.adb_bin, "android-platform-tools");
    if probe {
        // Probe mirrors the Python one: log clicks for 20s without a phone action.
        let serial = Adb::new(tools.adb_bin.clone(), String::new())
            .run_capture(&["devices"])
            .map(|o| discover::pick_serial(&discover::parse_devices(&o)).unwrap_or_default())
            .unwrap_or_default();
        println!("Phone ({serial}). Probe logging clicks for 20s - try SHIFT+middle-click now.");
        listener::run(listener::ListenerConfig {
            adb: Adb::new(tools.adb_bin, serial),
            geo: Geometry::default(),
            probe: true,
        });
        return 0;
    }
    println!("==============================================");
    println!(" Reels SHIFT + WHEEL -> Samsung TV");
    println!(
        " SHIFT + wheel DOWN = NEXT | UP = PREV | SHIFT + middle/right-click = VIDEO pause/resume"
    );
    println!("==============================================");
    let phone = discover::connect_phone(&tools);
    println!("\nPhone OK ({}).\n", phone.adb.serial);
    if !listener::permissions_ok() {
        println!("*** macOS blocked the wheel listener. Fix (1 min, once): ***");
        println!("Allow screengood in Settings > Privacy > Input Monitoring and Accessibility, then reopen.");
        open_permission_panes();
        return 1;
    }
    println!("Permission OK. Hold SHIFT + turn wheel. Ctrl+C to stop.");
    listener::run(listener::ListenerConfig {
        adb: phone.adb,
        geo: phone.geo,
        probe: false,
    });
    0
}

fn run_mirror_mode() -> i32 {
    let tools = discover::resolve_tools();
    ensure_tool(&tools.adb_bin, "android-platform-tools");
    ensure_tool(&tools.scrcpy_bin, "scrcpy");
    let phone = discover::connect_phone(&tools);
    let _ = phone.adb.run(&phone.adb.keyevent_cmd(224)); // wake phone screen
    spawn_listener_if_allowed(&phone);

    let serial = phone.adb.serial.clone();
    let (profile, args): (&str, Vec<&str>) = if discover::is_usb_serial(&serial) {
        (
            "USB near-zero: 664x1440 H.265 12M 60fps | video buffer 0 | raw audio buffer 30ms",
            vec![
                "--max-size",
                "1440",
                "--video-codec=h265",
                "--video-bit-rate=12M",
                "--max-fps=60",
                "--video-buffer=0",
                "--audio-codec=raw",
                "--audio-buffer=30",
                "--render-driver=metal",
            ],
        )
    } else {
        (
            "WiFi smooth: 414x900 H.264 | buffer video/audio 100ms",
            vec![
                "--max-size",
                "900",
                "--video-buffer=100",
                "--audio-buffer=100",
            ],
        )
    };
    println!("==============================================");
    println!(" MIRROR MODE (scrcpy): PHONE SCREEN on Mac -> {serial}");
    println!(" click=TAP | drag=swipe | wheel=NEXT/PREV | right-click=Back (inside mirror)");
    println!(" ANYWHERE: light SHIFT+wheel=NEXT/PREV | SHIFT+mid/right=pause");
    println!(" Profile: {profile}");
    println!("==============================================");
    let status = Command::new(&tools.scrcpy_bin)
        .args(["-s", &serial, "--stay-awake"])
        .args(&args)
        .args([
            "--window-title",
            "Android - click=tap, light SHIFT+wheel=NEXT/PREV, SHIFT+click=tap",
        ])
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("scrcpy failed to start: {e}");
            1
        }
    }
}

/// Assemble `ScreenGood.app` next to the project dir: copy this binary in,
/// write Info.plist, ad-hoc codesign with a stable identifier so TCC
/// (Input Monitoring / Accessibility) grants survive rebuilds.
fn make_app() -> i32 {
    let dir = discover::project_dir();
    let app = dir.join("ScreenGood.app");
    let macos = app.join("Contents").join("MacOS");
    if let Err(e) = std::fs::create_dir_all(&macos) {
        eprintln!("make-app: {e}");
        return 1;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("screengood"));
    let dest = macos.join("screengood");
    if exe != dest {
        if let Err(e) = std::fs::copy(&exe, &dest) {
            eprintln!("make-app copy: {e}");
            return 1;
        }
    }
    let plist = app.join("Contents").join("Info.plist");
    let plist_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>screengood</string>
    <key>CFBundleIdentifier</key><string>com.screengood.app</string>
    <key>CFBundleName</key><string>ScreenGood</string>
    <key>CFBundleDisplayName</key><string>ScreenGood</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
"#;
    if let Err(e) = std::fs::write(&plist, plist_body) {
        eprintln!("make-app plist: {e}");
        return 1;
    }
    let signed = Command::new("codesign")
        .args([
            "--force",
            "--sign",
            "-",
            "--identifier",
            "com.screengood.app",
            app.to_str().unwrap_or_default(),
        ])
        .status();
    match signed {
        Ok(s) if s.success() => {
            println!(
                "ScreenGood.app ready at {} (double-click = mirror)",
                app.display()
            );
            0
        }
        Ok(s) => {
            eprintln!("codesign failed: {s}");
            1
        }
        Err(e) => {
            eprintln!("codesign: {e}");
            1
        }
    }
}
