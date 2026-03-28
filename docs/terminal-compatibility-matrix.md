# Terminal Compatibility Matrix

This matrix tracks compatibility expectations when running inside nested terminal environments.

## Automated Coverage

Automated tests currently cover:

- PTY resize storm handling (`handles_resize_storm_without_failure`)
- Bracketed paste payload roundtrip (`supports_bracketed_paste_payload_roundtrip`)
- Interrupt propagation to long-running process (`interrupt_stops_long_running_process`)

## Manual Matrix

Run the manual script in each environment combination:

- plain terminal
- inside tmux
- inside zellij
- nested tmux->zellij (if available)

Scenarios:

1. Rapid resize sequence
2. Bracketed paste input
3. Interrupt (`Ctrl-C`) behavior
4. Recovery after forced exit

## Current Status

- tmux: pending manual verification
- zellij: pending manual verification
- nested tmux/zellij: pending manual verification
