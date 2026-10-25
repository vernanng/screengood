# fb-reels-lib.sh - shared bash for Android-Mouse.command and Reels-Scroll.command.
# Sourced, not run directly. Caller sets DIR first, then sources this file and
# calls resolve_tools. Functions read ADB, PY, IPFILE at call time.
# Keep this file next to both launchers.

resolve_tools() {  # sets BREW_BIN, ADB, SCRCPY, PY; exports ADB_BIN and PATH
  if command -v brew >/dev/null 2>&1; then BREW_BIN="$(brew --prefix)/bin"
  elif [ -x /opt/homebrew/bin/adb ]; then BREW_BIN="/opt/homebrew/bin"
  else BREW_BIN="/usr/local/bin"; fi
  ADB="$BREW_BIN/adb"; [ -x "$ADB" ] || ADB="$(command -v adb || echo "$BREW_BIN/adb")"
  SCRCPY="$BREW_BIN/scrcpy"; [ -x "$SCRCPY" ] || SCRCPY="$(command -v scrcpy || echo "$BREW_BIN/scrcpy")"
  PY="$BREW_BIN/python3"; [ -x "$PY" ] || PY="$(command -v python3 || echo "$BREW_BIN/python3")"
  export ADB_BIN="$ADB"  # helper .py files use the same adb
  export PATH="$BREW_BIN:/opt/homebrew/bin:/usr/local/bin:$PATH"
}

ensure_pynput() {  # system python if pynput already there, else project .venv (never --break-system-packages)
  "$PY" -c "import pynput" 2>/dev/null && return 0
  [ -x "$DIR/.venv/bin/python" ] || "$PY" -m venv --system-site-packages "$DIR/.venv" || return 1
  "$DIR/.venv/bin/python" -m pip install -q pynput 2>/dev/null
  if "$DIR/.venv/bin/python" -c "import pynput" 2>/dev/null; then PY="$DIR/.venv/bin/python"; else return 1; fi
}

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
  SERIAL=$(printf '%s\n' "$d" | grep -vE '^[0-9.]+:[0-9]+$' | grep -v '_tcp$' | head -n 1)  # USB cable first: lowest delay
  [ -z "$SERIAL" ] && SERIAL=$(printf '%s\n' "$d" | grep -E '^[0-9.]+:[0-9]+$' | head -n 1)
  [ -z "$SERIAL" ] && SERIAL=$(printf '%s\n' "$d" | head -n 1)
}

auto_connect() {  # USB or already-connected wins instantly; else saved IP, then mDNS search
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

check_trust() {  # 0 = Input Monitoring allowed, 1 = blocked by macOS
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

detect_phone_size() {  # exports PHONE_W/PHONE_H from real `wm size` when parseable
  SIZE=$("$ADB" -s "$SERIAL" shell wm size 2>/dev/null | grep -o '[0-9]*x[0-9]*' | head -n 1)
  case "$SIZE" in *x*) export PHONE_W="${SIZE%x*}" PHONE_H="${SIZE#*x}" ;; esac
}

connect_phone() {  # find the phone (retry loop), then pin+export SERIAL, ADB_SERIAL, PHONE_W/H
  [ -f "$IPFILE" ] && SAVED_IP=$(cat "$IPFILE") || SAVED_IP=""
  auto_connect
  while ! is_connected; do
    echo ""
    echo "--- Phone not found automatically ---"
    echo "On S25 FE: USB cable plugged in, or SAME WiFi as Mac with Wireless debugging ON."
    read -p "Fix that, then press ENTER to search again: " _
    auto_connect
  done
  pick_serial
  export ADB_SERIAL="$SERIAL"
  detect_phone_size
}
