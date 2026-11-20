#!/usr/bin/env bash
# Model-check every TLC configuration in this directory.
#
#   <Spec>.cfg                      must pass
#   <Spec>.<name>-counterexample.cfg must FAIL (documents an accepted finding)
#
# After the model-check loop, TLC is run in trace-validation mode against the
# JSON/NDJSON traces emitted by the instrumented Rust tests (traces/*.ndjson)
# using a dedicated trace-validation config. Any rejected trace fails the run.
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

# --- Trace validation -------------------------------------------------------
# Validate the traces emitted by the instrumented Rust tests against the spec.
# The trace-validation config is named <Spec>.trace-validation.cfg and the
# spec is <Spec>.tla. Traces live in traces/*.ndjson (one JSON event per line).
TRACE_SPEC="AttestationLifecycle"
TRACE_CFG="${TRACE_SPEC}.trace-validation.cfg"
TRACE_DIR="traces"

if [ -f "$TRACE_CFG" ]; then
  shopt -s nullglob
  traces=("${TRACE_DIR}"/*.ndjson)
  shopt -u nullglob

  if [ "${#traces[@]}" -eq 0 ]; then
    echo "::error::trace validation: no traces found in ${TRACE_DIR}/*.ndjson"
    status=1
  else
    for trace in "${traces[@]}"; do
      echo "=== trace validation: ${TRACE_SPEC}.tla with ${trace}"
      log="$(mktemp)"
      if java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -workers auto -deadlock -noGenerateSpecTE \
          -metadir "$(mktemp -d)" -config "$TRACE_CFG" \
          -DtraceFile="$trace" "${TRACE_SPEC}.tla" > "$log" 2>&1; then
        result=pass
      else
        result=fail
        grep -q "is violated" "$log" && result=violated
      fi
      tail -n 5 "$log"
      if [ "$result" != "pass" ]; then
        echo "::error::trace validation: ${trace} rejected (${result})"
        cat "$log"
        status=1
      fi
      rm -f "$log"
    done
  fi
else
  echo "=== trace validation: ${TRACE_CFG} not present, skipping"
fi

exit "$status"