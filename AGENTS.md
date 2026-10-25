# AGENTS.md

ScreenGood mirrors an Android phone inside a Mac window (USB-C or Wi-Fi) and
controls the whole phone with the Mac mouse. One Rust binary, `screengood`,
coordinates the external Homebrew tools `adb` and `scrcpy`. Mirror mode is
the only screen mode (pad mode was removed by decision, 2026-10-25); keep
docs and code general-purpose — never reintroduce app-specific wording
(Reel/Facebook).

## Commands

- `cargo test` — 28 unit tests; no phone needed. Run after any change.
- `cargo clippy -- -D warnings` and `cargo fmt --check` — both must pass
  (toolchain pinned in `rust-toolchain.toml`: stable + rustfmt + clippy).
- `cargo build --release` — build; `./target/release/screengood test` must
  print `SELFTEST OK - DOWN=NEXT, UP=PREV`.
- `./target/release/screengood make-app` — assemble + ad-hoc sign
  `ScreenGood.app` (identifier `com.screengood.app`, icon from `assets/`).
- Install: `ditto ScreenGood.app /Applications/ScreenGood.app` (replace
  first). Release packages: zip via `ditto -c -k --sequesterRsrc
  --keepParent`, DMG via `hdiutil create -volname ScreenGood -srcfolder
  <stage with app + /Applications symlink> -format UDZO` (verify with
  `hdiutil verify` and a mount + `codesign --verify` before upload);
  publish/update with `gh release` on `vernanng/screengood`.
- Subcommands: `mirror` (default), `listen`, `test`, `wheel-probe`,
  `click-probe`, `make-app`. No bash anywhere in the product.

## CI & releases

- `.github/workflows/ci.yml` runs on every push to `main` and on PRs:
  `macos-latest` (mandatory — the crate links macOS frameworks and will
  not compile on Linux), then fmt → clippy (`--all-targets -D warnings`)
  → `cargo test --locked` → release build → selftest → `make-app` +
  codesign verify → zip + DMG artifacts. Keep local gates identical.
- Version lives in two places that must move together: `Cargo.toml`
  `version` and `CFBundleShortVersionString` in `make_app`'s plist.
  Any binary *or plist* change produces a new ad-hoc code hash, which
  invalidates TCC grants (see invariants) — bundle changes so users only
  re-grant once per release, and say so in release notes.
- CI artifact zips/DMGs are unsigned-CI builds for smoke-testing the
  packaging path; user-facing downloads are the GitHub Release assets.

## Code Map

- `src/main.rs` — CLI + app flow: mirror profiles, permission
  check-then-request, `kill_previous_instances` (single instance by
  replacement), `child_env_path`, `redirect_output_to_log_if_gui`,
  `make_app` (signing + icon), selftest.
- `src/listener.rs` — `CGEventTap` via `core-graphics`; `check_permissions`
  (silent preflight), `request_permissions` (missing only), `scrcpy_at`
  mirror-window detection, lifetime `run`/`spawn`.
- `src/logic.rs` — pure constants/decisions: `PIXEL_THRESHOLD = 6.0`,
  `COOLDOWN = 900 ms`, `SWIPE_DUR_MS = 400`, `Geometry`, `wheel_route`.
- `src/discover.rs` — `resolve_tools`, `connect_phone` (USB-first,
  `phone-ip.txt`, mDNS `_adb-tls-connect._tcp.local.`, `wm size`),
  `project_dir`.
- `src/adb.rs` — pinned-serial `adb -s <serial> shell input`, 5 s timeout.
- `assets/` — `icon-1024.png` + `AppIcon.icns` (regenerate with `sips` +
  `iconutil`). `docs/plans/` — design record. `README.md` — user docs.

## Conventions

- Rust 2021; `use` declarations at the top of each file; unit tests inline
  in `#[cfg(test)] mod tests` in the file they cover. When test count
  changes, update the count in `README.md`.
- User-facing control wording: "click: tap · mouse scroll: up/down ·
  onscreen scroll · offscreen scroll with SHIFT+SCROLL".
- Agent-made commits: author stays the repo default (Vernan Tjoa);
  committer is `Muse Code <muse-code@localhost>` via
  `GIT_COMMITTER_NAME` / `GIT_COMMITTER_EMAIL`.
- Ignored, never commit: `/target`, `/ScreenGood.app/`, `phone-ip.txt`.

## Invariants — do not break

- **SHIFT+wheel is the reference.** On-screen plain wheel and SHIFT+wheel
  anywhere share one NEXT/PREV path (same normalization, accumulator,
  cooldown, swipe). `SWIPE_DUR_MS = 400` is user-tuned; change only on
  explicit request.
- **Check before asking for permissions.** Preflight Accessibility and
  Input Monitoring separately on every launch; request/open a Settings
  pane only for one that is missing; never re-request a granted one.
- **Ad-hoc signing is deliberate** (no keychain identity). TCC therefore
  keys grants to the build's code hash: after a rebuild that changes code,
  Settings can show "on" while preflight still fails ("Failed to match
  existing code requirement" in the unified log). Recovery:
  `tccutil reset Accessibility com.screengood.app` and
  `tccutil reset ListenEvent com.screengood.app`, relaunch, toggle once.
  Downloaded releases do not rebuild, so this is one-time setup there.
- **Finder PATH has no Homebrew.** `scrcpy` spawns `adb` by bare name; the
  child must get `ADB=<absolute adb>` plus a Homebrew-prefixed `PATH`
  (`child_env_path`). `open` from a terminal forwards the shell PATH and
  therefore *masks* this class of bug — verify GUI launches the Finder way
  (`osascript -e 'tell application "Finder" to open POSIX file
  "/Applications/ScreenGood.app"'`) and read
  `~/Library/Logs/ScreenGood.log`, where non-terminal launches (and the
  `scrcpy` child) write all output.
- **One mirror at a time.** A second launch terminates the previous
  `screengood` + its `scrcpy` child rather than fighting over the phone's
  single screen capture.
- Clicks over the mirror belong to `scrcpy`; the listener must not also
  fire on them (double-tap). SHIFT+middle/right-click off-mirror = center
  tap (`video_cmd`).
