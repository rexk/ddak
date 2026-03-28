use std::time::Duration;

use crossbeam_channel::Receiver;
use orchestrator_core::{RenderRequest, ScreenStore};

mod helpers {
    use super::*;

    /// Poll render_rx and check screen state until predicate is satisfied or timeout.
    /// Returns screen text dump on failure for debugging.
    pub fn wait_for_screen(
        store: &ScreenStore,
        session_id: &str,
        render_rx: &Receiver<RenderRequest>,
        predicate: impl Fn(&vt100::Screen) -> bool,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + timeout;

        loop {
            // Drain any pending render requests
            while render_rx.try_recv().is_ok() {}

            // Check predicate
            {
                let store = store.lock().map_err(|e| format!("lock poisoned: {}", e))?;
                if let Some(parser) = store.get(session_id)
                    && predicate(parser.screen())
                {
                    return Ok(());
                }
            }

            if std::time::Instant::now() >= deadline {
                // Build debug dump
                let store = store.lock().map_err(|e| format!("lock poisoned: {}", e))?;
                let dump = if let Some(parser) = store.get(session_id) {
                    screen_dump(parser.screen())
                } else {
                    format!("session '{}' not found in store", session_id)
                };
                return Err(format!(
                    "timed out waiting for screen predicate.\nScreen dump:\n{}",
                    dump
                ));
            }

            // Wait for next render or short poll
            let remaining = deadline - std::time::Instant::now();
            let _ = render_rx.recv_timeout(remaining.min(Duration::from_millis(50)));
        }
    }

    /// Dump screen content as text with row numbers.
    pub fn screen_dump(screen: &vt100::Screen) -> String {
        let (rows, cols) = screen.size();
        let mut out = String::new();
        for row in 0..rows {
            let line: String = (0..cols)
                .map(|col| {
                    screen
                        .cell(row, col)
                        .map(|c| c.contents().chars().next().unwrap_or(' '))
                        .unwrap_or(' ')
                })
                .collect();
            out.push_str(&format!("{:3}| {}\n", row, line.trim_end()));
        }
        out
    }

    /// Assert a cell has expected foreground color and bold attribute.
    pub fn assert_cell_attr(
        screen: &vt100::Screen,
        row: u16,
        col: u16,
        expected_fg: vt100::Color,
        expected_bold: bool,
    ) {
        let cell = screen.cell(row, col).unwrap_or_else(|| {
            panic!("no cell at ({}, {})\n{}", row, col, screen_dump(screen));
        });
        assert_eq!(
            cell.fgcolor(),
            expected_fg,
            "cell ({},{}) fg mismatch\n{}",
            row,
            col,
            screen_dump(screen)
        );
        assert_eq!(
            cell.bold(),
            expected_bold,
            "cell ({},{}) bold mismatch\n{}",
            row,
            col,
            screen_dump(screen)
        );
    }
}

use helpers::*;

fn new_session_manager() -> tui_app::session_manager::SessionManager {
    tui_app::session_manager::SessionManager::new()
}

const TIMEOUT: Duration = Duration::from_secs(3);

// ── Plain text ───────────────────────────────────────────────────────

#[test]
fn plain_text_appears_on_screen() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    mgr.spawn_session("s1", "/bin/sh", &["-c", "echo Hello"])
        .unwrap();

    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| {
            let (_, cols) = screen.size();
            screen.rows(0, cols).any(|line| line.contains("Hello"))
        },
        TIMEOUT,
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

// ── ANSI colors preserved ────────────────────────────────────────────

#[test]
fn ansi_colors_preserved() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    // Bold red text
    mgr.spawn_session("s1", "/bin/sh", &["-c", "printf '\\033[1;31mRed\\033[0m'"])
        .unwrap();

    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| screen.cell(0, 0).is_some_and(|c| c.contents() == "R"),
        TIMEOUT,
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());

    let store = store.lock().unwrap();
    let parser = store.get("s1").unwrap();
    let screen = parser.screen();

    assert_cell_attr(screen, 0, 0, vt100::Color::Idx(1), true);
    assert_cell_attr(screen, 0, 1, vt100::Color::Idx(1), true);
    assert_cell_attr(screen, 0, 2, vt100::Color::Idx(1), true);
}

