# PTY Architecture Refactor - In Progress

## Status: BLOCKED - Terminal rendering not working

## Goal
Refactor PTY runtime to zellij-aligned architecture for smooth 60fps terminal rendering.

## Problem Statement
The original implementation used a single-threaded event loop with 120ms poll interval (~8 FPS), causing:
- Laggy text rendering when running terminal apps inside TUI
- Poor responsiveness for fast-updating apps like opencode

## What Was Done

### Phase 1: New Crates and Dependencies (COMPLETE)
- Created `terminal-surface` crate with custom VTE parser (`ScreenGrid`)
- Added `tokio` and `crossbeam-channel` to workspace dependencies
- Added bounded channel support to `runtime-pty`

### Phase 2: Event Bus Infrastructure (COMPLETE)
- Created `SessionEvent` enum for PTY events
- Created `RenderRequest` for render signaling
- Implemented `ScreenThread` with dedicated VTE processing thread

### Phase 3: Session Manager (COMPLETE)
- Created `SessionManager` in tui-app
- Manages PTY sessions with reader threads
- Uses channels for event communication

### Phase 4: TUI Integration (IN PROGRESS - BLOCKED)
- Refactored `board_poc.rs` to use `SessionManager`
- Changed from 120ms poll to 16ms poll (60fps target)
- **BLOCKING ISSUE**: Terminal content not rendering in right pane

## Current Architecture

```
┌──────────────────┐     bounded channel     ┌──────────────────┐
│ PTY Reader       │ ───────────────────────▶│ ScreenThread     │
│  per-session     │   PtyBytes event        │  vt100::Parser   │
│  thread          │   Render signal         │  HashMap<id,parser>
└──────────────────┘                         └──────────────────┘
        ▲                                            │
        │ input                                      ▼
┌───────┴──────────┐                         ┌──────────────────┐
│ Main Thread      │                         │ screen_store     │
│  16ms poll       │◀─────────────────────── │  Arc<Mutex>      │
│  render from     │   RenderRequest         └──────────────────┘
│  screen_store    │
└──────────────────┘
```

## Key Files Changed

| File | Changes |
|------|---------|
| `crates/terminal-surface/src/lib.rs` | NEW - VTE parser, ScreenGrid (not currently used) |
| `crates/runtime-pty/src/lib.rs` | Async PTY support, `take_output_receiver()` |
| `crates/orchestrator-core/src/session_bus.rs` | NEW - SessionEvent, ScreenThread |
| `crates/tui-app/src/session_manager.rs` | NEW - SessionManager |
| `crates/tui-app/src/board_poc.rs` | Uses SessionManager, 16ms poll |

## Blocking Issue

Terminal content is being processed but not displayed:
- Debug logs show `vt100::Parser` receiving bytes and cursor moving
- `parser.screen().rows(0, pane_w)` returns content
- But right pane shows empty/black screen

### Debug Evidence
From `/tmp/ddak-debug.log`:
```
DEBUG: session xxx state=cols=84 rows=34 cursor=(17,5) in_alt_screen=false non_empty_rows=[(10, "OpenCode ASCII art...")]
```

Content IS in the parser, but not rendered to screen.

## Architecture Choices Made

1. **vt100 crate over custom parser**: Switched back to `vt100::Parser` because:
   - Original implementation used it successfully
   - Custom `terminal-surface::ScreenGrid` was having rendering issues
   - `vt100::Parser` is battle-tested

2. **Thread-per-session for PTY reading**: Each PTY session has its own reader thread that sends bytes to the screen thread via channels

3. **Separate screen thread**: Dedicated thread for VTE parsing, decoupled from main UI thread

4. **Bounded channels**: 50-item channel size for backpressure

5. **16ms poll interval**: Changed from 120ms to 16ms for 60fps target

## Next Steps for Next Agent

1. **Debug why vt100::Screen content not rendering**:
   - Check if `pane_w` parameter is correct
   - Verify `screen_store` is being read correctly
   - Check if rows() is returning correct data
   - Compare with original working implementation at commit `66a9688`

2. **Possible causes**:
   - Screen size mismatch (parser created with different dimensions)
   - Content is in scrollback buffer, not visible area
   - Lock contention issue
   - Race condition between screen thread and render

3. **Debugging approach**:
   - Add logging in render path to see actual content being rendered
   - Check `parser.screen().size()` vs pane dimensions
   - Try `parser.screen().rows_formatted()` for formatted output

## Reverting

If needed, revert to commit `66a9688` which had working terminal rendering:
```bash
git checkout 66a9688 -- crates/tui-app/src/board_poc.rs
```

## Tests Passing

```
cargo test -p orchestrator-core session_bus  # 3 passing
cargo test -p tui-app session_manager        # 3 passing
cargo test -p terminal-surface               # 8 passing
cargo test -p runtime-pty                    # 14 passing
```

## Lint Status
All clippy warnings resolved, `cargo clippy --workspace --all-targets -- -D warnings` passes.
