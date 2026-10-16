#!/usr/bin/env bash
# Record game-day phase timestamps (see docs/runbooks/game-day.md).
#   scripts/game_day_timer.sh S1 inject|detected|contained|recovered [csv]
set -euo pipefail
scenario="${1:?scenario (S1|S2|S3)}"
phase="${2:?phase (inject|detected|contained|recovered)}"
csv="${3:-game-day-timeline.csv}"
case "$phase" in inject|detected|contained|recovered) ;; *) echo "bad phase: $phase" >&2; exit 2 ;; esac
ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "${ts},${scenario},${phase}" | tee -a "$csv"
