#!/usr/bin/env bash
# One-time setup so 1-1 can be profiled without sudo while it is in use.
#
# Installs Sysprof (GNOME's trace profiler) and a perf copy with file
# capabilities (not setuid root) that only the given user's group may run:
#   /usr/local/bin/perf-cap  root:<group> 0750  cap_perfmon,cap_sys_ptrace,cap_syslog
# Undo: sudo rm /usr/local/bin/perf-cap
#
# Usage: sudo scripts/linux/profile-setup.sh
set -euo pipefail
OWNER="${SUDO_USER:?run with sudo}"
GROUP="$(id -gn "$OWNER")"

apt-get install -y sysprof libcap2-bin

install -o root -g "$GROUP" -m 0750 "$(command -v perf)" /usr/local/bin/perf-cap
setcap cap_perfmon,cap_sys_ptrace,cap_syslog=ep /usr/local/bin/perf-cap
getcap /usr/local/bin/perf-cap
echo "Ready: perf-cap runs for group $GROUP without sudo."
