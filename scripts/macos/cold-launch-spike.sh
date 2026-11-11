#!/usr/bin/env bash
# Task 2.4 — macOS built-app cold launch + WebdriverIO Tauri-service
# embedded-driver spike (CI-runner conversion, user decision 2026-11-11;
# no macOS dev host exists in this environment, so the spike runs as an
# Actions job on the fork's macOS runner).
#
# Runs ON the macOS runner (macos-15, aarch64). Two stages, evidence ALWAYS
# exported into $DRIFT_EVIDENCE_DIR:
#   stage 1 — OS-level built-app cold launch WITHOUT any WebDriver
#             (Req 12.8 cold-launch demonstration; twins the proven 2.3
#             Windows / 2.5 Linux pattern: launch the built binary ->
#             real window mapped -> screenshot -> graceful quit) using
#             CGWindowList via scripts/macos/window-probe.swift and
#             screencapture.
#   stage 2 — WebdriverIO via @wdio/tauri-service with the EMBEDDED
#             WebDriver server (design.md Testing Strategy mandate; the
#             service's own docs: 'embedded' is the only macOS provider
#             that needs no paid API key — 'external' tauri-driver is
#             Windows/Linux-only and the keyed third-party driver needs a
#             secret this repository's CI forbids).
#
# This script NEVER exits non-zero: it collects evidence and writes
# cold-launch-driver-status.json; the WORKFLOW verdict step gates the job
# on that status (missing / not-ok / ok-without-screenshot -> job fails).
# Fail-loudly contract mirrors scripts/linux/cold-launch.sh (task 2.5).
#
# Isolation (Req 12.7): HOME and TMPDIR are scoped into the evidence tree.
# On macOS the Tauri/dirs stack resolves app data under
# $HOME/Library/Application Support/com.drift.app and WKWebView keeps its
# state under $HOME/Library/WebKit — a per-run HOME isolates both (the
# Windows Known-Folders caveat from the 2.3 spike does not apply here).
# Cleanup touches only this run's own resources: the built rdp-client
# process and the evidence tree, on an ephemeral runner.

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EVID="${DRIFT_EVIDENCE_DIR:-$REPO_ROOT/ci-artifacts/macos-e2e-spike}"
APP_BIN="${DRIFT_APP_BINARY:-$REPO_ROOT/src-tauri/target/aarch64-apple-darwin/release/rdp-client}"
RUN_HOME="$EVID/run-home"
SPIKE_DIR="$REPO_ROOT/scripts/macos/e2e-spike"
WDIO_CAP_SECS="${DRIFT_WDIO_WATCHDOG_SECS:-420}"

WDIO_RC=-1
OS_OK=false
OS_REASON="not-run"
APP_QUIT=false

mkdir -p "$EVID" \
  "$RUN_HOME/Library/Application Support" \
  "$RUN_HOME/Library/Caches" \
  "$RUN_HOME/Library/Preferences" \
  "$RUN_HOME/Library/WebKit" \
  "$EVID/tmp"
export HOME="$RUN_HOME"   # app data + WKWebView state isolation (Req 12.7)
export TMPDIR="$EVID/tmp"

shot() { screencapture -x "$1" >/dev/null 2>&1 || true; }

# Alive = process exists and is not a zombie (kill -0 alone reports
# zombies as alive, which would fake a "launched" verdict).
alive() {
  local stat
  stat="$(ps -o stat= -p "$1" 2>/dev/null | tr -d ' ')"
  [ -n "$stat" ] && [ "$stat" != "Z" ]
}

write_status() { # $1 ok (true/false), $2 reason
  printf '{"ok": %s, "reason": "%s", "wdioExitCode": %s, "osColdLaunch": {"ok": %s, "reason": "%s"}, "appQuitClean": %s}\n' \
    "$1" "$2" "$WDIO_RC" "$OS_OK" "$OS_REASON" "$APP_QUIT" \
    > "$EVID/cold-launch-driver-status.json"
  ls -la "$EVID" > "$EVID/evidence-inventory.txt" 2>&1 || true
}

# --- stage 0: the built binary must exist --------------------------------
if [ ! -x "$APP_BIN" ]; then
  OS_REASON="binary-missing"
  shot "$EVID/driver-failure-screen.png"
  write_status false "built binary missing at $APP_BIN"
  exit 0
fi
shasum -a 256 "$APP_BIN" > "$EVID/app-binary-sha256.txt" 2>&1 || true
sw_vers > "$EVID/runner-os.txt" 2>&1 || true
uname -a >> "$EVID/runner-os.txt" 2>&1 || true
echo "evidence dir: $EVID" > "$EVID/paths.txt"
echo "app binary:   $APP_BIN" >> "$EVID/paths.txt"

