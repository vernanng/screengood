//! Tool resolution + phone discovery, ported from the legacy shared shell lib
//! (`resolve_tools`, `pick_serial`, `auto_connect`, `detect_phone_size`,
//! `connect_phone`). mDNS browsing uses the `mdns-sd` crate instead of
//! backgrounding `dns-sd` and parsing temp files.

use crate::adb::Adb;
use crate::logic::Geometry;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Tools {
    pub adb_bin: PathBuf,
    pub scrcpy_bin: PathBuf,
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let cand = dir.join(name);
        if cand.exists() {
            return Some(cand);
        }
    }
    None
}

/// Homebrew prefix first, then PATH (same order as `resolve_tools`).
pub fn resolve_tools() -> Tools {
    let prefix: PathBuf = match Command::new("brew").args(["--prefix"]).output() {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                brew_fallback_prefix()
            } else {
                PathBuf::from(s)
            }
        }
        _ => brew_fallback_prefix(),
    };
    let brew_bin = prefix.join("bin");
    let resolve = |name: &str| -> PathBuf {
        let cand = brew_bin.join(name);
        if cand.exists() {
            cand
        } else if let Some(found) = find_on_path(name) {
            found
        } else {
            cand
        }
    };
    Tools {
        adb_bin: resolve("adb"),
        scrcpy_bin: resolve("scrcpy"),
    }
}

fn brew_fallback_prefix() -> PathBuf {
    if Path::new("/opt/homebrew/bin/adb").exists() {
        PathBuf::from("/opt/homebrew")
    } else {
        PathBuf::from("/usr/local")
    }
}

/// Serials of devices in `device` state from `adb devices` output.
pub fn parse_devices(out: &str) -> Vec<String> {
    let mut devices = Vec::new();
    for line in out.lines() {
        if let Some((serial, state)) = line.split_once('\t') {
            if state.trim() == "device" && !serial.is_empty() {
                devices.push(serial.to_string());
            }
        }
    }
    devices
}

