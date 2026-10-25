#!/bin/bash
# Reels-Scroll.command - ONE FILE on Desktop. SHIFT + mouse wheel = Reels on TV.
#   SHIFT + wheel DOWN = NEXT Reel | SHIFT + wheel UP = PREV Reel (works anywhere on Mac)
#   SHIFT + middle-click (or SHIFT+right-click) = pause/resume VIDEO via tap center (keyevent 85 ignored by FB)
# Needs ONE-TIME allow: macOS Input Monitoring for Terminal (this file opens it for you).
ADB="/opt/homebrew/bin/adb"
PY="/opt/homebrew/bin/python3"
IPFILE="$HOME/Desktop/fb-reels-tv-ip.txt"
PYFILE="$HOME/Desktop/fb-reels-scroll.py"
SERIAL=""

is_connected() { "$ADB" devices 2>/dev/null | grep -q $'\tdevice$'; }

resolve_ip() {  # $1 = hostname -> IPv4 (adb can't use .local names)
  local h="${1%.}" f=/tmp/adb-g.txt
  rm -f "$f"
  dns-sd -G v4 "$h" >"$f" 2>&1 &
  local pid=$!
  sleep 4
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
  grep -ao '[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*' "$f" 2>/dev/null | head -n 1
  rm -f "$f"
}

pick_serial() {  # pin one transport so commands never hit "more than one device"
  local d
  d=$("$ADB" devices 2>/dev/null | awk -F'\t' '$2=="device"{print $1}')
  SERIAL=$(printf '%s\n' "$d" | grep -E '^[0-9.]+:[0-9]+$' | head -n 1)
  [ -z "$SERIAL" ] && SERIAL=$(printf '%s\n' "$d" | head -n 1)
}

