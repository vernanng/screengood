<p align="center">
  <img src="assets/icon-1024.png" width="160" height="160" alt="ScreenGood logo — Android holding a screen">
</p>

# ANDROID SCREEN MIRROR INSIDE MAC WITH CONTROL

[![CI](https://github.com/vernanng/screengood/actions/workflows/ci.yml/badge.svg)](https://github.com/vernanng/screengood/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/vernanng/screengood)](https://github.com/vernanng/screengood/releases/latest)

**Wi-Fi or USB-C.**

See your Android phone inside a window on your Mac and control the entire
phone with your Mac mouse — every app, every screen — without touching the
phone:

**click: tap · mouse scroll: up/down · onscreen scroll · offscreen scroll
with SHIFT+SCROLL**

One Rust binary does the work. It finds the phone, opens the mirror with
[`scrcpy`](https://github.com/Genymobile/scrcpy), and turns mouse-wheel moves
into precise up/down phone swipes over
[`adb`](https://developer.android.com/tools/adb), so scrolling behaves the
same inside any app. Tuned on a Samsung S25 FE, including while the same
phone is also mirrored to an external monitor.

## Download

Grab the latest build from
[Releases](https://github.com/vernanng/screengood/releases)
(Apple Silicon, macOS 13 or newer):

1. Download `ScreenGood-vX.Y.Z-macOS-arm64.zip` and unzip it.
2. Move `ScreenGood.app` to `/Applications`.
3. First launch only: **right-click the app → Open** (the build is ad-hoc
   signed, not notarized).
4. Do the [one-time setup](#one-time-setup) below, then just double-click
   the app from then on.

Building from source instead? See
[Build from source](#build-from-source).

## Quick start

1. Connect the phone by USB-C, or put phone and Mac on the same Wi-Fi with
   Wireless debugging on.
2. Double-click **ScreenGood.app**.
3. The phone screen appears in a Mac window. Control it with the mouse.

USB-C is picked automatically whenever a cable-connected phone is available.
Without a cable, the app tries the last saved Wi-Fi address, then searches the
local network (mDNS `_adb-tls-connect`) and saves the address it finds.

## Wi-Fi or USB-C

| Connection | Experience | Mirror profile |
|---|---|---|
| **USB-C** | Preferred. Near-zero delay and the steadiest control. | `--max-size 1440` (664×1440 on the S25 FE), H.265, 12 Mbps, up to 60 fps, 0 ms video buffer, 30 ms raw-audio buffer, Metal renderer |
| **Wi-Fi** | No cable. Extra buffering absorbs network jitter. | `--max-size 900` (414×900), 100 ms video buffer, 100 ms audio buffer |

The frame is intentionally smaller than the phone's native 1080×2340 screen
to keep encoding and decoding delay low. On the S25 FE, the hardware HEVC
encoder held 59–61 fps at the USB profile even while another encode was
running (mirroring out to a monitor at the same time).

## Controls

| Input | Action on the phone |
|---|---|
| **Click** (inside the mirror) | Tap |
| **Drag** (inside the mirror) | Swipe |
| **Right-click** (inside the mirror) | Android Back |
| **Onscreen scroll** — plain wheel, mouse on the mirror | Scroll down / up (NEXT / PREV) |
| **Offscreen scroll — SHIFT + SCROLL**, mouse anywhere else | Scroll down / up (NEXT / PREV) |
| **SHIFT + middle-click** or **SHIFT + right-click**, away from the mirror | Tap screen center (pause/resume in video players) |

Onscreen scroll and offscreen SHIFT+SCROLL are one and the same function:
wheel movement is accumulated in light-scroll steps and fired as one tuned
**400 ms swipe** per step (`SHIFT+wheel` is the reference behaviour; the
on-screen wheel was made to match it exactly). This works in any app, not
just one feed or player.

Clicks on the mirror itself always belong to `scrcpy`; the extra listener
deliberately steps aside there so one click can never land as a double tap.

Watching on a monitor is optional and independent: mirror the phone to
your monitor separately (wireless display or cable) if you want the same
screen there while you control it from the Mac.

## One-time setup

### Android

For USB-C:

1. Enable Developer options.
2. Enable **USB debugging**.
3. Connect with a data-capable USB-C cable.
4. Unlock the phone and accept the USB-debugging prompt if shown.

For Wi-Fi:

1. Put the phone and Mac on the same Wi-Fi network.
2. Enable Developer options.
3. Enable **Wireless debugging**.
4. Keep the phone screen on for the first connection.

For monitor viewing, mirror the phone to the monitor (wireless display or cable).

### Mac permissions

The wheel listener needs ScreenGood allowed in both of these:

- **Settings → Privacy & Security → Input Monitoring**
- **Settings → Privacy & Security → Accessibility**

Every launch **checks first**. The app silently preflights both permissions:

- **Both granted** → no dialog, no Settings window, the mirror just opens.
  Ordinary launches never ask again.
- **One missing** → only that one is requested. The app registers itself in
  just that list (no `+` button or file picker needed) and opens just that
  pane. Switch ScreenGood **on** there, then reopen the app. macOS does not
  let any app flip its own security switch, so that click is yours — once.

The mirror itself (click/tap/drag through `scrcpy`) works even without these
grants; only the wheel NEXT/PREV controls need them.

Two things worth knowing:

- The app is ad-hoc signed, so macOS ties grants to the exact build. A
  *downloaded release never rebuilds*, so this is a one-time setup there.
  If you rebuild from source with `make-app`, grant both permissions once
  more for the new build.
- After a rebuild the Settings rows can show **on** yet not take effect
  (the saved grant belongs to the previous build's code hash). Fix it by
  clearing the stale entries, then reopening the app so it re-registers:

  ```sh
  tccutil reset Accessibility com.screengood.app
  tccutil reset ListenEvent com.screengood.app
  ```

Each launch logs its preflight result (and any `scrcpy` output) to
`~/Library/Logs/ScreenGood.log` — Finder launches have no terminal, so that
file is the first place to look when something seems off.

### Tools

`adb` and `scrcpy` stay external. If they are missing, the app installs the
Homebrew packages `android-platform-tools` and `scrcpy` automatically
([Homebrew](https://brew.sh) itself must be installed). No Python is needed
at runtime.

## Build from source

Requires a Rust toolchain (pinned by `rust-toolchain.toml`: stable with
`rustfmt` and `clippy`) and macOS 13+.

```sh
cargo build --release
./target/release/screengood make-app   # assembles + ad-hoc signs ScreenGood.app
open ScreenGood.app
```

Terminal alternative (no app bundle):

```sh
./target/release/screengood            # mirror mode is the default
```

Package a release zip the same way the GitHub release is made:

```sh
ditto -c -k --sequesterRsrc --keepParent ScreenGood.app ScreenGood-macOS-arm64.zip
```

## Commands

```sh
./target/release/screengood              # mirror on the Mac (default)
./target/release/screengood mirror       # same, explicit
./target/release/screengood listen       # wheel/click listener only, no mirror window
./target/release/screengood test         # built-in selftest; no phone needed
./target/release/screengood wheel-probe  # 20 s raw wheel/click event log (diagnostics)
./target/release/screengood click-probe  # click-focused probe alias
./target/release/screengood make-app     # assemble + ad-hoc sign ScreenGood.app
```

Tests and checks (no phone needed):

```sh
cargo test                        # 28 unit tests
cargo clippy -- -D warnings
cargo fmt --check
./target/release/screengood test  # prints SELFTEST OK - DOWN=NEXT, UP=PREV
```

`make-app` signs ad hoc (`codesign --sign -`, identifier
`com.screengood.app`) and bundles `assets/AppIcon.icns` when present. No
keychain signing identity is used.

## How it works

`screengood` is a single Cargo binary (Rust 2021). It coordinates `adb` and
`scrcpy`; it does not replace them.

| File | Role |
|---|---|
| `src/main.rs` | Commands and app flow: mirror profiles, permission check-then-request, single-instance replacement, GUI log redirect, `make-app`, selftest |
| `src/discover.rs` | Phone discovery: USB-first, saved Wi-Fi address (`phone-ip.txt`), mDNS `_adb-tls-connect._tcp.local.`, real screen size via `wm size` |
| `src/listener.rs` | Direct `CGEventTap` (via `core-graphics`): wheel/click handling, mirror-window detection, consumed-event suppression, tap auto-recovery, permission preflight/request |
| `src/logic.rs` | Pure decisions: NEXT/PREV mapping, light-scroll pixel accumulation (6 px step), 900 ms cooldown, swipe geometry, 400 ms swipe duration |
| `src/adb.rs` | Pinned-serial `adb -s <serial> shell input` commands with a 5 s timeout |
| `assets/` | App icon: `icon-1024.png` master + `AppIcon.icns` (made with `sips` + `iconutil`) |

A few behaviours exist because of hard-won macOS facts:

- **Finder has no Homebrew in `PATH`.** A double-clicked app inherits a
  minimal GUI `PATH`, and `scrcpy` spawns `adb` by bare name — that used to
  make double-clicks silently do nothing. `screengood` now passes `scrcpy`
  the absolute `adb` path via the `ADB` environment variable (plus a
  Homebrew-prefixed `PATH`).
- **One mirror at a time.** Launching while a mirror is running replaces the
  old instance instead of two processes fighting over the phone's one
  screen capture.
- **Check before asking.** Permissions are preflighted silently on every
  launch; see [Mac permissions](#mac-permissions).

Design history (the Rust stack switch, decisions, and parity checks) is in
[`docs/plans/2026-10-25-rust-stack-switch.md`](docs/plans/2026-10-25-rust-stack-switch.md).

Local-only files, never committed: `phone-ip.txt` (saved Wi-Fi address),
`ScreenGood.app` and `target/` (build outputs).

## Troubleshooting

**Double-click does nothing**

`ScreenGood.app` has no window of its own — look for the separate `scrcpy`
mirror window, which can open behind other apps. Then read
`~/Library/Logs/ScreenGood.log`; every GUI launch and its `scrcpy` output
land there. If a permission is missing, System Settings opens instead (see
[Mac permissions](#mac-permissions)).

**Settings shows ScreenGood on, but it still asks**

The grant is stale: it was saved for a previous build's code hash. Run the
two `tccutil reset` commands in [Mac permissions](#mac-permissions), reopen
the app, and toggle on once for the current build.

**SHIFT+wheel does nothing**

Confirm both Input Monitoring and Accessibility are on for
`ScreenGood.app`, then reopen the app (grants apply to new launches). The
log's preflight line tells you exactly which one macOS still reports as
missing. `./target/release/screengood wheel-probe` shows whether wheel
events reach the listener at all.

**Phone not found on Wi-Fi**

Wireless debugging rotates its port. Keep Wireless debugging on; from a
terminal run, press ENTER at the retry prompt. The app searches by mDNS and
saves the new address to `phone-ip.txt` when it connects.

**USB-C phone is not detected**

Unlock the phone, confirm USB debugging is on, accept the computer prompt
on the phone, and use a data-capable cable rather than a charge-only one.

**Center-tap (pause) fires twice**

Clicks over the mirror are intentionally left to `scrcpy`. If a
SHIFT+click still double-fires, the mirror window was probably
misdetected — bring the mirror to the front and try again.

**Two phone entries in `adb devices`**

An IP entry plus an mDNS (`_tcp`) entry for one phone is normal. The app
pins one serial for every `adb` command, so input cannot split across
transports.

