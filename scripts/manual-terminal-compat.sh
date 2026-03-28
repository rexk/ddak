#!/usr/bin/env bash
set -euo pipefail

echo "Manual terminal compatibility verification"
echo
echo "Environment:"
echo "  TMUX=${TMUX:-<not set>}"
echo "  ZELLIJ=${ZELLIJ:-<not set>}"
echo

echo "1) Running PTY compatibility tests"
cargo test -p runtime-pty handles_resize_storm_without_failure -- --nocapture
cargo test -p runtime-pty supports_bracketed_paste_payload_roundtrip -- --nocapture
cargo test -p runtime-pty interrupt_stops_long_running_process -- --nocapture

echo
echo "2) Manual checks to perform now:"
echo "  - Resize the terminal rapidly while running: cargo run -p tui-app -- --demo-shell"
echo "  - Paste multi-line text into the shell and confirm behavior"
echo "  - Press Ctrl-C during active command and confirm return to prompt"
echo
echo "Done. Record outcomes in docs/terminal-compatibility-matrix.md"
