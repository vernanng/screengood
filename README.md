# Reels on TV, mouse on Mac

Control Facebook Reels on an Android phone (written for a Samsung S25 FE) from a
Mac mouse while the phone mirrors to a Samsung TV with Smart View. Everything is
`adb shell input` swipe/tap over WiFi — no touching the phone.

## Run it

Double-click a launcher (or run it from Terminal):

- `Android-Mouse.command` — main launcher. Default **B mode**: no video mirror on
  the Mac (so no scrcpy lag/crackle; sound comes from the TV). A blank
  phone-shaped pad window opens; use the mouse on it and watch the TV.
  `./Android-Mouse.command --mirror` uses the old scrcpy mirror instead.
- `Reels-Scroll.command` — listener only, no pad/mirror.

Controls:

- Pad: click = tap same spot (center = pause) · drag = swipe ·
  wheel = NEXT/PREV · right-click = Android Back · Space = pause · ↑/↓ = PREV/NEXT
- Anywhere on the Mac: light **SHIFT+wheel** down = NEXT, up = PREV ·
  **SHIFT+middle-click** (or SHIFT+right-click) = video pause/resume
- In the scrcpy mirror (`--mirror`): click/drag work as touch; plain wheel is
  disabled there, and SHIFT+click is left to scrcpy so nothing double-taps.

## Setup (one time)

1. Phone and Mac on the same WiFi. On the phone: Wireless debugging ON
   (Developer options), Smart View to the TV, Reels fullscreen, screen on.
2. The launchers find the phone themselves: saved address in
   `fb-reels-tv-ip.txt` first, then mDNS (`_adb-tls-connect`). A freshly found
   address is saved back to that file. It is never committed.
3. macOS permission for the SHIFT+wheel listener: allow Terminal in
   Settings → Privacy → Input Monitoring **and** Accessibility, then fully quit
   Terminal (Cmd+Q) and reopen. The launchers detect this and open the right
   panes. The pad needs no permission and keeps working without it.
4. Dependencies: Homebrew `android-platform-tools`, `python`, `scrcpy`
   (mirror only) — auto-installed if missing. `pynput` is used from the system
   Python if present, otherwise from a project `.venv` next to the launchers
   (created with `--system-site-packages` for PyObjC). Nothing installs with
   `--break-system-packages`.

## Where the code lives — read this before editing

**The two `.command` files are the source of truth and the only code in git.**
Each embeds its Python helper in a heredoc and rewrites it on every run:

- `Reels-Scroll.command` (`write_python`) → `fb-reels-scroll.py`
- `Android-Mouse.command` (`write_pad`) → `fb-reels-pad.py`

Editing a `.py` file directly is lost on the next launch. Edit the heredoc in
the `.command` file instead. `Android-Mouse.command` regenerates
`fb-reels-scroll.py` via `Reels-Scroll.command --test` on every launch too, so
listener fixes apply to B mode immediately.

Paths are relative to wherever the launchers live (Desktop today), tools are
found via Homebrew's prefix or `PATH`, and swipe/tap points are computed from
the phone's real screen size (`adb shell wm size`, exported as
`PHONE_W`/`PHONE_H`; S25 FE 1080×2340 is the fallback).

## Tests

Maintained selftests, no phone needed:

```sh
./Reels-Scroll.command --test     # wheel mapping, coords, no-double-fire, light scroll
./Android-Mouse.command --test    # pad coordinate mapping
python3 fb-reels-scroll.py --test # same as above, incl. with ADB_SERIAL set
python3 fb-reels-pad.py --test
./Reels-Scroll.command --click-probe  # 20s: are middle/right clicks seen at all?
```

## Troubleshooting

- **Phone not found, port changed:** wireless debugging rotates its port. Leave
  Wireless debugging ON and press ENTER — mDNS search finds and re-saves it.
- **SHIFT+wheel does nothing:** Input Monitoring/Accessibility not allowed (see
  Setup 3), or the Quartz tap fell back to line scroll (message on startup).
- **Pause fires twice / unpauses instantly:** only over the scrcpy mirror if
  scrcpy is misdetected; the listener steps aside there on purpose.
- **Two `adb devices` entries for one phone** (IP + mDNS) is normal; the
  launchers pin one serial (`ADB_SERIAL`) for every command.
