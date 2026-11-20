#!/usr/bin/env bash
# Profile the running 1-1 app on Linux while it is in use.
# See docs/profiling-linux.md. One-time setup: sudo scripts/linux/profile-setup.sh
#
#   profile.sh start [--wait]  record the app and its web process in the
#                              background (--wait: until 1-1 starts)
#   profile.sh dump            save the last ~seconds as perf.data.<time>
#   profile.sh report [file]   summary of a dump (default: the newest)
#   profile.sh cpu             print main-thread and web-process CPU each second
#   profile.sh stop            stop recording and the CPU log
#   profile.sh status
#
# Each start writes a new $PROFILE_DIR/run-<time>/ (default
# ~/.cache/1-1-profiles) with its dumps, recorder.log and cpu.log.
set -euo pipefail

PERF="${PERF:-/usr/local/bin/perf-cap}"
DIR="${PROFILE_DIR:-$HOME/.cache/1-1-profiles}"
PIDFILE="$DIR/recorder.pid"
CPUPIDFILE="$DIR/cpu.pid"
RUNFILE="$DIR/current-run"

app_pid() { pgrep -x rdp-client | head -n1; }
web_pids() { pgrep -P "$1" -f WebKitWebProcess | paste -sd, || true; }
running() { [[ -f "$1" ]] && kill -0 "$(cat "$1")" 2>/dev/null; }

need_perf() {
  [[ -x "$PERF" ]] || { echo "$PERF is missing: run sudo scripts/linux/profile-setup.sh" >&2; exit 1; }
}

cpu_log() {
  local app="$1" web="$2" hz pm pw m w
  hz="$(getconf CLK_TCK)"
  ticks() { awk '{print $14 + $15}' "$1"; }
  pm="$(ticks "/proc/$app/task/$app/stat")"; pw="$(ticks "/proc/$web/stat")"
  while sleep 1; do
    m="$(ticks "/proc/$app/task/$app/stat" 2>/dev/null)" || exit 0
    w="$(ticks "/proc/$web/stat" 2>/dev/null)" || exit 0
    echo "$(date +%T) main=$(((m - pm) * 100 / hz))% web=$(((w - pw) * 100 / hz))%"
    pm="$m"; pw="$w"
  done
}

case "${1:-}" in
start)
  need_perf
  mkdir -p "$DIR"
  if running "$PIDFILE"; then echo "already recording (pid $(cat "$PIDFILE"))"; exit 0; fi
  app="$(app_pid || true)"
  if [[ -z "$app" && "${2:-}" == "--wait" ]]; then
    echo "waiting for 1-1 to start..."
    until app="$(app_pid)" && [[ -n "$(web_pids "$app")" ]]; do sleep 1; done
    sleep 2
  fi
  [[ -n "$app" ]] || { echo "1-1 is not running (use start --wait)" >&2; exit 1; }
  web="$(web_pids "$app")"
  run="$DIR/run-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$run"
  echo "$run" >"$RUNFILE"
  # Frame-pointer call graphs: cheap enough to leave running. --overwrite keeps
  # only the newest samples; 128 KiB per CPU stays under the default 4 MiB
  # perf_event_mlock_kb.
  nohup "$PERF" record -F 299 -g --overwrite -m 128K --switch-output=signal \
    -p "$app${web:+,$web}" -o "$run/perf.data" >"$run/recorder.log" 2>&1 &
  echo $! >"$PIDFILE"
  nohup "$0" _cpu "$app" "${web%%,*}" >"$run/cpu.log" 2>&1 &
  echo $! >"$CPUPIDFILE"
  echo "recording app $app, web ${web:-none} into $run"
  ;;
_cpu)
  cpu_log "$2" "$3"
  ;;
cpu)
  tail -f "$(cat "$RUNFILE")/cpu.log"
  ;;
dump)
  running "$PIDFILE" || { echo "not recording (profile.sh start)" >&2; exit 1; }
  log="$(cat "$RUNFILE")/recorder.log"
  before="$(grep -c 'Dump ' "$log" || true)"
  kill -USR2 "$(cat "$PIDFILE")"
  for _ in $(seq 50); do
    [[ "$(grep -c 'Dump ' "$log" || true)" -gt "$before" ]] && break
    sleep 0.2
  done
  grep -o 'Dump [^ ]*' "$log" | tail -n1 | cut -d' ' -f2
  ;;
report)
  need_perf
  file="${2:-$(ls -t "$DIR"/run-*/perf.data.* 2>/dev/null | head -n1)}"
  [[ -f "$file" ]] || { echo "no dump yet (profile.sh dump)" >&2; exit 1; }
  # Symbol downloads (debuginfod) make the first report slow; set
  # DEBUGINFOD_URLS=https://debuginfod.ubuntu.com to resolve WebKit and GTK.
  export DEBUGINFOD_URLS="${DEBUGINFOD_URLS-}"
  rep() { "$PERF" report -i "$file" --stdio -g none "$@" 2>/dev/null | grep -E '^ +[0-9]' || true; }
  echo "== $file"
  echo "== time per thread";            rep --no-children --sort comm,pid | head -12
  echo "== time per library";           rep --no-children --sort comm,dso | head -15
  echo "== main thread, inclusive (paint path)"
  rep --children --comm rdp-client --sort sym \
    | grep -E 'gtk_main_do_event|cairo_fill|pixman_fill|g_main_context_iteration|app_lib' | head -12
  echo "== main thread, self";          rep --no-children --comm rdp-client --sort sym | head -15
  ;;
stop)
  # perf flushes on SIGINT; the CPU log is a background shell, which ignores it.
  for entry in "$PIDFILE:INT" "$CPUPIDFILE:TERM"; do
    f="${entry%:*}"
    if running "$f"; then
      pid="$(cat "$f")"
      kill -"${entry##*:}" "$pid"
      # perf writes its last data on exit; wait so a new run cannot collide.
      while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
    fi
    rm -f "$f"
  done
  echo "stopped"
  ;;
status)
  running "$PIDFILE" && echo "recording (pid $(cat "$PIDFILE"))" || echo "not recording"
  ls -t "$DIR"/run-*/perf.data.* 2>/dev/null | head -n5 || true
  ;;
*)
  sed -n '2,13p' "$0" >&2
  exit 2
  ;;
esac
