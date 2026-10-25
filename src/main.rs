//! screengood — mirror an Android phone inside a Mac window and control
//! the entire phone with the Mac mouse (Wi-Fi or USB-C).
//!
//! Subcommands: `mirror` (default, scrcpy + listener), `listen`,
//! `test`, `click-probe`, `make-app`, `install`, `setup-signing`. Legacy `--mirror`/
//! `--test`/`--click-probe` flags are accepted as aliases. Mirror is the
//! only screen mode (pad mode was removed by user decision, 2026-10-25).

mod adb;
mod discover;
mod listener;
mod logic;

use adb::Adb;
use logic::{Action, Geometry};
use std::path::{Path, PathBuf};
use std::process::Command;

/// GUI launches (Finder double-click) have stdout/stderr wired to
/// /dev/null, which made startup failures completely invisible. When
/// stdout is not a terminal, move both streams to
/// ~/Library/Logs/ScreenGood.log so screengood and scrcpy errors survive.
fn redirect_output_to_log_if_gui() {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let path = format!("{home}/Library/Logs/ScreenGood.log");
    let Ok(cpath) = std::ffi::CString::new(path) else {
        return;
    };
    unsafe {
        let fd = libc::open(
            cpath.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
            0o644,
        );
        if fd >= 0 {
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 {
                libc::close(fd);
            }
        }
    }
    println!(
        "\n===== screengood start pid={} args={:?} =====",
        std::process::id(),
        std::env::args().skip(1).collect::<Vec<_>>()
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("mirror");
    if matches!(
        mode,
        "mirror"
            | "--mirror"
            | "listen"
            | "click-probe"
            | "--click-probe"
            | "wheel-probe"
            | "--wheel-probe"
    ) {
        redirect_output_to_log_if_gui();
    }
    let code = match mode {
        "test" | "--test" => run_test(),
        "make-app" => make_app(),
        "install" => install_app(),
        "setup-signing" => setup_signing(),
        "listen" | "click-probe" | "--click-probe" | "wheel-probe" | "--wheel-probe" => {
            run_listener_mode(mode != "listen")
        }
        "mirror" | "--mirror" => run_mirror_mode(),
        other => {
            eprintln!("unknown mode: {other}\nusage: screengood [mirror|listen|test|click-probe|wheel-probe|make-app|install|setup-signing]");
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
        // Absolute brew path: Finder-launched GUI processes get a PATH
        // without Homebrew in it.
        let brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .unwrap_or("brew");
        let _ = Command::new(brew).args(["install", brew_pkg]).status();
    }
}

/// Human names of the permissions a preflight found missing.
fn missing_permission_names(perms: listener::Perms) -> String {
    let mut names = Vec::new();
    if !perms.accessibility {
        names.push("Accessibility");
    }
    if !perms.input_monitoring {
        names.push("Input Monitoring");
    }
    names.join(" + ")
}

/// Open Settings panes only for permissions the preflight found missing.
/// Accessibility first, Input Monitoring last so it stays on screen.
fn open_permission_panes(perms: listener::Perms) {
    let mut panes = Vec::new();
    if !perms.accessibility {
        panes.push("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
    }
    if !perms.input_monitoring {
        panes.push("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
    }
    for pane in panes {
        let _ = Command::new("open").arg(pane).status();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Start the SHIFT+wheel listener in the background if permissions allow;
/// the mirror keeps working without it.
fn spawn_listener_if_allowed(phone: &discover::Phone) {
    // Check first, silently. Only a permission the preflight finds
    // missing is ever requested or gets a Settings pane opened.
    let perms = listener::check_permissions();
    println!(
        "Permissions preflight: Accessibility={} InputMonitoring={}",
        perms.accessibility, perms.input_monitoring
    );
    if perms.all_granted() {
        listener::spawn(listener::ListenerConfig {
            adb: phone.adb.clone(),
            geo: phone.geo,
            probe: false,
        });
    } else {
        println!("macOS blocked the SHIFT+wheel listener. Mirror still works.");
        println!(
            "ScreenGood registered itself under Settings > Privacy for: {}. Switch it ON there, then reopen.",
            missing_permission_names(perms)
        );
        listener::request_permissions(perms);
        open_permission_panes(perms);
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
    println!(" SHIFT + WHEEL -> Android control");
    println!(
        " SHIFT + wheel DOWN = NEXT | UP = PREV | SHIFT + middle/right-click = VIDEO pause/resume"
    );
    println!("==============================================");
    let phone = discover::connect_phone(&tools);
    println!("\nPhone OK ({}).\n", phone.adb.serial);
    let perms = listener::check_permissions();
    println!(
        "Permissions preflight: Accessibility={} InputMonitoring={}",
        perms.accessibility, perms.input_monitoring
    );
    if !perms.all_granted() {
        println!("*** macOS blocked the wheel listener. Fix (1 min): ***");
        println!(
            "ScreenGood registered itself under Settings > Privacy for: {}. Switch it ON there, then reopen.",
            missing_permission_names(perms)
        );
        listener::request_permissions(perms);
        open_permission_panes(perms);
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

/// Pids from `pgrep` output, excluding our own.
fn parse_other_pids(out: &str, me: u32) -> Vec<u32> {
    out.lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .filter(|&p| p != me)
        .collect()
}

fn term_pid(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}

/// Single-instance by replacement: a second launch (for example a Finder
/// double-click while a mirror is already running) terminates the previous
/// screengood and its scrcpy child instead of silently fighting it over
/// the phone's one screen capture — the loser used to exit in under a
/// second with no visible window or error.
fn kill_previous_instances() {
    let me = std::process::id();
    let Ok(out) = Command::new("/usr/bin/pgrep")
        .args(["-x", "screengood"])
        .output()
    else {
        return;
    };
    let others = parse_other_pids(&String::from_utf8_lossy(&out.stdout), me);
    if others.is_empty() {
        return;
    }
    println!("Replacing previous screengood instance(s): {others:?}");
    for pid in &others {
        // Children first (scrcpy) so phone capture releases cleanly.
        if let Ok(kids) = Command::new("/usr/bin/pgrep")
            .args(["-P", &pid.to_string()])
            .output()
        {
            for kid in parse_other_pids(&String::from_utf8_lossy(&kids.stdout), me) {
                term_pid(kid);
            }
        }
        term_pid(*pid);
    }
    std::thread::sleep(std::time::Duration::from_millis(700));
}

/// PATH for child processes: make the directory holding `adb` (Homebrew
/// bin) searchable even when launched from Finder, where the GUI session
/// PATH is only `/usr/bin:/bin:/usr/sbin:/sbin`. Without this, scrcpy's
/// own `adb start-server` spawn fails with "Command not found: [adb]"
/// and the app appears to do nothing on double-click.
fn child_env_path(adb_bin: &Path, existing: Option<&str>) -> String {
    let base = existing
        .filter(|s| !s.is_empty())
        .unwrap_or("/usr/bin:/bin:/usr/sbin:/sbin");
    match adb_bin.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => format!("{}:{base}", dir.display()),
        None => base.to_string(),
    }
}

fn run_mirror_mode() -> i32 {
    kill_previous_instances();
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
    let current_path = std::env::var("PATH").ok();
    let status = Command::new(&tools.scrcpy_bin)
        // scrcpy spawns `adb` by name (or $ADB); Finder GUI PATH lacks
        // Homebrew, so hand it the absolute adb path explicitly.
        .env("ADB", &tools.adb_bin)
        .env(
            "PATH",
            child_env_path(&tools.adb_bin, current_path.as_deref()),
        )
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
/// write Info.plist, then codesign (see `sign_app`/`SignMode`): with the
/// `setup-signing` local identity when present, so permission grants
/// survive rebuilds, else ad-hoc. The check-then-request permission flow
/// (see `spawn_listener_if_allowed`) stays silent once granted.
fn make_app() -> i32 {
    let dir = discover::project_dir();
    let app = dir.join("ScreenGood.app");
    // Icon: assets/AppIcon.icns when the source checkout is present; the
    // plist only references it then, so a bare checkout still builds a
    // valid app.
    let icon = std::fs::read(dir.join("assets").join("AppIcon.icns")).ok();
    finish_app(&app, icon.as_deref(), "make-app")
}

/// App-style install for a Cargo-installed binary (no source checkout):
/// assemble `ScreenGood.app` with the embedded icon directly into an
/// Applications folder, ad-hoc signed. Because the app is built on this
/// Mac, it carries no download quarantine, so Gatekeeper shows no malware
/// check on first launch. `SCREENGOOD_INSTALL_DIR` overrides the target
/// folder.
fn install_app() -> i32 {
    let app = install_root().join("ScreenGood.app");
    let icon: &[u8] = include_bytes!("../assets/AppIcon.icns");
    finish_app(&app, Some(icon), "install")
}

/// Pick the install folder: `SCREENGOOD_INSTALL_DIR` when set, else
/// `/Applications` when writable, else `~/Applications` (no admin needed).
fn install_root() -> PathBuf {
    if let Ok(dir) = std::env::var("SCREENGOOD_INSTALL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    choose_install_root(
        dir_is_writable(Path::new("/Applications")),
        home_applications(),
    )
}

/// Prefer system Applications when writable, else the user's. Split out so
/// the decision is unit-testable without touching the real filesystem.
fn choose_install_root(system_writable: bool, home_apps: PathBuf) -> PathBuf {
    if system_writable {
        PathBuf::from("/Applications")
    } else {
        home_apps
    }
}

fn home_applications() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Applications")
}

fn dir_is_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(".screengood-write-probe-{}", std::process::id()));
    if std::fs::write(&probe, b"probe").is_err() {
        return false;
    }
    let _ = std::fs::remove_file(&probe);
    true
}

/// Shared bundle assembly used by both `make-app` (checkout + file icon)
/// and `install` (cargo binary + embedded icon).
fn finish_app(app: &Path, icon: Option<&[u8]>, cmd: &str) -> i32 {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("screengood"));
    if let Err(e) = assemble_app(app, &exe, icon) {
        eprintln!("{cmd}: {e}");
        return 1;
    }
    match sign_app(app) {
        Ok(how) => {
            println!(
                "ScreenGood.app ready at {} (double-click = mirror)\nSigning: {how}",
                app.display()
            );
            0
        }
        Err(e) => {
            eprintln!("{cmd}: {e}");
            1
        }
    }
}

fn assemble_app(app: &Path, exe: &Path, icon: Option<&[u8]>) -> std::io::Result<()> {
    let macos = app.join("Contents").join("MacOS");
    std::fs::create_dir_all(&macos)?;
    let dest = macos.join("screengood");
    if exe != dest {
        std::fs::copy(exe, &dest)?;
    }
    let mut icon_key = String::new();
    if let Some(bytes) = icon {
        let resources = app.join("Contents").join("Resources");
        std::fs::create_dir_all(&resources)?;
        std::fs::write(resources.join("AppIcon.icns"), bytes)?;
        icon_key = "\n    <key>CFBundleIconFile</key><string>AppIcon</string>".to_string();
    }
    std::fs::write(
        app.join("Contents").join("Info.plist"),
        plist_body(env!("CARGO_PKG_VERSION"), &icon_key),
    )?;
    Ok(())
}

fn plist_body(version: &str, icon_key: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>screengood</string>
    <key>CFBundleIdentifier</key><string>com.screengood.app</string>
    <key>CFBundleName</key><string>ScreenGood</string>
    <key>CFBundleDisplayName</key><string>ScreenGood</string>{icon_key}
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
"#
    )
}

fn sign_app(app: &Path) -> Result<String, String> {
    let app_str = app.to_str().unwrap_or_default().to_string();
    let mode = resolve_sign_mode();
    match run_codesign(&codesign_args(&mode, &app_str)) {
        Ok(()) => Ok(mode.describe().to_string()),
        Err(e) if !matches!(mode, SignMode::AdHoc) => {
            // A broken/locked identity must not brick installs, but falling
            // back silently would revive per-rebuild permission prompts.
            eprintln!(
                "warning: signing with {SIGN_CN} failed ({e}); falling back to \
                 ad-hoc. Fix with `screengood setup-signing`, or this build \
                 will ask for permissions again after the next rebuild."
            );
            run_codesign(&codesign_args(&SignMode::AdHoc, &app_str))?;
            Ok(SignMode::AdHoc.describe().to_string())
        }
        Err(e) => Err(e),
    }
}

fn run_codesign(args: &[String]) -> Result<(), String> {
    match Command::new("codesign").args(args).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("codesign failed: {s}")),
        Err(e) => Err(format!("codesign: {e}")),
    }
}

/// How the app gets signed. Ad-hoc is the zero-setup default; its TCC
/// designated requirement is the build's own code hash, so every rebuild
/// needs fresh permission grants. `LocalIdentity` is a self-signed
/// certificate in a dedicated keychain file (login keychain untouched);
/// its designated requirement (`identifier + certificate leaf`) is the
/// same for every build, so grants survive rebuilds. Created once via
/// `screengood setup-signing`; never auto-created silently.
#[derive(Debug, PartialEq)]
enum SignMode {
    AdHoc,
    LocalIdentity { hash: String, keychain: String },
}

impl SignMode {
    fn describe(&self) -> &'static str {
        match self {
            SignMode::AdHoc => "ad-hoc signed (grant permissions again after each rebuild; run `screengood setup-signing` for one-time setup)",
            SignMode::LocalIdentity { .. } => {
                "signed with ScreenGood Local Signing (permission grants survive rebuilds)"
            }
        }
    }
}

fn codesign_args(mode: &SignMode, app_str: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match mode {
        SignMode::LocalIdentity { hash, keychain } => {
            args.push("--keychain".into());
            args.push(keychain.clone());
            args.push("--force".into());
            args.push("--sign".into());
            args.push(hash.clone());
        }
        SignMode::AdHoc => {
            args.push("--force".into());
            args.push("--sign".into());
            args.push("-".into());
        }
    }
    args.push("--identifier".into());
    args.push("com.screengood.app".into());
    args.push(app_str.to_string());
    args
}

const SIGN_CN: &str = "ScreenGood Local Signing";

/// All signing material lives under
/// `~/Library/Application Support/ScreenGood/`: a dedicated keychain file
/// (never the login keychain), its password file (0600), and the public
/// certificate (kept so trust can be removed later).
fn signing_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/ScreenGood")
}

