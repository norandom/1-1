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
#             CGWindowList via the ONCE-COMPILED window-probe.bin and
#             screencapture.
#   stage 2 — WebdriverIO via @wdio/tauri-service with the EMBEDDED
#             WebDriver server (design.md Testing Strategy mandate; the
#             service's own docs: 'embedded' is the only macOS provider
#             that needs no paid API key — 'external' tauri-driver is
#             Windows/Linux-only and the keyed third-party driver needs a
#             secret this repository's CI forbids).
#
# This script NEVER exits non-zero and NEVER relies on the GitHub job
# timeout as its terminator: every stage carries a hard wall-time cap
# (STAGE1_CAP_SECS / STAGE2_CAP_SECS) that writes an explicit failure
# status and exits cleanly; teardown always goes through kill_bounded
# (TERM -> bounded wait -> KILL -> reap). Run-1 lesson (live run
# 43077191197, cancelled at the 60-min job timeout): a bare `wait` on a
# healthy GUI app hung the job after the window probe failed 60/60
# iterations because the /tmp-invoked script mis-resolved the repo root —
# REPO_ROOT now comes from DRIFT_REPO_ROOT (workflow env), and the probe
# is compiled once, not per poll. The WORKFLOW verdict step gates the job
# on cold-launch-driver-status.json (missing / not-ok / ok-without-
# screenshot -> job fails). Fail-loudly contract mirrors
# scripts/linux/cold-launch.sh (task 2.5).
#
# Isolation (Req 12.7): HOME and TMPDIR are scoped into the evidence tree.
# On macOS the Tauri/dirs stack resolves app data under
# $HOME/Library/Application Support/com.drift.app and WKWebView keeps its
# state under $HOME/Library/WebKit — a per-run HOME isolates both (the
# Windows Known-Folders caveat from the 2.3 spike does not apply here).
# Run-1 evidence confirmed this: the app logged its data dir inside
# run-home/ and Drift.log landed there in the exported artifact. Cleanup
# touches only this run's own resources: the built rdp-client process and
# the evidence tree, on an ephemeral runner.

set -u

now() { date +%s; }
stamp() { date -u +%FT%TZ; }

EVID="${DRIFT_EVIDENCE_DIR:-$PWD/ci-artifacts/macos-e2e-spike}"
mkdir -p "$EVID" 2>/dev/null || true
TIMELINE="$EVID/harness-timeline.txt"

log() { echo "[$(stamp)] $*" >> "$TIMELINE" 2>/dev/null || true; }

# Resolve the repo root: DRIFT_REPO_ROOT first (the workflow passes
# github.workspace; the script usually runs as a CR-stripped /tmp copy,
# where dirname-based resolution collapses to / — run-1 root cause), then
# the script-relative fallback for direct repo invocations.
REPO_ROOT="${DRIFT_REPO_ROOT:-}"
if [ -z "$REPO_ROOT" ] || [ ! -d "$REPO_ROOT/scripts/macos" ]; then
  REPO_ROOT="$(cd "$(dirname "$0")/../.." 2>/dev/null && pwd)"
fi
if [ -z "$REPO_ROOT" ] || [ ! -d "$REPO_ROOT/scripts/macos" ]; then
  printf '{"ok": false, "reason": "harness-repo-root-unresolved", "wdioExitCode": -1, "osColdLaunch": {"ok": false, "reason": "not-run"}, "appQuitClean": false}\n' \
    > "$EVID/cold-launch-driver-status.json"
  log "FATAL repo root unresolved (DRIFT_REPO_ROOT unset and script-relative fallback failed)"
  exit 0
fi

APP_BIN="${DRIFT_APP_BINARY:-$REPO_ROOT/src-tauri/target/aarch64-apple-darwin/release/rdp-client}"
RUN_HOME="$EVID/run-home"
SPIKE_DIR="$REPO_ROOT/scripts/macos/e2e-spike"
WDIO_CAP_SECS="${DRIFT_WDIO_WATCHDOG_SECS:-420}"
STAGE1_CAP_SECS="${DRIFT_STAGE1_CAP_SECS:-300}"
STAGE2_CAP_SECS="${DRIFT_STAGE2_CAP_SECS:-600}"

WDIO_RC=-1
OS_OK=false
OS_REASON="not-run"
APP_QUIT=false
REAP_RC=-1

mkdir -p "$RUN_HOME/Library/Application Support" \
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

# Bounded reap: TERM -> wait at most N secs -> KILL -> reap. Sets REAP_RC
# to the reaped exit code (137 after a KILL) and KILLED=true only when the
# KILL was needed. NEVER wait on a live GUI process without this bound
# (run-1 hang).
kill_bounded() { # $1 pid, $2 term-wait-secs (default 15)
  local pid="$1" secs="${2:-15}" n=0
  REAP_RC=-1
  KILLED=false
  kill -TERM "$pid" 2>/dev/null || true
  while alive "$pid" && [ "$n" -lt "$secs" ]; do sleep 1; n=$((n + 1)); done
  if alive "$pid"; then
    kill -9 "$pid" 2>/dev/null || true
    KILLED=true
    sleep 1
  fi
  wait "$pid" 2>/dev/null
  REAP_RC=$?
}

