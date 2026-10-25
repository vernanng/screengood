# AGENTS.md

ScreenGood mirrors an Android phone inside a Mac window (USB-C or Wi-Fi) and
controls the whole phone with the Mac mouse. One Rust binary, `screengood`,
coordinates the external Homebrew tools `adb` and `scrcpy`. Mirror mode is
the only screen mode (pad mode was removed by decision, 2026-10-25); keep
docs and code general-purpose — never reintroduce app-specific wording
(Reel/Facebook).

## Commands

- `cargo test --locked` — run the full test suite
- `cargo clippy --all-targets --locked -- -D warnings` — run the lint task
- `cargo fmt --check` — run the format task
- `cargo build --release --locked` — build the project
- `./target/release/screengood make-app` — assemble + sign
  `ScreenGood.app` next to the project dir (icon from `assets/` when
  present; stable local identity when `setup-signing` was run, else
  ad-hoc).
- `./target/release/screengood install` — assemble the same app (icon
  embedded) directly into `/Applications`, or `~/Applications` when the
  system folder is not writable; `SCREENGOOD_INSTALL_DIR` overrides the
  target. The free no-Gatekeeper install path: `cargo install --git
  https://github.com/vernanng/screengood --locked` then `screengood
  install` (locally built apps carry no download quarantine).
- Release packages: native macOS app as a ZIP via `ditto -c -k
  --sequesterRsrc --keepParent` (the only release asset).
  Publish/update with `gh release` on `vernanng/screengood`, and keep the
  release notes' install block, asset table, and SHA-256s accurate.
- `./target/release/screengood setup-signing` — one-time, user-run:
  creates the free `ScreenGood Local Signing` identity in a dedicated
  keychain file under `~/Library/Application Support/ScreenGood/` (login
  keychain never touched) so grants survive rebuilds. Never auto-run it:
  trusting the cert pops a macOS password dialog only the user can answer.
- Subcommands: `mirror` (default), `listen`, `settings`, `test`,
  `wheel-probe`, `click-probe`, `make-app`, `install`, `setup-signing`.
  No bash anywhere in the product.

## CI & releases

- `.github/workflows/ci.yml` runs on every push to `main`, on PRs, and
  on `v*` tag pushes: `macos-latest` (mandatory — the crate links macOS
  frameworks and will not compile on Linux), 25 min timeout, then fmt →
  clippy (`--all-targets -D warnings`) → `cargo test --locked` →
  release build → selftest → `make-app` + codesign verify → bundle
  version check (plist vs `Cargo.toml`) → Cargo-install smoke test
  (`cargo install --path .`, `install` to a temp dir, verify + selftest)
  → zip artifact. Keep local gates identical.
- Pushing a `v*` tag additionally runs the `release` job (10 min
  timeout, `contents: write`): it re-verifies the artifacts, stages
  versioned filenames, and publishes the GitHub release live with
  generated notes + SHA-256s (`--verify-tag` guards the tag target).
- Version is single-sourced: `Cargo.toml` `version` only; the plist
  `CFBundleShortVersionString` is generated from
  `env!("CARGO_PKG_VERSION")` by `assemble_app`. Any binary *or plist*
  change produces a new ad-hoc code hash, which invalidates TCC grants
  (see invariants) — bundle changes so users only re-grant once per
  release, and say so in release notes.
- CI artifact ZIPs are ad-hoc-signed CI builds for smoke-testing
  the packaging path; user-facing downloads are the GitHub Release
  assets.

## Code Map

- `src/main.rs` — CLI + app flow: mirror profiles, `MIRROR_WINDOW_TITLE`
  (short project-name title), spawns the floating title bar beside
  scrcpy, permission
  check-then-request (preflight runs before any phone/mirror work;
  missing-only prompt via `panes_for_missing`, silent re-check at
  spawn), `kill_previous_instances` (single instance by
  replacement), `child_env_path`, `redirect_output_to_log_if_gui`,
  `assemble_app`/`sign_app` shared bundle assembly (`make_app` with file
  icon, `install_app` with embedded icon, `resolve_sign_mode` picking
  the `setup_signing` local identity when present), selftest.
- `src/listener.rs` — `CGEventTap` via `core-graphics`; `check_permissions`
  (silent preflight), `request_permissions` (missing only), `scrcpy_at`
  mirror-window detection, `scrcpy_windows`/`scrcpy_window_bounds_for_pid`
  for the title bar's display-rate follow loop, lifetime `run`/`spawn`.
- `src/logic.rs` — pure constants/decisions: `PIXEL_THRESHOLD = 6.0`,
  `COOLDOWN = 900 ms`, `SWIPE_DUR_MS = 400` (the settings defaults),
  `Geometry`, `wheel_route`.
- `src/settings.rs` — scroll-feel settings storage: `ScrollSettings`
  (`scroll_step_px`/`cooldown_ms`/`swipe_ms`), forgiving `key = value`
  parse + clamp, `settings_path`/`load`/`save_to`
  (`~/Library/Application Support/ScreenGood/settings.txt`,
  `SCREENGOOD_SETTINGS` override). Read once at launch.
- `src/settings_window.rs` — native AppKit Settings window via `objc2`
  (upstream `hello_world_app` delegate pattern): three sliders, Save
  writes the settings file, applies on next launch, never touches the
  phone.
- `src/title_bar.rs` — floating Settings tab (`title-bar` helper
  spawned by mirror mode): gear Settings button on the left edge of
  the mirror (title bar stays draggable), sticks to the scrcpy window
  via Accessibility move/resize notifications (owner-pid bounds poll
  as fallback), opens `settings` as a
  sibling process, exits with the mirror. The scrcpy window is another
  process's, so this attached panel — not a literal title-bar button —
  is the supported shape; native close/minimize stay clickable.
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
  cooldown, swipe). `SWIPE_DUR_MS = 400` is the user-tuned default;
  `screengood settings` is the explicit, user-driven way to change the
  effective step/cooldown/swipe values — defaults never change silently,
  and both wheel routes always share the one configured path.
- **Check before asking for permissions.** Preflight Accessibility and
  Input Monitoring separately on every launch, before any phone or
  mirror work; request/open a Settings pane only for one that is
  missing; never re-request a granted one.
- **Signing is local: ad-hoc by default, stable after `setup-signing`.**
  With ad-hoc signing TCC keys grants to the build's code hash: after a
  rebuild that changes code, Settings can show "on" while preflight still
  fails ("Failed to match existing code requirement" in the unified
  log). Recovery: `tccutil reset Accessibility com.screengood.app` and
  `tccutil reset ListenEvent com.screengood.app`, relaunch, toggle once.
  Downloaded releases never rebuild, so this is one-time setup there. The
  permanent fix for source installs is the `ScreenGood Local Signing`
  identity from `screengood setup-signing` (dedicated keychain file under
  `~/Library/Application Support/ScreenGood/`; the login keychain is
  never used, and the identity is never created silently — trusting the
  cert needs the user's password dialog). Once present, every build is
  signed with it and grants survive rebuilds. Never make identity
  creation automatic, and never fall back to ad-hoc silently when the
  identity exists but signing fails.
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
