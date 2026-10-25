# ANDROID SCREEN MIRROR INSIDE MAC WITH CONTROL

**Wi-Fi or USB-C.**

See your Android phone inside a window on your Mac and control the entire
phone with your Mac mouse — every app, every screen — without touching the
phone. Tuned on a Samsung S25 FE, including while the same phone is mirrored
to a Samsung TV with Smart View.

One Rust binary does the work. It finds the phone, opens the mirror with
`scrcpy`, and turns mouse wheel actions into precise up/down phone swipes over
`adb`, so scrolling works the same inside any app.

## Quick start

If `ScreenGood.app` is already built, double-click it.

Otherwise, build and run:

```sh
cargo build --release
./target/release/screengood make-app
open ScreenGood.app
```

Terminal alternative:

```sh
./target/release/screengood mirror   # default mode
```

The Android screen appears inside the Mac mirror window. Clicks and drags in
that window act on the phone immediately.

## Wi-Fi or USB-C

| Connection | Experience | Mirror profile |
|---|---|---|
| **USB-C** | Preferred. Near-zero delay and the steadiest control. | 664×1440 H.265, 12 Mbps, up to 60 fps, 0 ms video buffer, 30 ms raw-audio buffer |
| **Wi-Fi** | No cable. Extra buffering absorbs network jitter. | 414×900, 100 ms video buffer, 100 ms audio buffer |

USB-C wins automatically whenever a cable-connected phone is available. Without
a cable, the app tries the last saved Wi-Fi address and then searches the local
network with mDNS.

The S25 FE's hardware HEVC encoder held 59–61 fps at the USB profile even while
another encode was running. The frame is intentionally smaller than the phone's
native 1080×2340 screen to keep encoding and decoding delay low.

## Controls

| Input | Action |
|---|---|
| **Click** | Tap |
| **Mouse scroll down / up** | Swipe up / down (scroll forward / back) |
| **On-screen scroll** — plain wheel, mouse on the mirror | Swipe up / down |
| **Off-screen scroll — SHIFT + SCROLL**, mouse anywhere else | Swipe up / down |

On-screen scroll and off-screen SHIFT+SCROLL are the same function: one
light-scroll step at a time with the same 400 ms phone swipe, in any app.

Also: drag = swipe · right-click inside the mirror = Android Back ·
SHIFT+middle-click (or SHIFT+right-click) away from the mirror = tap center
of the phone screen.

Clicks on the mirror itself belong to `scrcpy`. The listener deliberately steps
aside there so a click cannot produce a double tap.

Watching on a TV is optional. Start Samsung Smart View separately if you want
the same phone shown on the TV while controlling it from the Mac.

## One-time setup

### Android

For USB-C:

1. Enable Developer options.
2. Enable **USB debugging**.
3. Connect the phone with a data-capable USB-C cable.
4. Unlock the phone and accept the USB-debugging prompt if shown.

For Wi-Fi:

1. Put the phone and Mac on the same Wi-Fi network.
2. Enable Developer options.
3. Enable **Wireless debugging**.
4. Keep the phone screen on for the first connection.

For TV viewing, start Smart View on the phone and choose the TV.

### Mac

Allow **ScreenGood.app** in both places:

- **Settings → Privacy & Security → Input Monitoring**
- **Settings → Privacy & Security → Accessibility**

Then reopen the app. These permissions power the global SHIFT+wheel and
wheel-on-mirror listener. The scrcpy mirror can open without them, but the
custom NEXT/PREV wheel controls need the listener.

The app detects a missing grant and opens the relevant Settings panes.

### Tools

Homebrew packages `android-platform-tools` and `scrcpy` are installed
automatically if missing. A Rust toolchain is needed only when building from
source. There is no Python runtime dependency.

## Commands

```sh
./target/release/screengood              # mirror on the Mac (default)
./target/release/screengood mirror       # same, explicit
./target/release/screengood listen       # wheel/click listener only
./target/release/screengood test         # built-in selftest; no phone needed
./target/release/screengood wheel-probe  # 20 s raw wheel/click event log
./target/release/screengood click-probe  # click-focused probe alias
./target/release/screengood make-app     # rebuild and sign ScreenGood.app
```

Tests:

```sh
cargo test  # 25 unit tests, no phone needed
```

`make-app` uses ad-hoc signing with the stable identifier
`com.screengood.app`, so macOS permission grants normally survive rebuilds.

## How it works

`screengood` is a single Cargo binary. `adb` and `scrcpy` remain external tools;
Rust coordinates connection, input translation, and control.

- `src/main.rs` — commands, mirror profiles, application flow, and selftest.
- `src/logic.rs` — NEXT/PREV decisions, light-scroll accumulation, cooldown,
  swipe geometry, and the 400 ms swipe duration.
- `src/adb.rs` — pinned-serial `adb shell input` commands with a 5-second
  execution timeout.
- `src/discover.rs` — USB-first device selection, saved Wi-Fi address,
  `mdns-sd` discovery, real phone-size detection, and retry flow.
- `src/listener.rs` — direct macOS `CGEventTap` input handling, mirror-window
  detection, suppression of consumed wheel events, and automatic tap recovery.

The design and decision record is in
`docs/plans/2026-10-25-rust-stack-switch.md`.

The saved Wi-Fi address lives in `phone-ip.txt`. That file is local-only
and is never committed.

## Troubleshooting

**Nothing seems to launch**

`ScreenGood.app` has no window of its own. Look for the separate `scrcpy`
mirror window; it may have opened behind another app. Avoid launching extra
copies while one mirror is already running.

**Phone not found on Wi-Fi**

Wireless debugging rotates its port. Keep Wireless debugging on, return to the
terminal prompt, and press ENTER. The app searches by mDNS and saves the new
address when it connects.

**USB-C phone is not detected**

Unlock the phone, confirm USB debugging is enabled, accept the computer prompt
on the phone, and use a data-capable cable rather than a charge-only cable.

**SHIFT+wheel does nothing**

Check both Input Monitoring and Accessibility for `ScreenGood.app`, then reopen
the app. Run `screengood wheel-probe` to see whether wheel events reach the
listener at all.

**Pause fires twice**

Clicks over the mirror are intentionally left to `scrcpy`. If pause still
double-fires, the mirror window was probably misdetected; bring the mirror to
the front and try again.

**Two phone entries in `adb devices`**

An IP entry plus an mDNS entry for one phone is normal. The app pins one serial
for every `adb` command so commands cannot split across transports.
