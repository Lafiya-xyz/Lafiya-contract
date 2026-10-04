#!/usr/bin/env bash
# Model-check every TLC configuration in this directory.
#
#   <Spec>.cfg                      must pass
#   <Spec>.<name>-counterexample.cfg must FAIL (documents an accepted finding)
#
# Usage: verification/tla/run.sh [path/to/tla2tools.jar]
set -euo pipefail

TLA_VERSION="1.8.0"
TLA_SHA256="7c6a30fcfca96c6d7476e705a545837afbf66446c3fcb34bf39b838cd50ee0c0"

cd "$(dirname "$0")"
JAR="${1:-${TLA2TOOLS_JAR:-.tla2tools/tla2tools-${TLA_VERSION}.jar}}"

if [ ! -f "$JAR" ]; then
  mkdir -p "$(dirname "$JAR")"
  curl -fsSL -o "$JAR" \
    "https://github.com/tlaplus/tlaplus/releases/download/v${TLA_VERSION}/tla2tools.jar"
fi
echo "${TLA_SHA256}  ${JAR}" | sha256sum -c -

status=0
for cfg in *.cfg; do
  spec="${cfg%%.*}"
  echo "=== ${spec}.tla with ${cfg}"
  if java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -workers auto -deadlock -noGenerateSpecTE \
      -metadir "$(mktemp -d)" -config "$cfg" "${spec}.tla" > "${cfg}.log" 2>&1; then
    result=pass
  else
    result=fail
    grep -q "is violated" "${cfg}.log" && result=violated
  fi
  tail -n 5 "${cfg}.log"
  case "$cfg" in
    *-counterexample.cfg) expected=violated ;;
    *) expected=pass ;;
  esac
  if [ "$result" != "$expected" ]; then
    echo "::error::${cfg}: expected ${expected}, got ${result}"
    cat "${cfg}.log"
    status=1
  fi
  rm -f "${cfg}.log"
done
exit "$status"
