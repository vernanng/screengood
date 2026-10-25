//! Thin wrapper around the external `adb` binary (plan Q2: adb stays
//! external). Command *construction* is pure and unit-tested; execution
//! mirrors the Python `run_cmd` (5s timeout, log rc + stderr).

use crate::logic::{Action, Geometry};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Adb {
    pub bin: PathBuf,
    /// Pinned serial; empty means "no -s flag" (single-device setups).
    pub serial: String,
}

impl Adb {
    pub fn new(bin: PathBuf, serial: String) -> Self {
        Self { bin, serial }
    }

    /// `[bin]` or `[bin, "-s", serial]`.
    pub fn cmd_prefix(&self) -> Vec<String> {
        let mut prefix = vec![self.bin.to_string_lossy().into_owned()];
        if !self.serial.is_empty() {
            prefix.push("-s".to_string());
            prefix.push(self.serial.clone());
        }
        prefix
    }

    /// `prefix + ["shell", "input", ...args]`.
    pub fn input_cmd(&self, args: &[&str]) -> Vec<String> {
        let mut cmd = self.cmd_prefix();
        cmd.push("shell".to_string());
        cmd.push("input".to_string());
        cmd.extend(args.iter().map(|a| a.to_string()));
        cmd
    }

    pub fn swipe_cmd(&self, x1: i64, y1: i64, x2: i64, y2: i64, dur_ms: u32) -> Vec<String> {
        self.input_cmd(&[
            "swipe",
            &x1.to_string(),
            &y1.to_string(),
            &x2.to_string(),
            &y2.to_string(),
            &dur_ms.to_string(),
        ])
    }

    /// NEXT = finger UP (bottom -> top); PREV = finger DOWN (top -> bottom).
    pub fn swipe_action_cmd(&self, geo: Geometry, action: Action, dur_ms: u32) -> Vec<String> {
        match action {
            Action::Next => self.swipe_cmd(geo.cx, geo.swipe_bot, geo.cx, geo.swipe_top, dur_ms),
            Action::Prev => self.swipe_cmd(geo.cx, geo.swipe_top, geo.cx, geo.swipe_bot, dur_ms),
        }
    }

    pub fn tap_cmd(&self, x: i64, y: i64) -> Vec<String> {
        self.input_cmd(&["tap", &x.to_string(), &y.to_string()])
    }

    /// Tap center = Reels video pause/resume (keyevent 85 is ignored by FB).
    pub fn video_cmd(&self, geo: Geometry) -> Vec<String> {
        self.tap_cmd(geo.cx, geo.tap_y)
    }

    pub fn keyevent_cmd(&self, code: u32) -> Vec<String> {
        self.input_cmd(&["keyevent", &code.to_string()])
    }

    /// Run to completion with a 5s timeout; returns process rc or -1.
    pub fn run(&self, cmd: &[String]) -> i32 {
        if cmd.is_empty() {
            println!("adb error: empty command");
            return -1;
        }
        let mut child = match Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                println!("adb error: {e}");
                return -1;
            }
        };

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait_with_output();
                        println!("adb error: timeout after 5s");
                        return -1;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    println!("adb error: {e}");
                    return -1;
                }
            }
        }

        match child.wait_with_output() {
            Ok(output) => {
                let rc = output.status.code().unwrap_or(-1);
                if rc != 0 {
                    println!(
                        "adb rc={rc}: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                rc
            }
            Err(e) => {
                println!("adb error: {e}");
                -1
            }
        }
    }

    /// Fire-and-forget on a daemon thread (listener callbacks never block on adb).
    pub fn run_detached(&self, cmd: &[String]) {
        let adb = self.clone();
        let cmd_owned: Vec<String> = cmd.to_vec();
        std::thread::spawn(move || {
            let _ = adb.run(&cmd_owned);
        });
    }

    /// Run `bin + args` and capture stdout (devices, wm size, connect...).
    pub fn run_capture(&self, args: &[&str]) -> Option<String> {
        match Command::new(&self.bin).args(args).output() {
            Ok(output) => Some(String::from_utf8_lossy(&output.stdout).into_owned()),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_prefix_empty_serial() {
        let adb = Adb::new(PathBuf::from("adb"), String::new());
        assert_eq!(adb.cmd_prefix(), vec!["adb".to_string()]);
    }

    #[test]
    fn cmd_prefix_with_serial() {
        let adb = Adb::new(PathBuf::from("adb"), "S".to_string());
        assert_eq!(
            adb.cmd_prefix(),
            vec!["adb".to_string(), "-s".to_string(), "S".to_string()]
        );
    }

    #[test]
    fn production_swipe_duration_is_500() {
        assert_eq!(crate::logic::SWIPE_DUR_MS, 500);
        let adb = Adb::new(PathBuf::from("adb"), "FAKE:1".to_string());
        let cmd = adb.swipe_action_cmd(
            crate::logic::Geometry::default(),
            crate::logic::Action::Next,
            crate::logic::SWIPE_DUR_MS,
        );
        assert_eq!(cmd.last().map(String::as_str), Some("500"));
    }

    #[test]
    fn swipe_action_next_is_bottom_to_top() {
        let geo = Geometry::default();
        let adb = Adb::new(PathBuf::from("adb"), "FAKE:1".to_string());
        let cmd = adb.swipe_action_cmd(geo, Action::Next, 400);
        let tail = &cmd[cmd.len() - 5..cmd.len() - 1];
        assert_eq!(
            tail,
            [
                geo.cx.to_string(),
                geo.swipe_bot.to_string(),
                geo.cx.to_string(),
                geo.swipe_top.to_string()
            ]
            .as_slice()
        );
    }

    #[test]
    fn swipe_action_prev_is_top_to_bottom() {
        let geo = Geometry::default();
        let adb = Adb::new(PathBuf::from("adb"), "FAKE:1".to_string());
        let cmd = adb.swipe_action_cmd(geo, Action::Prev, 400);
        let tail = &cmd[cmd.len() - 5..cmd.len() - 1];
        assert_eq!(
            tail,
            [
                geo.cx.to_string(),
                geo.swipe_top.to_string(),
                geo.cx.to_string(),
                geo.swipe_bot.to_string()
            ]
            .as_slice()
        );
    }

    #[test]
    fn video_cmd_is_tap_center() {
        let geo = Geometry::default();
        let adb = Adb::new(PathBuf::from("adb"), "FAKE:1".to_string());
        let cmd = adb.video_cmd(geo);
        let tail = &cmd[cmd.len() - 3..];
        assert_eq!(
            tail,
            ["tap".to_string(), geo.cx.to_string(), geo.tap_y.to_string()].as_slice()
        );
    }
}
