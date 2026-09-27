#!/usr/bin/env bash
# Runs a scenario against a deployment and fails if it no longer holds up, or is slower than a
# baseline report by more than the tolerance. For CI, from the repository root, with the
# deployment's aspen.toml (database and NATS) in the working directory:
#
#   bench/scripts/regression.sh [baseline.json] [scenario] [tolerance]
#
# Without a baseline it only judges the service levels. The seeded population is purged however
# the run ends. The run's report is left in bench-report/.
set -euo pipefail
baseline="${1:-}"
scenario="${2:-smoke}"
tolerance="${3:-0.1}"
bin="${ASPEN_BIN:-target/release}"
run="ci-$(date +%s)"
work="$(mktemp -d)"
cleanup() {
  "$bin/aspen-chat-server" bench purge --run "$run" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
"$bin/aspen-bench" plan "$scenario" --run "$run" --out "$work/plan.json"
"$bin/aspen-chat-server" bench seed --plan "$work/plan.json" --out "$work/manifest.json"
status=0
"$bin/aspen-bench" run "$scenario" --manifest "$work/manifest.json" --out bench-report || status=$?
if [ "$status" -ne 0 ]; then
  echo "the $scenario scenario did not hold up (exit $status)"
  exit "$status"
fi
if [ -n "$baseline" ]; then
  "$bin/aspen-bench" compare "$baseline" bench-report/report.json --tolerance "$tolerance"
fi
