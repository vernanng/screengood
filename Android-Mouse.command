#!/bin/bash
# Android-Mouse.command - DEFAULT (double-click): scrcpy MIRROR.
#   The PHONE SCREEN shows on the Mac. Inside it: click = tap | drag = swipe | wheel = NEXT/PREV | right-click = Back
#   ANYWHERE: light SHIFT+wheel = NEXT/PREV | SHIFT+middle/right-click = VIDEO pause/resume (listener)
#   Explicit --mirror is accepted as an alias for this default. --test runs selftests (no phone needed).
DIR="$(cd "$(dirname "$0")" && pwd)"  # wherever this file lives (~/projects/screen-mirror today)
LIB="$DIR/fb-reels-lib.sh"
if [ ! -f "$LIB" ]; then echo "Missing $LIB - keep fb-reels-lib.sh next to this file."; read -p "Press Enter to close: " _; exit 1; fi
. "$LIB"  # shared: resolve_tools, ensure_pynput, pick_serial, auto_connect, check_trust, connect_phone
resolve_tools
IPFILE="$DIR/fb-reels-tv-ip.txt"
PYFILE="$DIR/fb-reels-scroll.py"
REELS="$DIR/Reels-Scroll.command"

if [ "${1:-}" = "--pad" ]; then
  echo "Pad mode was removed. Run with no arguments (or --mirror) for the scrcpy mirror."
  exit 1
fi

if [ "${1:-}" = "--test" ]; then
  fail=0
  echo "=== ANDROID-MOUSE SELFTEST (no phone needed) ==="
  for f in Android-Mouse.command Reels-Scroll.command fb-reels-lib.sh; do
    if bash -n "$DIR/$f"; then
      echo "PASS bash -n $f"
    else
      echo "FAIL bash -n $f"; fail=1
    fi
  done
  pad_existed_before=0
  [ -e "$DIR/fb-reels-pad.py" ] && pad_existed_before=1
  pad_out="$("$DIR/Android-Mouse.command" --pad 2>&1)"; pad_rc=$?
  echo "$pad_out"
  if [ "$pad_rc" -eq 1 ] && printf '%s\n' "$pad_out" | grep -q "Pad mode was removed"; then
    echo "PASS --pad exits 1 with removal message"
  else
    echo "FAIL --pad exits 1 with removal message (rc=$pad_rc)"; fail=1
  fi
  if [ "$pad_existed_before" -eq 0 ]; then
    if [ -e "$DIR/fb-reels-pad.py" ]; then
      echo "FAIL --pad created fb-reels-pad.py"; fail=1
    else
      echo "PASS --pad did not create fb-reels-pad.py"
    fi
  fi
  echo "--- Reels-Scroll.command --test ---"
  if "$REELS" --test; then
    echo "PASS Reels-Scroll.command --test"
  else
    echo "FAIL Reels-Scroll.command --test"; fail=1
  fi
  if [ "$fail" -eq 0 ]; then
    echo "ANDROID-MOUSE SELFTEST OK"
  else
    echo "ANDROID-MOUSE SELFTEST FAILED"
  fi
  exit $fail
fi

cd "$HOME" || exit 1

[ -x "$ADB" ] || brew install android-platform-tools
[ -x "$PY" ] || brew install python

connect_phone
"$ADB" -s "$SERIAL" shell input keyevent 224 >/dev/null 2>&1  # wake phone screen

# --- Reels-Scroll features: SHIFT+wheel / SHIFT+click listener, in background ---
# Always regenerate the helper from Reels-Scroll.command so listener fixes reach the mirror at once.
LISTENER_PID=""
if [ -f "$REELS" ]; then
  "$REELS" --test >/dev/null 2>&1 || echo "Warning: Reels listener selftest failed - using on-disk helper if present."
fi
ensure_pynput
if [ -f "$PYFILE" ] && "$PY" -c "import pynput" 2>/dev/null; then
  if check_trust; then
    "$PY" "$PYFILE" &
    LISTENER_PID=$!
  else
    echo "macOS blocked the SHIFT+wheel listener (Input Monitoring). Mirror click/drag still works; only SHIFT+wheel is unavailable."
    echo "Allow Terminal in Settings > Privacy > Input Monitoring + Accessibility, quit Terminal (Cmd+Q), reopen."
    open "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent" 2>/dev/null
    sleep 1
    open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" 2>/dev/null
  fi
else
  echo "Reels listener unavailable (helper/pynput missing) - mirror click/drag still works; only SHIFT+wheel is unavailable."
fi
cleanup() { [ -n "$LISTENER_PID" ] && kill "$LISTENER_PID" 2>/dev/null; }
trap cleanup EXIT INT TERM

# scrcpy mirror, phone screen on the Mac (default, incl. explicit --mirror)
[ -x "$SCRCPY" ] || brew install scrcpy
case "$SERIAL" in
  *:*|*_tcp)  # WiFi transport: proven smooth profile, buffers absorb jitter
    PROFILE="WiFi smooth: 414x900 H.264 | buffer video/audio 100ms"
    SCRCPY_PROFILE=(--max-size 900 --video-buffer=100 --audio-buffer=100) ;;
  *)          # USB cable: near-zero delay, quality bumped a lil bit - Exynos HEVC hw encoder held 59-61fps at this size even dual-encoding
    PROFILE="USB near-zero: 664x1440 H.265 12M 60fps | video buffer 0 | raw audio buffer 30ms"
    SCRCPY_PROFILE=(--max-size 1440 --video-codec=h265 --video-bit-rate=12M --max-fps=60 --video-buffer=0 --audio-codec=raw --audio-buffer=30 --render-driver=metal) ;;
esac
echo "=============================================="
echo " MIRROR MODE (scrcpy): PHONE SCREEN on Mac -> $SERIAL"
echo " click=TAP | drag=swipe | wheel=NEXT/PREV | right-click=Back (inside mirror)"
echo " ANYWHERE: light SHIFT+wheel=NEXT/PREV | SHIFT+mid/right=pause"
echo " Profile: $PROFILE"
echo "=============================================="
"$SCRCPY" -s "$SERIAL" --stay-awake "${SCRCPY_PROFILE[@]}" \
  --window-title "Android - click=tap, light SHIFT+wheel=NEXT/PREV, SHIFT+click=tap"
RC=$?
cleanup
trap - EXIT INT TERM
exit $RC
