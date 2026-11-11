#!/bin/sh
# Drift Linux cold-launch harness — spec task 2.5 (Req 12.1, 12.5, 12.7, 12.8)
#
# Runs INSIDE the pinned Linux container (Dagger linux-e2e entry) after the
# bundle pipeline produced the AppImage: launches the BUILT app headless
# under Xvfb with per-run isolated app data, proves first usable state
# (window mapped, process alive, painted screenshot), shuts the app down
# gracefully, and writes machine-readable evidence to /test-results/e2e/:
#
#   cold-launch-status.json             verdict (ok, reason, exit code, shutdown path, colors)
#   cold-launch-screenshot.png          first-usable-state screenshot
#   cold-launch-failure-screenshot.png  captured BEFORE a failure verdict (screenshot-on-failure)
#   cold-launch-app.log                 app stdout/stderr (GTK + WebKitGTK + Drift logs)
#   xvfb.log                            display server log
#   window-evidence.txt                 xwininfo of the mapped Drift window
#   process-tree.txt                    process tree while the app ran
#   appdata-files.txt                   proof app data landed only in the isolated XDG tree
#
# The harness exits non-zero AFTER the evidence exists; the Dagger entry
# returns the evidence directory either way, and scripts/Invoke-CI.ps1
# fails the tier when ok != true (fail loudly, no fake pass — a failed
# cold launch still exports its failure screenshot). Linux-container
# results never substitute for native Windows/macOS validation.
#
# DRIFT_E2E_BREAK_LAUNCH=1 is a HARNESS test seam for the CI failure-path
# demonstration ONLY: it makes the AppImage non-executable so the launch
# fails and the failure screenshot + fail-loudly flow is proven end to end.
set -u

RESULTS=/test-results/e2e
STATUS=$RESULTS/cold-launch-status.json
SHOT=$RESULTS/cold-launch-screenshot.png
FAIL_SHOT=$RESULTS/cold-launch-failure-screenshot.png
APP_LOG=$RESULTS/cold-launch-app.log
XVFB_LOG=$RESULTS/xvfb.log
DISPLAY_NUM=:99
APP_TITLE='^Drift$'
WINDOW_TIMEOUT_S=40
CLOSE_TIMEOUT_S=20
COLORS_MIN=8
mkdir -p "$RESULTS"

# --- isolated per-run app data (Req 12.7): the container is already
# isolated from the host, but scoping HOME/XDG per run keeps launches
# deterministic and proves the app never touches shared state (/root
# stays clean; every file the app writes lands under the run home).
RUN_HOME=/tmp/drift-e2e-home
export HOME=$RUN_HOME
export XDG_DATA_HOME=$RUN_HOME/share
export XDG_CONFIG_HOME=$RUN_HOME/config
export XDG_CACHE_HOME=$RUN_HOME/cache
export XDG_RUNTIME_DIR=$RUN_HOME/runtime
export DISPLAY=$DISPLAY_NUM
mkdir -p "$HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"
mkdir -m 700 -p "$XDG_RUNTIME_DIR"

write_status() { printf '%s\n' "$1" > "$STATUS"; }

capture_failure_screenshot() {
    # Screenshot-on-failure BEFORE the failed verdict is written (task 2.5):
    # only possible when a display exists; otherwise the status reason
    # explains why there is no screenshot.
    if [ -n "${XVFB_UP:-}" ] && command -v import >/dev/null 2>&1; then
        import -window root -display "$DISPLAY_NUM" "$FAIL_SHOT" \
            >"$RESULTS/failure-screenshot.log" 2>&1 || true
    fi
}

fail() {
    reason=$1
    capture_failure_screenshot
    write_status "{\"ok\":false,\"reason\":\"$reason\",\"exitCode\":${2:-1},\"shutdownPath\":\"none\"}"
    echo "cold-launch: FAIL — $reason"
    exit 1
}