struct SigningPaths {
    keychain: PathBuf,
    password_file: PathBuf,
    cert: PathBuf,
}

fn signing_paths(dir: &Path) -> SigningPaths {
    SigningPaths {
        keychain: dir.join("Signing.keychain-db"),
        password_file: dir.join("keychain-password.txt"),
        cert: dir.join("signing-cert.pem"),
    }
}

/// Find `<40-hex>` for `cn` in `security find-identity -v` output. Lines
/// look like `  1) C87B.. "ScreenGood Local Signing"`.
fn find_sign_identity(output: &str, cn: &str) -> Option<String> {
    for line in output.lines() {
        let Some(name_start) = line.find('"') else {
            continue;
        };
        let Some(name_end) = line[name_start + 1..].find('"') else {
            continue;
        };
        let name_end = name_start + 1 + name_end;
        if &line[name_start + 1..name_end] != cn {
            continue;
        }
        if let Some(hash) = line[..name_start]
            .split_whitespace()
            .find(|t| t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Some(hash.to_string());
        }
    }
    None
}

fn read_keychain_password(paths: &SigningPaths) -> Option<String> {
    std::fs::read_to_string(&paths.password_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn resolve_sign_mode() -> SignMode {
    let paths = signing_paths(&signing_dir());
    if !paths.keychain.exists() {
        return SignMode::AdHoc;
    }
    if let Some(pw) = read_keychain_password(&paths) {
        let kc = paths.keychain.to_str().unwrap_or_default();
        let _ = Command::new("security")
            .args(["unlock-keychain", "-p", &pw, kc])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let kc = paths.keychain.to_str().unwrap_or_default().to_string();
    let out = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning", &kc])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            match find_sign_identity(&text, SIGN_CN) {
                Some(hash) => SignMode::LocalIdentity { hash, keychain: kc },
                None => SignMode::AdHoc,
            }
        }
        Err(_) => SignMode::AdHoc,
    }
}

