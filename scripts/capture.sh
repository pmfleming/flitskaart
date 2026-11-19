#!/usr/bin/env bash
# Run the app in an isolated headless Weston session and save a screenshot.
# Nothing appears on the real desktop.
#
#   scripts/capture.sh OUTPUT.png [SECONDS] -- COMMAND [ARGS...]
#
# SECONDS (default 6) is how long to wait before the screenshot.
# Run inside `nix develop`.
set -euo pipefail

output=$(realpath -m "${1:?Usage: capture.sh OUTPUT.png [SECONDS] -- COMMAND}")
shift
wait_seconds=6
if [ "${1:-}" != "--" ]; then wait_seconds=$1; shift; fi
[ "${1:-}" = "--" ] && shift
[ $# -gt 0 ] || { echo "capture.sh: missing command" >&2; exit 2; }

runtime=$(mktemp -d /tmp/dutch-flashcards-capture.XXXXXX)
chmod 700 "$runtime"
cleanup() {
  [ -n "${app_pid:-}" ] && kill "$app_pid" 2>/dev/null || true
  [ -n "${weston_pid:-}" ] && kill "$weston_pid" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$runtime"
}
trap cleanup EXIT

export XDG_RUNTIME_DIR=$runtime WAYLAND_DISPLAY=wayland-capture XDG_SESSION_TYPE=wayland
unset DISPLAY XAUTHORITY WAYLAND_SOCKET HYPRLAND_INSTANCE_SIGNATURE

weston --backend=headless --renderer=gl --shell=desktop-shell.so --no-config \
  --idle-time=0 --fake-seat --debug --socket=wayland-capture --width=1100 --height=760 \
  >"$runtime/weston.log" 2>&1 &
weston_pid=$!
for _ in $(seq 100); do [ -S "$runtime/wayland-capture" ] && break; sleep 0.1; done
[ -S "$runtime/wayland-capture" ] || { cat "$runtime/weston.log" >&2; exit 1; }

"$@" >"$runtime/app.log" 2>&1 &
app_pid=$!
sleep "$wait_seconds"
if ! kill -0 "$app_pid" 2>/dev/null; then
  echo "capture.sh: the app exited early" >&2
  cat "$runtime/app.log" >&2
  exit 1
fi

(cd "$runtime" && weston-screenshooter >/dev/null 2>&1)
shot=$(ls "$runtime"/wayland-screenshot-*.png 2>/dev/null | head -1)
[ -n "$shot" ] || { echo "capture.sh: no screenshot was taken" >&2; tail -20 "$runtime/weston.log" >&2; exit 1; }
mv "$shot" "$output"
echo "Saved $output"
tail -5 "$runtime/app.log"
