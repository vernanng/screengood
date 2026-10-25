#!/bin/bash
# Android-Mouse.command - Desktop. Mac mouse controls Android screen via scrcpy (over WiFi adb).
#   Left-click = tap | drag = swipe | wheel = scroll | right-click = Back | middle-click = Home
#   Watch on TV (Smart View) or in the Mac window - clicks land exactly where you click in the window.
ADB="/opt/homebrew/bin/adb"
SCRCPY="/opt/homebrew/bin/scrcpy"
IPFILE="$HOME/Desktop/fb-reels-tv-ip.txt"
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$HOME" || exit 1

[ -x "$SCRCPY" ] || brew install scrcpy
[ -x "$ADB" ] || brew install android-platform-tools

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

echo "=============================================="
echo " Android MOUSE CONTROL -> $SERIAL"
echo " Left-click = TAP | drag = swipe | wheel = scroll"
echo " Right-click = Back | middle-click = Home"
echo " Close the mirror window (or Ctrl+C here) to stop."
echo "=============================================="
# --no-audio: keep Reels audio on the TV, not duplicated on the Mac
exec "$SCRCPY" -s "$SERIAL" --no-audio --stay-awake --max-size 900 \
  --window-title "Android - click = tap, drag = swipe, wheel = scroll"
