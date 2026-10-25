# Switch the code stack to Rust — Draft plan (under grill)

Status: **Final** — grill settled Q1–Q6 as recommended (Q1 amended: no bash); accepted by user via `go` on 2026-10-25. **Scope amendment (same day, user instruction): pad mode removed entirely — mirror is the only screen mode.** Q4 below is superseded; `src/pad.rs`, the `pad` subcommand, `to_phone`, and the `objc2` dependencies were dropped before any pad code landed.

Implementation status: steps 1–4 done and verified (22 unit tests, clippy/fmt clean, binary selftest OK, `ScreenGood.app` codesigned `com.screengood.app`, live USB discovery prints `Phone OK`; old launchers' selftests still pass). Step 5 (cutover deletion) is gated on the Q6 manual parity pass: grant `ScreenGood.app` Input Monitoring + Accessibility once, run one real mirror session, confirm, then delete the three legacy files and rewrite README.

## Goal

Replace the current bash + embedded-Python stack with Rust, keeping identical user-visible behavior: double-click to mirror/control Facebook Reels on the S25 FE from the Mac mouse (`adb shell input` swipe/tap; phone mirrors to TV via Smart View).

Current stack (verified in-repo): `Android-Mouse.command` (259 lines), `Reels-Scroll.command` (374), `fb-reels-lib.sh` (118) are the only tracked code (`.gitignore` is an allowlist of 5 files). Each launcher embeds a Python helper in a heredoc, rewritten every run: `fb-reels-scroll.py` (pynput listeners + Quartz `CGEventTap` light-scroll + `CGWindowList` scrcpy detection) and `fb-reels-pad.py` (PyObjC AppKit `NSWindow`/`NSView` pad). Shared bash does tool resolution (Homebrew prefix), `pynput` `.venv` bootstrap, phone discovery (`dns-sd` mDNS `_adb-tls-connect`, saved-IP file, USB-first serial pick), permission probe, and `wm size` detection. External binaries: `adb`, `scrcpy`. Rust toolchain is not installed on this Mac (`rustc`/`cargo` not found).

## Success Criteria

- Every control works as today: pad click/drag/wheel/right-click/Space/↑↓; SHIFT+wheel NEXT/PREV anywhere incl. light (~6px) scrolls; SHIFT+middle/right-click pause; plain wheel over the scrcpy mirror = NEXT/PREV; SHIFT+click over mirror never double-taps.
- Discovery parity: USB cable preferred, then saved `fb-reels-tv-ip.txt`, then mDNS; one pinned serial for all `adb` calls; swipe/tap points scaled from real `wm size` (1080×2340 fallback).
- Mirror profiles unchanged: USB 664×1440 H.265 12M/60fps/buffer 0/raw audio 30ms/Metal; WiFi 414×900 with 100ms video/audio buffers.
- `--test` and `--click-probe` equivalents pass with no phone attached; old launchers removed only after parity is demonstrated on the real phone + TV.
- No Python, `pynput`, PyObjC, or `.venv` at runtime; setup is one `cargo build --release` (plus existing `adb`/`scrcpy`).

## Approach

**End state (settled, grill Q1 = A, amended: no bash):** one Cargo binary `screengood` with subcommands `mirror` (default), `pad`, `listen`, `test`, `click-probe`, double-clicked via a minimal `ScreenGood.app` bundle with the binary as its executable — zero shell scripts in the end state. `adb` and `scrcpy` stay external binaries (settled, grill Q2 = A) — reimplementing the ADB protocol or scrcpy's encoder pipeline is out of scope. Rust replaces only what bash/Python do today: discovery, event capture, pad window, orchestration.

**Listener (settled, grill Q3 = A):** port the Quartz tap 1:1 with `core-graphics` (`CGEventTap` at `kCGSessionEventTap`/`kCGHeadInsertEventTap`, point/fixed-point delta fields, `CGEventTapEnable` re-arm) on a `core-foundation` runloop thread, and `CGWindowListCopyWindowInfo` for scrcpy bounds (0.5s cache). `rdev` is rejected: event suppression needs its `unstable_grab` feature and it pins older `core-graphics`/cocoa crates; the current code's correctness lives in suppression + pixel deltas, which the direct tap already proves. Pure logic (`decide`, pixel accumulation, cooldown, coordinate scaling) becomes a testable module with no FFI.

**Pad (settled, grill Q4 = A):** native AppKit via `objc2` + `objc2-app-kit` — same `NSView` subclass shape (`drawRect`, `mouseDown/Dragged/Up`, `rightMouseDown`, `scrollWheel`, `keyDown`), same 350×758 window and `to_phone` mapping. `winit` is rejected: it needs a separate renderer (e.g. softbuffer) to reproduce trivial AppKit drawing and buys cross-platform support this macOS-only tool cannot use.

**Discovery:** browse `_adb-tls-connect._tcp` in-process with `mdns-sd` (no async runtime needed) instead of backgrounding `dns-sd` and parsing temp files; keep saved-IP-first, USB-first, retry-on-ENTER flow and the `/tmp` temp-file behavior disappears with it.

**Permissions (settled, grill Q6):** today Terminal holds Input Monitoring + Accessibility; the Rust binary needs both grants **for itself** (one-time re-grant, same Settings panes, binary detects failure when tap creation returns null and opens the panes; pad keeps needing none). The release binary is ad-hoc codesigned with a stable identifier and installed to a fixed path so rebuilds keep their TCC grants instead of forcing a re-grant per build.

**Repo shape:** flip `.gitignore` from allowlist to a normal Rust ignore (`/target`, generated helpers gone, `fb-reels-tv-ip.txt` still untracked); track `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `src/`, README, this plan dir (`ScreenGood.app` is regenerated by `screengood make-app` and stays untracked).

## Steps

1. **Toolchain + skeleton** — install `rustup` (user-run or approved command), pin stable in `rust-toolchain.toml`; `cargo init` binary `screengood`; port pure logic first (`decide`, `accumulate_pixels`, `wheel_decision` inputs, `to_phone`, swipe-point scaling) with `cargo test` cases copied from both heredoc selftests. Old launchers untouched.
2. **adb + discovery module** — `std::process` wrapper (pinned `-s SERIAL`, 5s timeout, rc logging as today); `mdns-sd` browse + saved-IP + `pick_serial` + `wm size`; demo via `screengood test` printing the same NEXT/PREV/VIDEO command vectors as the Python selftest asserts.
3. **Listener** — `core-graphics` tap thread + click/key handling + scrcpy-bounds check behind a small event-source trait so logic tests stay FFI-free; `screengood click-probe` parity (20s log). Verify: light-scroll 3×−2px → one NEXT; SHIFT+click over mirror → suppressed; cooldown blocks double-fire.
4. **Pad + orchestration** — `objc2-app-kit` pad; `mirror` mode spawns listener as a thread (not a child Python), launches `scrcpy` with the two existing profiles, cleans up on exit signals; assemble `ScreenGood.app` (binary + `Info.plist`, ad-hoc codesigned, stable identifier) as the bash-free double-click entry.
5. **Cutover** — real-device parity pass (checklist in Validation), then delete both `.command` launchers, `fb-reels-lib.sh`, the heredocs, and the `.venv` bootstrap so no bash or Python remains; rewrite README setup (binary grant replaces Terminal grant), flip `.gitignore`. Keep the old three files in git history as the rollback.

Migration is strangler-style (settled, grill Q5 = A): steps 1–4 never break the current double-click flow (old bash survives only as the temporary fallback); deletion happens only in step 5 after evidence.

## Validation Plan

- `cargo test` — all ported selftest cases (wheel mapping, `-s` prefix coords, on-screen points, no-double-tap, mirror-wheel, light scroll, pad map corners/center) green before any device test.
- `cargo clippy -- -D warnings` and `cargo fmt --check` clean.
- `./Reels-Scroll.command --test` and `./Android-Mouse.command --test` still pass until cutover (proves the fallback stayed intact during the port).
- Manual, phone + TV (highest risk, cannot automate): `screengood click-probe` sees SHIFT+middle/right clicks; USB mirror holds ~60fps profile; WiFi discovery after port rotation re-saves the IP file; SHIFT+wheel over mirror steps once (no double-fire); pad center-click pauses; permission-denied run opens the right Settings panes.

## Risks / Open Questions

- **Permission identity change** is the top risk: grants move from Terminal to the `screengood` binary; rebuilds can lose TCC grants — mitigated per Q6 (ad-hoc codesign, stable identifier, fixed path inside `ScreenGood.app`); if grants still reset, the binary's open-the-Settings-panes flow is the fallback.
- `unsafe` FFI surface (event tap, AppKit) replaces Python's dynamic safety; contained by keeping FFI behind modules and all decisions in tested pure code.
- Install size/toolchain: first build needs network for crates.io; offline rebuilds fine afterwards.
- Grill complete: Q1–Q6 all settled as recommended (A ×6), with Q1 amended to **no bash** in the end state. Document stays Draft until explicit user acceptance.

## Sources

- https://docs.rs/core-graphics/latest/core_graphics/event/index.html
- https://docs.rs/rdev/latest/rdev/
- https://docs.rs/mdns-sd/latest/mdns_sd/
- https://docs.rs/objc2/latest/objc2/
- https://docs.rs/winit/latest/winit/
- https://rust-lang.github.io/rustup/

## Grill Record

- Q1 end-state shape (single binary + shims / two binaries / keep bash): **settled — A as amended ("no bash"): one `screengood` binary (`mirror`/`pad`/`listen`/`test`/`click-probe`), double-click via minimal `ScreenGood.app`; zero shell scripts in the end state**
- Q2 `adb` + `scrcpy` stay external: **settled — A: both stay external Homebrew binaries; Rust spawns `adb -s SERIAL …` and `scrcpy` with today's profiles; no native ADB protocol, no own mirror**
- Q3 event capture (`core-graphics` direct / `rdev`): **settled — A: direct `CGEventTap` via `core-graphics` + `core-foundation` runloop, 1:1 with today's Quartz tap (suppression, pixel deltas, re-arm, `CGWindowList` mirror check); `rdev` rejected**
- Q4 pad window (`objc2-app-kit` / `winit`): **superseded — pad mode removed by user instruction (mirror-only); no pad window is built in any stack**
- Q5 migration (strangler / big-bang): **settled — A: strangler; old launchers stay runnable through step 4, deleted only in step 5 after the real-device parity pass; rollback = git history**
- Q6 done bar + permission/codesign handling: **settled — recommended: done = all ported selftests green in `cargo test` + clippy/fmt clean + full manual phone+TV parity checklist passed before any deletion; binary ad-hoc codesigned with stable identifier at a fixed path (`ScreenGood.app`) so TCC grants survive rebuilds; one-time re-grant from Terminal to the binary accepted**

## Settled Scope Contract

- **In scope (artifacts):** `Cargo.toml`/`Cargo.lock`/`rust-toolchain.toml`, `src/` (one `screengood` binary), `ScreenGood.app` assembly, flipped `.gitignore`, rewritten `README.md`, this plan. At cutover only: deletion of `Android-Mouse.command`, `Reels-Scroll.command`, `fb-reels-lib.sh`.
- **Out of scope:** native ADB protocol, replacing `scrcpy` or Smart View mirroring, non-macOS support, new controls/features, any phone-side app.
- **Done means:** Q6 bar above — ported selftests + clippy/fmt + manual parity checklist, then deletion lands; no bash or Python remains tracked or needed at runtime.
- **Staging:** steps 1–5 execute in order; accepting this plan approves the plan only — implementation starts on a separate `go`.
