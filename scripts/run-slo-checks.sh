#!/usr/bin/env bash
set -euo pipefail

echo "Running reliability SLO checks"
echo

export RUST_TEST_THREADS=1

echo "1) p95 input-to-echo latency"
cargo test -p runtime-pty slo_latency_p95_local_echo_under_budget -- --nocapture

echo
echo "2) restart recovery timing"
cargo test -p runtime-pty slo_restart_recovery_time_under_budget -- --nocapture

echo
echo "3) concurrent action-loss accounting"
cargo test -p transport-stdio slo_no_action_loss_under_concurrent_writes -- --nocapture

echo
echo "SLO checks completed"