write_status() { # $1 ok (true/false), $2 reason
  printf '{"ok": %s, "reason": "%s", "wdioExitCode": %s, "osColdLaunch": {"ok": %s, "reason": "%s"}, "appQuitClean": %s}\n' \
    "$1" "$2" "$WDIO_RC" "$OS_OK" "$OS_REASON" "$APP_QUIT" \
    > "$EVID/cold-launch-driver-status.json"
  log "status ok=$1 reason=$2"
  ls -la "$EVID" > "$EVID/evidence-inventory.txt" 2>&1 || true
}

log "harness start repo=$REPO_ROOT evid=$EVID app=$APP_BIN stage1cap=${STAGE1_CAP_SECS}s stage2cap=${STAGE2_CAP_SECS}s wdio-cap=${WDIO_CAP_SECS}s"

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
echo "repo root:    $REPO_ROOT" >> "$EVID/paths.txt"

# --- stage 1: OS-level cold launch (no WebDriver) ------------------------
STAGE1_DEADLINE=$(( $(now) + STAGE1_CAP_SECS ))

# Compile the window probe ONCE (run-1 lesson: invoking the swift
# interpreter per poll iteration wasted a compile each time; a binary
# poll costs milliseconds).
PROBE_BIN="$EVID/window-probe.bin"
log "stage1 probe compile start"
if ! swiftc "$REPO_ROOT/scripts/macos/window-probe.swift" -o "$PROBE_BIN" > "$EVID/swiftc-compile.log" 2>&1; then
  OS_REASON="swiftc-compile-failed"
  write_status false "stage1-probe-compile-failed"
  exit 0
fi
log "stage1 probe compiled"

"$APP_BIN" > "$EVID/os-launch-app.log" 2>&1 &
APP_PID=$!
log "stage1 app launched pid=$APP_PID"

WINDOW_FOUND=false
STAGE1_TIMEOUT=false
i=0
while [ "$i" -lt 300 ]; do
  if [ "$(now)" -ge "$STAGE1_DEADLINE" ]; then
    STAGE1_TIMEOUT=true
    OS_REASON="stage1-timeout"
    break
  fi
  if "$PROBE_BIN" "$APP_PID" >> "$EVID/os-window-probe.txt" 2>&1; then
    WINDOW_FOUND=true
    break
  fi
  if ! alive "$APP_PID"; then break; fi
  sleep 1
  i=$((i + 1))
done
log "stage1 poll done window=$WINDOW_FOUND timeout=$STAGE1_TIMEOUT iterations=$i"

if [ "$STAGE1_TIMEOUT" = true ]; then
  kill_bounded "$APP_PID" 15
  echo "stage1-timeout app exit code: $REAP_RC" >> "$EVID/os-launch-app.log"
  shot "$EVID/os-cold-launch-failure.png"
  write_status false "stage1-timeout-no-window"
  exit 0
fi

if [ "$WINDOW_FOUND" = true ]; then
  sleep 3   # settle, then capture the OS-level first-usable-state evidence
  shot "$EVID/os-cold-launch.png"
  if alive "$APP_PID"; then
    kill_bounded "$APP_PID" 15
    echo "os probe app exit code: $REAP_RC (killed-9: $KILLED)" >> "$EVID/os-launch-app.log"
    if [ "$KILLED" = false ]; then
      # terminated by our TERM within the bound — the graceful path; the
      # exit code itself (0 handled, 143 default-TERM) is recorded above
      OS_OK=true
      OS_REASON="window-mapped-graceful-quit"
      APP_QUIT=true
    else
      OS_REASON="window-mapped-but-term-hang-killed-9"
    fi
  else
    OS_REASON="app-died-after-window"
  fi
else
  OS_REASON="${OS_REASON:-window-not-found}"
  shot "$EVID/os-cold-launch-failure.png"
  # The app may still be alive and healthy (probe failure != app failure):
  # reap it BOUNDED, recording its exit code — never a bare wait (run-1).
  kill_bounded "$APP_PID" 15
  echo "window-not-found app exit code: $REAP_RC" >> "$EVID/os-launch-app.log"
  OS_REASON="window-not-found-app-exit-$REAP_RC"
fi
log "stage1 done os-ok=$OS_OK reason=$OS_REASON"

# --- stage 2: WebdriverIO embedded-driver session (design mandate) -------
STAGE2_DEADLINE=$(( $(now) + STAGE2_CAP_SECS ))
log "stage2 start"

cd "$SPIKE_DIR" || {
  write_status false "stage2-spike-dir-missing"
  exit 0
}

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
log "stage2 wdio done rc=$WDIO_RC"

# Clean-quit check: the service must tear the spawned app down with the
# session; a lingering rdp-client process is a failed clean quit.
sleep 2
CLEAN_QUIT="$APP_QUIT"
if pgrep -x rdp-client >/dev/null 2>&1; then
  CLEAN_QUIT=false
fi
if [ "$(now)" -ge "$STAGE2_DEADLINE" ]; then
  pkill -TERM -x rdp-client 2>/dev/null || true
  sleep 3
  pkill -9 -x rdp-client 2>/dev/null || true
  shot "$EVID/driver-failure-screen.png"
  write_status false "stage2-timeout"
  exit 0
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
log "harness end"

exit 0
