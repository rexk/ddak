# Reliability SLO Verification

This document captures the benchmark-style checks used for v1 reliability validation.

## SLO Checks

1. p95 input-to-echo latency
   - Test: `slo_latency_p95_local_echo_under_budget`
   - Command: `cargo test -p runtime-pty slo_latency_p95_local_echo_under_budget -- --nocapture`
   - Budget: `<= 150ms`

2. Restart recovery time
   - Test: `slo_restart_recovery_time_under_budget`
   - Command: `cargo test -p runtime-pty slo_restart_recovery_time_under_budget -- --nocapture`
   - Budget: `<= 250ms` for fixed test fixture

3. Action-loss accounting under concurrency
   - Test: `slo_no_action_loss_under_concurrent_writes`
   - Command: `cargo test -p transport-stdio slo_no_action_loss_under_concurrent_writes -- --nocapture`
   - Invariant: `submitted == success + conflicts`

## Runner Script

Run all checks in sequence:

```bash
scripts/run-slo-checks.sh
```
