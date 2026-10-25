#!/bin/bash
# Android-Mouse.command - Desktop. Mac mouse controls Android + ALL Reels-Scroll features, one launcher.
#   INSIDE mirror window : click=tap (center=pause) | drag=swipe | wheel=NEXT/PREV | right=Back | middle=Home
#   ANYWHERE (TV, no window focus): SHIFT+wheel=NEXT/PREV | SHIFT+middle/right-click=VIDEO pause/resume
#   Listener auto-steps-aside while cursor is over mirror window, so actions never fire twice.
ADB="/opt/homebrew/bin/adb"
SCRCPY="/opt/homebrew/bin/scrcpy"
PY="/opt/homebrew/bin/python3"
IPFILE="$HOME/Desktop/fb-reels-tv-ip.txt"
PYFILE="$HOME/Desktop/fb-reels-scroll.py"
REELS="$HOME/Desktop/Reels-Scroll.command"
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$HOME" || exit 1

[ -x "$SCRCPY" ] || brew install scrcpy
[ -x "$ADB" ] || brew install android-platform-tools
[ -x "$PY" ] || brew install python

pick_serial() {
  local d
  d=$("$ADB" devices 2>/dev/null | awk -F'\t' '$2=="device"{print $1}')
  SERIAL=$(printf '%s\n' "$d" | grep -E '^[0-9.]+:[0-9]+$' | head -n 1)
  [ -z "$SERIAL" ] && SERIAL=$(printf '%s\n' "$d" | head -n 1)
}

pick_serial
if [ -z "$SERIAL" ] && [ -f "$IPFILE" ]; then
  "$ADB" connect "$(cat "$IPFILE")" >/dev/null 2>&1; sleep 2; pick_serial
fi
while [ -z "$SERIAL" ]; do
  echo "Phone not found. Same WiFi as Mac + Wireless debugging ON on S25 FE."
  read -p "Press ENTER to search again: " _
  [ -f "$IPFILE" ] && "$ADB" connect "$(cat "$IPFILE")" >/dev/null 2>&1
  sleep 2; pick_serial
done
export ADB_SERIAL="$SERIAL"

echo "=============================================="
echo " Android MOUSE + REELS -> $SERIAL"
echo " INSIDE window : click=TAP | drag=swipe | wheel=NEXT/PREV"
echo " ANYWHERE      : SHIFT+wheel=NEXT/PREV | SHIFT+mid/right-click=pause"
echo " Close the mirror window (or Ctrl+C here) to stop both."
echo "=============================================="

# --- Reels-Scroll features: same SHIFT+wheel / SHIFT+click listener, in background ---
LISTENER_PID=""
if [ ! -f "$PYFILE" ] && [ -f "$REELS" ]; then "$REELS" --test >/dev/null 2>&1; fi
if [ -f "$PYFILE" ] && "$PY" -c "import pynput" 2>/dev/null; then
  "$PY" "$PYFILE" &
  LISTENER_PID=$!
  echo "Reels listener ON (pid $LISTENER_PID) - auto-off while cursor is over the mirror window."
else
  echo "Reels listener unavailable (helper/pynput missing) - mirror control still works."
fi
cleanup() { [ -n "$LISTENER_PID" ] && kill "$LISTENER_PID" 2>/dev/null; }
trap cleanup EXIT INT TERM

# --no-audio: keep Reels audio on the TV, not duplicated on the Mac
"$SCRCPY" -s "$SERIAL" --no-audio --stay-awake --max-size 900 \
  --window-title "Android - click=tap, wheel=NEXT/PREV, SHIFT+click=tap"
RC=$?
cleanup
trap - EXIT INT TERM
exit $RC