auto_connect() {
  is_connected && return 0
  "$ADB" disconnect >/dev/null 2>&1
  "$ADB" kill-server >/dev/null 2>&1
  sleep 1
  "$ADB" start-server >/dev/null 2>&1
  for _ in 1 2 3; do
    is_connected && return 0
    sleep 2
  done
  if [ -n "$SAVED_IP" ]; then
    "$ADB" connect "$SAVED_IP" >/dev/null 2>&1; sleep 2
    is_connected && return 0
  fi
  rm -f /tmp/adb-browse.txt
  dns-sd -B _adb-tls-connect._tcp local >/tmp/adb-browse.txt 2>&1 &
  BPID=$!
  sleep 6
  kill $BPID 2>/dev/null; wait $BPID 2>/dev/null
  INSTANCES=$(awk '/Add/ {print $NF}' /tmp/adb-browse.txt 2>/dev/null | sort -u)
  rm -f /tmp/adb-browse.txt
  for inst in $INSTANCES; do
    rm -f /tmp/adb-lookup.txt
    dns-sd -L "$inst" _adb-tls-connect._tcp local >/tmp/adb-lookup.txt 2>&1 &
    LPID=$!
    sleep 4
    kill $LPID 2>/dev/null; wait $LPID 2>/dev/null
    HP=$(sed -n 's/.*can be reached at \([^ :]*\)\.*:\([0-9]*\).*/\1:\2/p' /tmp/adb-lookup.txt | head -n 1 | tr -d ' ')
    rm -f /tmp/adb-lookup.txt
    [ -z "$HP" ] && continue
    HP=${HP/.local.:/.local:}
    HOST=${HP%:*}; PORT=${HP##*:}
    IP=$(resolve_ip "$HOST")
    [ -n "$IP" ] && HP="$IP:$PORT"
    echo "Found $inst -> $HP, connecting..."
    "$ADB" connect "$HP" >/dev/null 2>&1; sleep 2
    if is_connected; then echo "$HP" > "$IPFILE"; SAVED_IP="$HP"; return 0; fi
  done
  return 1
}

write_python() {
  cat > "$PYFILE" <<'PYEOF'
#!/opt/homebrew/bin/python3
"""S25 FE Shift+wheel listener. DOWN=NEXT, UP=PREV. No phone touch."""
import time, subprocess, sys, os
ADB = "/opt/homebrew/bin/adb"
SERIAL = os.environ.get("ADB_SERIAL", "")
BASE = [ADB] + (["-s", SERIAL] if SERIAL else []) + ["shell", "input", "swipe"]
NEXT_CMD = BASE + ["540", "1700", "540", "700", "400"]  # finger UP = next
PREV_CMD = BASE + ["540", "700", "540", "1700", "400"]  # finger DOWN = prev
VIDEO_CMD = [ADB] + (["-s", SERIAL] if SERIAL else []) + ["shell", "input", "tap", "540", "1150"]  # tap center = Reels video pause/resume (keyevent 85 is ignored by FB player)
COOLDOWN = 0.9
INVERT = False
last_fire = 0.0
shift_held = False

def scrcpy_at(x, y):
    """True when the Mac cursor (x, y) is over the scrcpy mirror window."""
    try:
        from Quartz import CGWindowListCopyWindowInfo, kCGWindowListOptionOnScreenOnly, kCGWindowListExcludeDesktopElements, kCGNullWindowID
        wins = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID)
        for w in wins:
            if "scrcpy" not in (w.get("kCGWindowOwnerName") or "").lower():
                continue
            b = w.get("kCGWindowBounds") or {}
            bx, by = float(b.get("X", 0)), float(b.get("Y", 0))
            if bx <= x <= bx + float(b.get("Width", 0)) and by <= y <= by + float(b.get("Height", 0)):
                return True
        return False
    except Exception:
        return False

def decide(dx, dy):
    if abs(dx) < 1e-9 and abs(dy) < 1e-9:
        return None
    if abs(dx) > abs(dy):
        is_next = (dx < 0)
    else:
        is_next = (dy < 0)
    if INVERT:
        is_next = not is_next
    return "NEXT" if is_next else "PREV"

def selftest():
    cases = [(0,-1,"NEXT"),(0,1,"PREV"),(-1,0,"NEXT"),(1,0,"PREV"),(0,0,None)]
    ok = True
    for dx, dy, exp in cases:
        got = decide(dx, dy)
        s = "PASS" if got == exp else "FAIL"
        if got != exp: ok = False
        print(f"  dx={dx:+} dy={dy:+} -> {got} (expect {exp}) [{s}]", flush=True)
    assert NEXT_CMD[4:8] == ["540","1700","540","700"], "NEXT must be bottom->top"
    assert PREV_CMD[4:8] == ["540","700","540","1700"], "PREV must be top->bottom"
    print("  swipe NEXT=1700->700 [PASS]", flush=True)
    print("  swipe PREV=700->1700 [PASS]", flush=True)
    vp = (VIDEO_CMD[-3:] == ["tap", "540", "1150"])
    print(f"  video pause/resume tap center [{('PASS' if vp else 'FAIL')}: {' '.join(VIDEO_CMD[-3:])}]", flush=True)
    if not vp: ok = False
    sf = scrcpy_at(0, 0)
    print(f"  scrcpy-window detect returns bool [{('PASS' if isinstance(sf, bool) else 'FAIL')}: {sf} at 0,0]", flush=True)
    if not isinstance(sf, bool): ok = False
    print("SELFTEST " + ("OK - DOWN=NEXT, UP=PREV" if ok else "FAILED"), flush=True)
    return ok

def run_cmd(cmd):
    try:
        r = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        if r.returncode != 0:
            print(f"adb rc={r.returncode}: {(r.stderr or b'').decode(errors='replace').strip()}", flush=True)
        return r.returncode
    except Exception as e:
        print(f"adb error: {e}", flush=True)
        return -1

def on_scroll(x, y, dx, dy):
    global last_fire
    if not shift_held: return
    if scrcpy_at(x, y): return  # cursor over mirror: scrcpy forwards wheel itself, else double-NEXT
    now = time.time()
    if now - last_fire < COOLDOWN: return
    action = decide(dx, dy)
    if action is None: return
    last_fire = now
    print(f"[{time.strftime('%H:%M:%S')}] Shift+wheel -> {action}", flush=True)
    run_cmd(NEXT_CMD if action == "NEXT" else PREV_CMD)

def on_press(key):
    global shift_held
    try:
        from pynput.keyboard import Key
        if key in (Key.shift, Key.shift_l, Key.shift_r): shift_held = True
    except Exception: pass

def on_release(key):
    global shift_held
    try:
        from pynput.keyboard import Key
        if key in (Key.shift, Key.shift_l, Key.shift_r): shift_held = False
    except Exception: pass

def on_click(x, y, button, pressed):
    try:
        from pynput.mouse import Button
        name = str(button)
    except Exception:
        return
    if not pressed:
        return
    if scrcpy_at(x, y): return  # cursor over mirror: scrcpy handles click (SHIFT+click = tap there), else double-tap
    try:
        from pynput.mouse import Button
        if button in (Button.middle, Button.right):
            # always log so you can see if the hardware click is detected at all
            print(f"[{time.strftime('%H:%M:%S')}] {name} pressed shift={shift_held} -> " + ("VIDEO pause/resume" if shift_held else "ignored (hold SHIFT)"), flush=True)
            if shift_held:
                rc = run_cmd(VIDEO_CMD)
                if rc == 0:
                    print(f"[{time.strftime('%H:%M:%S')}] sent tap 540 1150 to {SERIAL or 'phone'}", flush=True)
    except Exception as e:
        print(f"click error: {e}", flush=True)

if __name__ == "__main__":
    if "--test" in sys.argv:
        print("== wheel selftest ==", flush=True)
        sys.exit(0 if selftest() else 1)
    if "--click-probe" in sys.argv:
        from pynput import mouse, keyboard
        print("CLICK-PROBE: press SHIFT+middle-click and SHIFT+right-click. Logging every middle/right press for 20s...", flush=True)
        import threading
        def stop():
            time.sleep(20)
            print("PROBE DONE - if no lines appeared, macOS blocked the listener or the mouse has no middle button (use SHIFT+right-click instead).", flush=True)
            os._exit(0)
        threading.Thread(target=stop, daemon=True).start()
        kl = keyboard.Listener(on_press=on_press, on_release=on_release)
        ml = mouse.Listener(on_scroll=on_scroll, on_click=on_click)
        kl.start(); ml.start()
        kl.join(); ml.join()
        sys.exit(0)
    from pynput import mouse, keyboard
    print("Hold SHIFT + wheel DOWN = NEXT | UP = PREV | SHIFT+middle-click (or SHIFT+right-click) = VIDEO pause/resume. Ctrl+C stops.", flush=True)
    kl = keyboard.Listener(on_press=on_press, on_release=on_release)
    ml = mouse.Listener(on_scroll=on_scroll, on_click=on_click)
    kl.start(); ml.start()
    try: kl.join(); ml.join()
    except KeyboardInterrupt: print("\nStopped.", flush=True)
PYEOF
}

