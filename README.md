# Reels on TV, mouse on Mac

Control Facebook Reels on an Android phone (written for a Samsung S25 FE) from a
Mac mouse while the phone mirrors to a Samsung TV with Smart View. Everything is
`adb shell input` swipe/tap — no touching the phone. One Rust binary, `screengood`;
mirror is the only screen mode.

## Run it

Double-click **ScreenGood.app** (scrcpy mirror + SHIFT+wheel listener), or from a
terminal:

```sh
./target/release/screengood          # = mirror (default)
./target/release/screengood listen   # listener only, no mirror
```

The phone screen shows on the Mac; mouse click = tap. The mirror profile is picked
by transport (cable is preferred whenever present): **USB = near-zero delay** —
664×1440 H.265 at 12 Mbps, 60 fps cap, video buffer 0, raw audio with a 30 ms
buffer. On the S25 FE the hardware HEVC encoder measured steadier than H.264 at
this size and up (59–61 fps sustained even while a second encode was running), and
the smaller-than-native frame keeps encode/decode time near the floor.
**WiFi = smooth profile** — 414×900 with 100 ms video/audio buffers, kept for
jitter absorption.

Controls:

- In the scrcpy mirror: click = tap · drag = swipe · right-click = Android Back ·
  plain wheel while the cursor is on the mirror screen = NEXT/PREV
- Anywhere on the Mac: light **SHIFT+wheel** down = NEXT, up = PREV ·
  **SHIFT+middle-click** (or SHIFT+right-click) = video pause/resume
- SHIFT+click over the mirror is left to scrcpy, so nothing double-taps.

## Build it

Needs a Rust toolchain ([rustup](https://rustup.rs)):

```sh
cargo build --release
./target/release/screengood make-app   # assembles + codesigns ScreenGood.app
cargo test                             # 25 unit tests, no phone needed
./target/release/screengood test       # binary selftest (same cases as the old --test)
./target/release/screengood wheel-probe  # 20s: log raw wheel/click events
```

`make-app` signs ad-hoc with the stable identifier `com.screengood.app`, so the
one-time macOS permission grant below survives rebuilds.

## Setup (one time)

1. Phone and Mac on the same WiFi, or USB cable (preferred whenever present). On
   the phone: Wireless debugging ON (Developer options), Smart View to the TV,
   Reels fullscreen, screen on.
2. The binary finds the phone itself: USB first, then the saved address in
   `fb-reels-tv-ip.txt`, then mDNS (`_adb-tls-connect`). A freshly found address is
   saved back to that file. It is never committed.
3. macOS permission for the SHIFT+wheel listener: allow **ScreenGood.app** in
   Settings → Privacy → Input Monitoring **and** Accessibility, then reopen the
   app. (Older bash versions asked for Terminal; the grant now belongs to the app
   itself.) The binary detects a missing grant and opens the right panes. The
   mirror keeps working without it — only SHIFT+wheel / wheel-on-mirror stepping
   needs the listener.
4. Dependencies: Homebrew `android-platform-tools` and `scrcpy` — auto-installed
   if missing. No Python at runtime.

## Where the code lives — read this before editing

`screengood` is a single Cargo binary; `adb` and `scrcpy` stay external tools it
orchestrates.

- `src/main.rs` — subcommands (`mirror` default, `listen`, `test`, `click-probe`,
  `make-app`), scrcpy profiles, the printed selftest.
- `src/logic.rs` — pure decisions: wheel → NEXT/PREV, light-scroll accumulation
  (~6 px per step), cooldown, swipe/tap geometry scaled from the phone's real
  screen size (`adb shell wm size`; S25 FE 1080×2340 is the fallback).
- `src/adb.rs` — builds and runs `adb -s <serial> shell input …` (serial pinned
  for every command, 5 s timeout).
- `src/discover.rs` — tool resolution (Homebrew prefix), device/serial picking
  (USB first), saved-IP + `mdns-sd` discovery, ENTER-to-retry connect loop.
- `src/listener.rs` — the global event listener: a direct `CGEventTap` (pixel
  deltas, event suppression, self re-arm) plus scrcpy-window detection via
  `CGWindowListCopyWindowInfo`.

The plan and decision record for the Rust switch live in
`docs/plans/2026-10-25-rust-stack-switch.md`.

## Troubleshooting

- **Phone not found, port changed:** wireless debugging rotates its port. Leave
  Wireless debugging ON and press ENTER — mDNS search finds and re-saves it.
- **SHIFT+wheel does nothing:** Input Monitoring/Accessibility not allowed for
  ScreenGood.app (see Setup 3). `screengood click-probe` shows whether clicks
  are seen at all.
- **Pause fires twice / unpauses instantly:** only over the scrcpy mirror if
  scrcpy is misdetected; the listener steps aside there on purpose.
- **Two `adb devices` entries for one phone** (IP + mDNS) is normal; the binary
  pins one serial for every command.