fn run_quiet(cmd: &str, args: &[&str]) -> Result<(), String> {
    match Command::new(cmd).args(args).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "{cmd} {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("{cmd}: {e}")),
    }
}

fn random_password() -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// One-time, user-run setup: create the stable local signing identity so
/// permission grants survive rebuilds. macOS shows one password dialog to
/// trust the certificate; afterwards `make-app`/`install` sign with it
/// automatically. Nothing here touches the login keychain.
fn setup_signing() -> i32 {
    println!("== ScreenGood one-time signing setup ==");
    println!(
        "This creates a self-signed identity named \"{SIGN_CN}\" in a \
         dedicated keychain file (your login keychain is not used). Every \
         future build signed with it keeps your existing Accessibility / \
         Input Monitoring grants — no more re-granting after rebuilds."
    );
    let dir = signing_dir();
    let paths = signing_paths(&dir);
    if matches!(resolve_sign_mode(), SignMode::LocalIdentity { .. }) {
        println!("Already set up: {SIGN_CN} is valid and will be used automatically.");
        println!("Next: run `screengood install`, grant both permissions once, done.");
        return 0;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("setup-signing: cannot create {}: {e}", dir.display());
        return 1;
    }
    // Keychain password: generated once, kept beside the keychain (0600).
    let pw = match read_keychain_password(&paths) {
        Some(p) => p,
        None => {
            let p = match random_password() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("setup-signing: cannot read /dev/urandom: {e}");
                    return 1;
                }
            };
            if let Err(e) = std::fs::write(&paths.password_file, format!("{p}\n")) {
                eprintln!("setup-signing: cannot write password file: {e}");
                return 1;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &paths.password_file,
                    std::fs::Permissions::from_mode(0o600),
                );
            }
            p
        }
    };
    let kc = paths.keychain.to_str().unwrap_or_default().to_string();
    let steps: Vec<(&str, Vec<String>)> = {
        let mut v: Vec<(&str, Vec<String>)> = Vec::new();
        if !paths.keychain.exists() {
            v.push((
                "security",
                vec![
                    "create-keychain".into(),
                    "-p".into(),
                    pw.clone(),
                    kc.clone(),
                ],
            ));
        }
        v.push((
            "security",
            vec![
                "unlock-keychain".into(),
                "-p".into(),
                pw.clone(),
                kc.clone(),
            ],
        ));
        v.push(("security", vec!["set-keychain-settings".into(), kc.clone()]));
        v
    };
    for (cmd, args) in &steps {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(e) = run_quiet(cmd, &refs) {
            eprintln!("setup-signing: {e}");
            return 1;
        }
    }
    // Certificate material is generated in a temp folder; only the public
    // cert is kept (needed to remove trust later). The private key ends
    // up solely inside the encrypted keychain file.
    let tmp = dir.join("setup-tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        eprintln!("setup-signing: {e}");
        return 1;
    }
    let key_pem = tmp.join("key.pem");
    let cert_pem = tmp.join("cert.pem");
    let p12 = tmp.join("identity.p12");
    let passout = format!("pass:{pw}");
    let gen: Vec<(&str, Vec<String>)> = vec![
        (
            "openssl",
            vec![
                "req".into(),
                "-x509".into(),
                "-newkey".into(),
                "rsa:2048".into(),
                "-sha256".into(),
                "-days".into(),
                "3650".into(),
                "-nodes".into(),
                "-subj".into(),
                format!("/CN={SIGN_CN}"),
                "-addext".into(),
                "keyUsage=critical,digitalSignature".into(),
                "-addext".into(),
                "extendedKeyUsage=critical,codeSigning".into(),
                "-addext".into(),
                "basicConstraints=critical,CA:FALSE".into(),
                "-keyout".into(),
                key_pem.to_str().unwrap_or_default().to_string(),
                "-out".into(),
                cert_pem.to_str().unwrap_or_default().to_string(),
            ],
        ),
        (
            "openssl",
            vec![
                "pkcs12".into(),
                "-export".into(),
                "-out".into(),
                p12.to_str().unwrap_or_default().to_string(),
                "-inkey".into(),
                key_pem.to_str().unwrap_or_default().to_string(),
                "-in".into(),
                cert_pem.to_str().unwrap_or_default().to_string(),
                "-name".into(),
                SIGN_CN.into(),
                "-passout".into(),
                passout.clone(),
            ],
        ),
        (
            "security",
            vec![
                "import".into(),
                p12.to_str().unwrap_or_default().to_string(),
                "-k".into(),
                kc.clone(),
                "-P".into(),
                pw.clone(),
                "-T".into(),
                "/usr/bin/codesign".into(),
                "-T".into(),
                "/usr/bin/security".into(),
            ],
        ),
        (
            "security",
            vec![
                "set-key-partition-list".into(),
                "-S".into(),
                "apple-tool:,apple:".into(),
                "-s".into(),
                "-k".into(),
                pw.clone(),
                kc.clone(),
            ],
        ),
    ];
    for (cmd, args) in &gen {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(e) = run_quiet(cmd, &refs) {
            eprintln!("setup-signing: {e}");
            return 1;
        }
    }
    if let Err(e) = std::fs::copy(&cert_pem, &paths.cert) {
        eprintln!("setup-signing: cannot keep public cert: {e}");
        return 1;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    println!("macOS will now ask for your login password ONCE to trust this certificate for code signing. That dialog is expected.");
    let trust = Command::new("security")
        .args([
            "add-trusted-cert",
            "-r",
            "trustRoot",
            "-p",
            "codeSign",
            "-k",
            &kc,
            paths.cert.to_str().unwrap_or_default(),
        ])
        .status();
    match trust {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("setup-signing: trusting the certificate was cancelled ({s}). Re-run `screengood setup-signing` when ready.");
            return 1;
        }
        Err(e) => {
            eprintln!("setup-signing: security add-trusted-cert: {e}");
            return 1;
        }
    }
    match resolve_sign_mode() {
        SignMode::LocalIdentity { .. } => {
            println!("Done. \"{SIGN_CN}\" is trusted and will sign every build from now on.");
            println!("Next steps:");
            println!("  1. screengood install        (rebuilds + installs the stable-signed app)");
            println!(
                "  2. Open ScreenGood once and switch it ON in Accessibility + Input Monitoring."
            );
            println!("  3. Reopen it. Grants now survive every future rebuild.");
            0
        }
        SignMode::AdHoc => {
            eprintln!("setup-signing: identity imported but still not valid — trust may not have been approved. Re-run this command to retry.");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_other_pids_excludes_self_and_garbage() {
        assert_eq!(parse_other_pids("101\n202\n303\n", 202), vec![101, 303]);
        assert_eq!(parse_other_pids("", 1), Vec::<u32>::new());
        assert_eq!(parse_other_pids("42\n", 42), Vec::<u32>::new());
        assert_eq!(parse_other_pids("abc\n 77 \n-5\n", 1), vec![77]);
    }

    #[test]
    fn missing_permission_names_lists_only_ungranted() {
        use crate::listener::Perms;
        assert_eq!(
            missing_permission_names(Perms {
                accessibility: false,
                input_monitoring: false
            }),
            "Accessibility + Input Monitoring"
        );
        assert_eq!(
            missing_permission_names(Perms {
                accessibility: true,
                input_monitoring: false
            }),
            "Input Monitoring"
        );
        assert_eq!(
            missing_permission_names(Perms {
                accessibility: false,
                input_monitoring: true
            }),
            "Accessibility"
        );
        assert_eq!(
            missing_permission_names(Perms {
                accessibility: true,
                input_monitoring: true
            }),
            ""
        );
    }

    #[test]
    fn child_env_path_prepends_adb_dir_for_finder_launches() {
        let adb = Path::new("/opt/homebrew/bin/adb");
        // Finder GUI session: no PATH or minimal PATH.
        assert_eq!(
            child_env_path(adb, None),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
        assert_eq!(
            child_env_path(adb, Some("")),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
        assert_eq!(
            child_env_path(adb, Some("/usr/bin:/bin")),
            "/opt/homebrew/bin:/usr/bin:/bin"
        );
        // Terminal launches keep their richer PATH, still prefixed.
        assert_eq!(
            child_env_path(adb, Some("/x/bin:/opt/homebrew/bin")),
            "/opt/homebrew/bin:/x/bin:/opt/homebrew/bin"
        );
        assert_eq!(
            child_env_path(Path::new("adb"), Some("/usr/bin")),
            "/usr/bin"
        );
    }

    #[test]
    fn plist_body_carries_version_and_optional_icon() {
        let with_icon = plist_body(
            "0.2.0",
            "\n    <key>CFBundleIconFile</key><string>AppIcon</string>",
        );
        assert!(with_icon.contains("<string>0.2.0</string>"));
        assert!(with_icon.contains("CFBundleIconFile"));
        let bare = plist_body("0.2.0", "");
        assert!(bare.contains("<string>0.2.0</string>"));
        assert!(!bare.contains("CFBundleIconFile"));
    }

    #[test]
    fn choose_install_root_prefers_writable_system_apps() {
        let home = PathBuf::from("/Users/someone/Applications");
        assert_eq!(
            choose_install_root(true, home.clone()),
            PathBuf::from("/Applications")
        );
        assert_eq!(choose_install_root(false, home.clone()), home);
    }

    fn assemble_test_app(name: &str, icon: Option<&[u8]>) -> PathBuf {
        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fake_exe = root.join("screengood-src");
        std::fs::write(&fake_exe, b"fake-binary").unwrap();
        let app = root.join("ScreenGood.app");
        assemble_app(&app, &fake_exe, icon).unwrap();
        assert!(app.join("Contents/MacOS/screengood").is_file());
        app
    }

    #[test]
    fn assemble_app_writes_iconed_bundle_with_crate_version() {
        let app = assemble_test_app("screengood-assemble-icon", Some(b"fake-icon"));
        let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains(&format!("<string>{}</string>", env!("CARGO_PKG_VERSION"))));
        assert!(plist.contains("CFBundleIconFile"));
        assert_eq!(
            std::fs::read(app.join("Contents/Resources/AppIcon.icns")).unwrap(),
            b"fake-icon"
        );
        let _ = std::fs::remove_dir_all(app.parent().unwrap());
    }

    #[test]
    fn find_sign_identity_parses_named_identity_only() {
        let out = "\nPolicy: Code Signing\n  Matching identities\n  1) C87B4A89AB99ECF8C9C2EC70A59BC0BB1B262C76 \"ScreenGood Local Signing\"\n     1 identities found\n";
        assert_eq!(
            find_sign_identity(out, "ScreenGood Local Signing"),
            Some("C87B4A89AB99ECF8C9C2EC70A59BC0BB1B262C76".to_string())
        );
        assert_eq!(find_sign_identity(out, "Developer ID Application"), None);
        assert_eq!(find_sign_identity("", "ScreenGood Local Signing"), None);
        // Quoteless/other-identity lines must not abort the scan.
        let two = "  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA \"Other Dev\"\n  2) C87B4A89AB99ECF8C9C2EC70A59BC0BB1B262C76 \"ScreenGood Local Signing\"\n";
        assert_eq!(
            find_sign_identity(two, "ScreenGood Local Signing"),
            Some("C87B4A89AB99ECF8C9C2EC70A59BC0BB1B262C76".to_string())
        );
    }

    #[test]
    fn codesign_args_match_each_sign_mode() {
        assert_eq!(
            codesign_args(&SignMode::AdHoc, "/A.app"),
            vec![
                "--force",
                "--sign",
                "-",
                "--identifier",
                "com.screengood.app",
                "/A.app"
            ]
        );
        assert_eq!(
            codesign_args(
                &SignMode::LocalIdentity {
                    hash: "ABC".into(),
                    keychain: "/k.db".into()
                },
                "/A.app"
            ),
            vec![
                "--keychain",
                "/k.db",
                "--force",
                "--sign",
                "ABC",
                "--identifier",
                "com.screengood.app",
                "/A.app"
            ]
        );
    }

    #[test]
    fn signing_paths_live_under_support_dir() {
        let p = signing_paths(Path::new("/Users/x/Library/Application Support/ScreenGood"));
        assert!(p.keychain.ends_with("Signing.keychain-db"));
        assert!(p.password_file.ends_with("keychain-password.txt"));
        assert!(p.cert.ends_with("signing-cert.pem"));
    }

    #[test]
    fn assemble_app_without_icon_omits_icon_key() {
        let app = assemble_test_app("screengood-assemble-bare", None);
        let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
        assert!(!plist.contains("CFBundleIconFile"));
        assert!(!app.join("Contents/Resources").exists());
        let _ = std::fs::remove_dir_all(app.parent().unwrap());
    }
}