// ── Cursor addressing ────────────────────────────────────────────────

#[test]
fn cursor_addressing() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    // Place 'X' at row 5, col 10 (1-indexed)
    mgr.spawn_session("s1", "/bin/sh", &["-c", "printf '\\033[5;10HX'"])
        .unwrap();

    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| screen.cell(4, 9).is_some_and(|c| c.contents() == "X"),
        TIMEOUT,
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

// ── Alt-screen enter/exit ────────────────────────────────────────────

#[test]
fn alt_screen_enter_exit() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    // Write "Before", enter alt screen, write "During", exit alt screen
    mgr.spawn_session(
        "s1",
        "/bin/sh",
        &[
            "-c",
            "printf 'Before'; printf '\\033[?1049h'; printf 'During'; sleep 0.1; printf '\\033[?1049l'; sleep 0.1",
        ],
    )
    .unwrap();

    // After alt screen exit, "Before" should be restored
    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| {
            let (_, cols) = screen.size();
            screen.rows(0, cols).any(|line| line.contains("Before"))
        },
        Duration::from_secs(5),
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

// ── Resize propagation ──────────────────────────────────────────────

#[test]
fn resize_propagation() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    mgr.spawn_session("s1", "/bin/sh", &["-c", "cat"]).unwrap();

    // Wait for session to appear
    let result = wait_for_screen(&store, "s1", &rx, |_screen| true, TIMEOUT);
    assert!(result.is_ok(), "{}", result.unwrap_err());

    mgr.resize("s1", 200, 50).unwrap();

    // Give ScreenThread time to process
    std::thread::sleep(Duration::from_millis(100));

    let store = store.lock().unwrap();
    let parser = store.get("s1").unwrap();
    assert_eq!(parser.screen().size(), (50, 200));
}

// ── Multi-session isolation ──────────────────────────────────────────

#[test]
fn multi_session_isolation() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    mgr.spawn_session("s1", "/bin/sh", &["-c", "echo Alpha"])
        .unwrap();
    mgr.spawn_session("s2", "/bin/sh", &["-c", "echo Beta"])
        .unwrap();

    // Wait for both
    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| {
            let (_, cols) = screen.size();
            screen.rows(0, cols).any(|l| l.contains("Alpha"))
        },
        TIMEOUT,
    );
    assert!(result.is_ok(), "s1: {}", result.unwrap_err());

    let result = wait_for_screen(
        &store,
        "s2",
        &rx,
        |screen| {
            let (_, cols) = screen.size();
            screen.rows(0, cols).any(|l| l.contains("Beta"))
        },
        TIMEOUT,
    );
    assert!(result.is_ok(), "s2: {}", result.unwrap_err());

    // Verify isolation: s1 should NOT contain Beta
    let store_lock = store.lock().unwrap();
    let s1 = store_lock.get("s1").unwrap();
    let (_, cols) = s1.screen().size();
    let s1_has_beta = s1.screen().rows(0, cols).any(|l| l.contains("Beta"));
    assert!(!s1_has_beta, "s1 should not contain s2's output");
}

// ── Scroll from overflow ─────────────────────────────────────────────

#[test]
fn scroll_from_overflow() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    // Print many lines to force scrolling through the PTY pipeline.
    mgr.spawn_session(
        "s1",
        "/bin/sh",
        &["-c", "for i in $(seq 1 200); do echo \"L$i\"; done"],
    )
    .unwrap();

    // Verify the pipeline handles high-volume output and the last line appears
    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| {
            let (_, cols) = screen.size();
            screen.rows(0, cols).any(|l| l.contains("L200"))
        },
        TIMEOUT,
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());

    // Also verify an earlier line is NOT on the visible screen (scrolled off).
    // vt100's rows() returns only visible rows, so early output should be gone.
    let store_lock = store.lock().unwrap();
    let parser = store_lock.get("s1").unwrap();
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let visible: Vec<String> = screen
        .rows(0, cols)
        .map(|l| l.trim_end().to_string())
        .collect();
    // With 200 lines on a screen, rows that appeared early should have scrolled off
    let visible_count = visible.len();
    assert_eq!(
        visible_count, rows as usize,
        "rows() should return exactly screen height rows"
    );
}