check_trust() {  # 0 = allowed, 1 = blocked by macOS
  OUT=$("$PY" -c '
from pynput import mouse, keyboard
import time
m = mouse.Listener(on_scroll=lambda x,y,dx,dy: None)
k = keyboard.Listener(on_press=lambda key: None)
m.start(); k.start()
time.sleep(2)
m.stop(); k.stop()
print("PROBE_DONE")' 2>&1)
  echo "$OUT" | grep -qi "not trusted" && return 1
  echo "$OUT" | grep -q PROBE_DONE
}

if [ "$1" = "--click-probe" ]; then
  write_python
  [ -f "$IPFILE" ] && SAVED_IP=$(cat "$IPFILE") || SAVED_IP=""
  pick_serial; export ADB_SERIAL="$SERIAL"
  echo "Phone ($SERIAL). Probe logging clicks for 20s - try your middle-click now."
  "$PY" "$PYFILE" --click-probe
  exit $?
fi

if [ "$1" = "--test" ]; then
  echo "=== REVIEW (one file, helper auto-written to $PYFILE) ==="
  write_python
  "$PY" "$PYFILE" --test
  rc=$?
  export PATH="/opt/homebrew/bin:$PATH"
  "$ADB" devices -l 2>/dev/null | head -n 4
  exit $rc
fi

export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$HOME" || exit 1
[ -x "$ADB" ] || brew install android-platform-tools
[ -x "$PY" ] || brew install python
"$PY" -c "import pynput" 2>/dev/null || "$PY" -m pip install -q --break-system-packages pynput
write_python

echo "=============================================="
echo " Reels SHIFT + WHEEL -> Samsung TV"
echo " SHIFT + wheel DOWN = NEXT | UP = PREV | SHIFT + middle/right-click = VIDEO pause/resume"
echo "=============================================="
echo "1) S25 FE + TV same WiFi, Smart View ON, Reels fullscreen, screen ON"
echo ""

[ -f "$IPFILE" ] && SAVED_IP=$(cat "$IPFILE") || SAVED_IP=""
auto_connect
while ! is_connected; do
  echo ""
  echo "--- Phone not found automatically ---"
  echo "On S25 FE: SAME WiFi as Mac, Wireless debugging ON."
  read -p "Turn it ON, then press ENTER to search again: " _
  auto_connect
done
pick_serial
export ADB_SERIAL="$SERIAL"
echo ""; echo "Phone OK ($SERIAL)."
echo ""

echo "Checking macOS permission (one-time)..."
if ! check_trust; then
  echo ""
  echo "*** macOS blocked the wheel listener. Fix (1 min, once): ***"
  echo "1) I am opening Settings > Privacy > Input Monitoring for you."
  echo "2) Click +, add Terminal, turn it ON. Same in Accessibility."
  echo "3) FULLY quit Terminal (Cmd+Q), reopen, double-click this file again."
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent" 2>/dev/null
  sleep 1
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" 2>/dev/null
  read -p "Press Enter to close (keyboard file works without this: Reels-Shift.command): " _
  exit 1
fi
echo "Permission OK. Starting listener..."
echo "Hold SHIFT + turn wheel. Ctrl+C to stop."
"$PY" "$PYFILE"