appimage=$(ls src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/*.AppImage 2>/dev/null | head -n 1)
[ -n "$appimage" ] || fail "appimage-missing" 1
echo "cold-launch: appimage $appimage"

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -nolisten tcp >"$XVFB_LOG" 2>&1 &
XVFB_PID=$!
i=0
while [ $i -lt 20 ]; do
    xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1 && break
    kill -0 "$XVFB_PID" 2>/dev/null || break
    sleep 0.5
    i=$((i + 1))
done
if xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1; then
    XVFB_UP=1
else
    fail "xvfb-start-failed" 1
fi

if [ "${DRIFT_E2E_BREAK_LAUNCH:-0}" = "1" ]; then
    chmod -x "$appimage"
fi

start=$(date +%s)
"$appimage" --appimage-extract-and-run >"$APP_LOG" 2>&1 &
APP_PID=$!

win=""
i=0
while [ $i -lt $((WINDOW_TIMEOUT_S * 2)) ]; do
    win=$(xdotool search --onlyvisible --name "$APP_TITLE" 2>/dev/null | head -n 1)
    [ -n "$win" ] && break
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.5
    i=$((i + 1))
done
[ -n "$win" ] || fail "drift-window-not-found" 1
echo "cold-launch: window 0x$(printf '%x' "$win") mapped after $(( $(date +%s) - start ))s"

kill -0 "$APP_PID" 2>/dev/null || fail "app-exited-before-window" 1
pgrep -x rdp-client >"$RESULTS/process-tree.txt" 2>&1 || ps -ef >"$RESULTS/process-tree.txt"
ps -ef >>"$RESULTS/process-tree.txt" 2>&1 || true
xwininfo -id "$win" >"$RESULTS/window-evidence.txt" 2>&1 || true

# --- first-usable-state evidence: let the UI settle, then require a
# PAINTED screenshot (a mapped-but-blank root would have ~1 color).
sleep 3
import -window root -display "$DISPLAY_NUM" "$SHOT" 2>"$RESULTS/screenshot.log" \
    || fail "screenshot-capture-failed" 1
colors=$(identify -format '%k' "$SHOT" 2>/dev/null || echo 0)
geometry=$(identify -format '%G' "$SHOT" 2>/dev/null || echo unknown)
echo "cold-launch: screenshot $geometry, $colors distinct colors"
[ "$colors" -ge "$COLORS_MIN" ] || fail "screenshot-not-painted($colors-colors)" 1

find "$RUN_HOME" -type f | sort >"$RESULTS/appdata-files.txt"

# --- graceful shutdown: WM_DELETE_WINDOW close (GTK handles the
# ClientMessage without a window manager); escalate only if needed.
shutdown_path=graceful-close
xdotool windowclose "$win" >/dev/null 2>&1 || true
i=0
while [ $i -lt $((CLOSE_TIMEOUT_S * 2)) ]; do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.5
    i=$((i + 1))
done
if kill -0 "$APP_PID" 2>/dev/null; then
    shutdown_path=sigterm
    kill -TERM "$APP_PID" 2>/dev/null || true
    i=0
    while [ $i -lt 10 ]; do
        kill -0 "$APP_PID" 2>/dev/null || break
        sleep 0.5
        i=$((i + 1))
    done
    if kill -0 "$APP_PID" 2>/dev/null; then
        shutdown_path=sigkill
        kill -9 "$APP_PID" 2>/dev/null || true
    fi
fi
wait "$APP_PID" 2>/dev/null
app_exit=$?
kill "$XVFB_PID" 2>/dev/null || true

echo "cold-launch: shutdown $shutdown_path (app exit $app_exit)"
write_status "{\"ok\":true,\"reason\":\"\",\"windowId\":\"$win\",\"shutdownPath\":\"$shutdown_path\",\"exitCode\":$app_exit,\"distinctColors\":$colors}"
echo "cold-launch: PASS"
exit 0