# --- stage 1: OS-level cold launch (no WebDriver) ------------------------
"$APP_BIN" > "$EVID/os-launch-app.log" 2>&1 &
APP_PID=$!
WINDOW_FOUND=false
if command -v swift >/dev/null 2>&1; then
  i=0
  while [ "$i" -lt 60 ]; do
    if swift "$REPO_ROOT/scripts/macos/window-probe.swift" "$APP_PID" >> "$EVID/os-window-probe.txt" 2>&1; then
      WINDOW_FOUND=true
      break
    fi
    if ! alive "$APP_PID"; then break; fi
    sleep 1
    i=$((i + 1))
  done
else
  OS_REASON="swift-unavailable"
fi

if [ "$WINDOW_FOUND" = true ]; then
  sleep 3   # settle, then capture the OS-level first-usable-state evidence
  shot "$EVID/os-cold-launch.png"
  if alive "$APP_PID"; then
    kill -TERM "$APP_PID" 2>/dev/null
    n=0
    while alive "$APP_PID" && [ "$n" -lt 15 ]; do sleep 1; n=$((n + 1)); done
    if alive "$APP_PID"; then
      kill -9 "$APP_PID" 2>/dev/null
      wait "$APP_PID" 2>/dev/null
      OS_REASON="window-mapped-but-sigterm-hang"
    else
      wait "$APP_PID" 2>/dev/null
      APP_RC=$?
      echo "os probe app exit code: $APP_RC" >> "$EVID/os-launch-app.log"
      OS_OK=true
      OS_REASON="window-mapped-graceful-quit"
      APP_QUIT=true
    fi
  else
    OS_REASON="app-died-after-window"
  fi
else
  OS_REASON="${OS_REASON:-window-not-found-or-app-dead}"
  shot "$EVID/os-cold-launch-failure.png"
  wait "$APP_PID" 2>/dev/null
fi

# --- stage 2: WebdriverIO embedded-driver session (design mandate) -------
cd "$SPIKE_DIR"

# Watchdog (macOS runners ship no GNU timeout): bounded wdio execution so a
# hung session cannot burn the job; 124 marks a watchdog kill.
run_with_watchdog() { # $1 cap-seconds, rest: command
  local cap="$1"
  shift
  "$@" > "$EVID/wdio-output.log" 2>&1 &
  local cmd_pid=$!
  local waited=0
  while alive "$cmd_pid"; do
    sleep 5
    waited=$((waited + 5))
    if [ "$waited" -ge "$cap" ]; then
      echo "watchdog: killed wdio after ${cap}s" >> "$EVID/wdio-output.log"
      kill -TERM "$cmd_pid" 2>/dev/null
      sleep 5
      kill -9 "$cmd_pid" 2>/dev/null
      return 124
    fi
  done
  wait "$cmd_pid"
}

run_with_watchdog "$WDIO_CAP_SECS" npx wdio run wdio.conf.js
WDIO_RC=$?
echo "wdio exit code: $WDIO_RC" >> "$EVID/wdio-output.log"

# Clean-quit check: the service must tear the spawned app down with the
# session; a lingering rdp-client process is a failed clean quit.
sleep 2
CLEAN_QUIT="$APP_QUIT"
if pgrep -x rdp-client >/dev/null 2>&1; then
  CLEAN_QUIT=false
fi

# --- verdict --------------------------------------------------------------
FIRST_SHOT="$EVID/first-usable-state.png"
SHOT_OK=false
if [ -f "$FIRST_SHOT" ] && [ "$(wc -c < "$FIRST_SHOT" | tr -d ' ')" -gt 4096 ]; then
  SHOT_OK=true
fi

if [ "$OS_OK" != true ]; then
  reason="os-cold-launch-failed:$OS_REASON"
elif [ "$WDIO_RC" -eq 124 ]; then
  reason="wdio-watchdog-timeout"
elif [ "$WDIO_RC" -ne 0 ]; then
  reason="wdio-exit-$WDIO_RC"
elif [ "$SHOT_OK" != true ]; then
  reason="first-usable-state-screenshot-missing"
elif [ "$CLEAN_QUIT" != true ]; then
  reason="app-process-lingered-after-session"
else
  reason="driver-session-passed"
fi

if [ "$reason" = "driver-session-passed" ]; then
  write_status true "$reason"
else
  shot "$EVID/driver-failure-screen.png"
  APP_QUIT="$CLEAN_QUIT"
  write_status false "$reason"
fi

# Own-resource cleanup only: the built app process (if anything survived)
# and nothing else; the evidence tree stays for the artifact upload.
pkill -TERM -x rdp-client 2>/dev/null || true
sleep 3
pkill -9 -x rdp-client 2>/dev/null || true

exit 0