/// `^[0-9.]+:[0-9]+$` (bash `pick_serial` WiFi-direct pattern).
fn is_ip_port(serial: &str) -> bool {
    match serial.rsplit_once(':') {
        Some((host, port)) => {
            !host.is_empty()
                && host.chars().all(|c| c.is_ascii_digit() || c == '.')
                && !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// USB cable first (lowest delay), then `ip:port`, then anything.
/// USB serials are those not matching `^[0-9.]+:[0-9]+$` and not ending `_tcp`.
pub fn pick_serial(devices: &[String]) -> Option<String> {
    if let Some(s) = devices
        .iter()
        .find(|d| !is_ip_port(d) && !d.ends_with("_tcp"))
    {
        return Some(s.clone());
    }
    if let Some(s) = devices.iter().find(|d| is_ip_port(d)) {
        return Some(s.clone());
    }
    devices.first().cloned()
}

/// First `WxH` in `adb shell wm size` output.
pub fn parse_wm_size(out: &str) -> Option<(i64, i64)> {
    let bytes = out.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'x' {
                let h_start = i + 1;
                let mut j = h_start;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > h_start {
                    let w: i64 = out[start..i].parse().ok()?;
                    let h: i64 = out[h_start..j].parse().ok()?;
                    return Some((w, h));
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Bash profile test: `*:*|*_tcp` = WiFi transport, everything else USB.
pub fn is_usb_serial(serial: &str) -> bool {
    !serial.contains(':') && !serial.ends_with("_tcp")
}

/// Directory holding `phone-ip.txt`: the folder the user launched
/// from. `ScreenGood.app/Contents/MacOS/screengood` -> the app's folder;
/// `target/<profile>/screengood` -> the repo root; else the exe's folder.
/// `SCREENGOOD_DIR` overrides everything.
pub fn project_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SCREENGOOD_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return PathBuf::from("."),
    };
    let parent = match exe.parent() {
        Some(p) => p.to_path_buf(),
        None => return PathBuf::from("."),
    };
    // <App>.app/Contents/MacOS/screengood -> parent of the .app
    if parent.file_name().map(|n| n == "MacOS").unwrap_or(false) {
        if let Some(contents) = parent.parent() {
            if contents
                .file_name()
                .map(|n| n == "Contents")
                .unwrap_or(false)
            {
                if let Some(app) = contents.parent() {
                    let is_app = app
                        .file_name()
                        .map(|n| n.to_string_lossy().ends_with(".app"))
                        .unwrap_or(false);
                    if is_app {
                        if let Some(app_parent) = app.parent() {
                            return app_parent.to_path_buf();
                        }
                    }
                }
            }
        }
    }
    // .../target/(debug|release)/screengood -> parent of the target dir
    let profile = parent.file_name().map(|n| n.to_string_lossy().into_owned());
    if matches!(profile.as_deref(), Some("debug") | Some("release")) {
        if let Some(target) = parent.parent() {
            if target.file_name().map(|n| n == "target").unwrap_or(false) {
                if let Some(root) = target.parent() {
                    return root.to_path_buf();
                }
            }
        }
    }
    parent
}

#[derive(Debug, Clone)]
pub struct Phone {
    pub adb: Adb,
    pub geo: Geometry,
}

fn connected(bare: &Adb) -> bool {
    match bare.run_capture(&["devices"]) {
        Some(out) => !parse_devices(&out).is_empty(),
        None => false,
    }
}

/// USB or already-connected wins instantly; else saved IP, then mDNS search.
fn auto_connect(bare: &Adb) -> bool {
    if connected(bare) {
        return true;
    }
    let _ = bare.run_capture(&["disconnect"]);
    let _ = bare.run_capture(&["kill-server"]);
    std::thread::sleep(Duration::from_secs(1));
    let _ = bare.run_capture(&["start-server"]);
    for _ in 0..3 {
        if connected(bare) {
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let ipfile = project_dir().join("phone-ip.txt");
    let saved_ip = std::fs::read_to_string(&ipfile)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !saved_ip.is_empty() {
        let _ = bare.run_capture(&["connect", &saved_ip]);
        std::thread::sleep(Duration::from_secs(2));
        if connected(bare) {
            return true;
        }
    }
    // mDNS: browse _adb-tls-connect for up to ~8s, collecting candidates.
    let mut candidates: Vec<(String, String)> = Vec::new(); // (fullname, host:port)
    if let Ok(daemon) = ServiceDaemon::new() {
        if let Ok(receiver) = daemon.browse("_adb-tls-connect._tcp.local.") {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match receiver.recv_timeout(remaining) {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        let host = match info.get_addresses_v4().into_iter().next() {
                            Some(ip) => ip.to_string(),
                            None => info.get_hostname().trim_end_matches('.').to_string(),
                        };
                        if host.is_empty() {
                            continue;
                        }
                        let hp = format!("{}:{}", host, info.get_port());
                        let name = info.get_fullname().to_string();
                        if !candidates.iter().any(|(_, existing)| existing == &hp) {
                            candidates.push((name, hp));
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        let _ = daemon.shutdown();
    }
    for (name, hp) in &candidates {
        println!("Found {name} -> {hp}, connecting...");
        let _ = bare.run_capture(&["connect", hp]);
        std::thread::sleep(Duration::from_secs(2));
        if connected(bare) {
            let _ = std::fs::write(&ipfile, format!("{hp}\n"));
            return true;
        }
    }
    false
}

/// Find the phone (USB, saved IP, then mDNS; ENTER-retry loop like bash),
/// pin the serial, detect the real screen size. Blocks until connected.
pub fn connect_phone(tools: &Tools) -> Phone {
    let bare = Adb::new(tools.adb_bin.clone(), String::new());
    auto_connect(&bare);
    while !connected(&bare) {
        println!();
        println!("--- Phone not found automatically ---");
        println!(
            "On S25 FE: USB cable plugged in, or SAME WiFi as Mac with Wireless debugging ON."
        );
        print!("Fix that, then press ENTER to search again: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        auto_connect(&bare);
    }
    let devices = parse_devices(&bare.run_capture(&["devices"]).unwrap_or_default());
    let serial = pick_serial(&devices).unwrap_or_default();
    let adb = Adb::new(tools.adb_bin.clone(), serial.clone());
    let geo = bare
        .run_capture(&["-s", &serial, "shell", "wm", "size"])
        .and_then(|out| parse_wm_size(&out))
        .map(|(w, h)| Geometry::new(w, h))
        .unwrap_or_default();
    Phone { adb, geo }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_devices_filters_states() {
        let out = "List of devices attached\nR5CX123456\tdevice\n192.168.1.5:5555\tdevice\nemulator-5554\toffline\nABC123\tunauthorized\n\n";
        assert_eq!(
            parse_devices(out),
            vec!["R5CX123456".to_string(), "192.168.1.5:5555".to_string()]
        );
        assert!(parse_devices("List of devices attached\n\n").is_empty());
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn pick_serial_usb_first() {
        let devs = vec![
            "192.168.1.5:5555".to_string(),
            "adb-XYZ._adb-tls-connect._tcp".to_string(),
            "R5CX123456".to_string(),
        ];
        assert_eq!(pick_serial(&devs), Some("R5CX123456".to_string()));
    }

    #[test]
    fn pick_serial_ip_fallback() {
        let devs = vec![
            "adb-XYZ._adb-tls-connect._tcp".to_string(),
            "192.168.1.5:5555".to_string(),
        ];
        assert_eq!(pick_serial(&devs), Some("192.168.1.5:5555".to_string()));
        let only_tcp = vec!["adb-XYZ._adb-tls-connect._tcp".to_string()];
        assert_eq!(
            pick_serial(&only_tcp),
            Some("adb-XYZ._adb-tls-connect._tcp".to_string())
        );
    }

    #[test]
    fn pick_serial_empty() {
        assert_eq!(pick_serial(&[]), None);
    }

    #[test]
    fn parse_wm_size_cases() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2340"),
            Some((1080, 2340))
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2340\nOverride size: 720x1600"),
            Some((1080, 2340))
        );
        assert_eq!(parse_wm_size("garbage no size here"), None);
        assert_eq!(parse_wm_size(""), None);
        assert_eq!(parse_wm_size("size: x2340"), None);
    }

    #[test]
    fn is_usb_serial_cases() {
        assert!(is_usb_serial("R5CX123456"));
        assert!(!is_usb_serial("192.168.1.5:5555"));
        assert!(!is_usb_serial("adb-XYZ._adb-tls-connect._tcp"));
        assert!(!is_usb_serial("something_tcp"));
    }

    #[test]
    fn is_ip_port_cases() {
        assert!(is_ip_port("192.168.1.5:5555"));
        assert!(!is_ip_port("R5CX123456"));
        assert!(!is_ip_port("192.168.1.5:abc"));
        assert!(!is_ip_port("host.local:5555"));
        assert!(!is_ip_port(":5555"));
    }
}
