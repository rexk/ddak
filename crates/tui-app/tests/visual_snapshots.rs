use std::time::Duration;

use crossbeam_channel::Receiver;
use orchestrator_core::{RenderRequest, ScreenStore};

fn wait_for_screen(
    store: &ScreenStore,
    session_id: &str,
    render_rx: &Receiver<RenderRequest>,
    predicate: impl Fn(&vt100::Screen) -> bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        while render_rx.try_recv().is_ok() {}
        {
            let store = store.lock().map_err(|e| format!("lock poisoned: {e}"))?;
            if let Some(parser) = store.get(session_id)
                && predicate(parser.screen())
            {
                return Ok(());
            }
        }
        if std::time::Instant::now() >= deadline {
            let store = store.lock().map_err(|e| format!("lock poisoned: {e}"))?;
            let dump = if let Some(parser) = store.get(session_id) {
                screen_text_dump(parser.screen())
            } else {
                format!("session '{session_id}' not found")
            };
            return Err(format!("timed out.\nScreen:\n{dump}"));
        }
        let remaining = deadline - std::time::Instant::now();
        let _ = render_rx.recv_timeout(remaining.min(Duration::from_millis(50)));
    }
}

fn screen_text_dump(screen: &vt100::Screen) -> String {
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
        out.push_str(&format!("{row:3}| {}\n", line.trim_end()));
    }
    out
}

fn snapshot_vt100_screen(screen: &vt100::Screen) -> String {
    use std::fmt::Write;

    let (rows, cols) = screen.size();
    let mut out = String::new();
    let _ = writeln!(out, "[{cols}x{rows}]");

    let mut last_nonempty = 0;
    for row in 0..rows {
        let line: String = (0..cols)
            .map(|col| {
                screen
                    .cell(row, col)
                    .map(|c| c.contents().chars().next().unwrap_or(' '))
                    .unwrap_or(' ')
            })
            .collect();
        if line.trim_end() != "" {
            last_nonempty = row + 1;
        }
    }

    for row in 0..last_nonempty {
        let line: String = (0..cols)
            .map(|col| {
                screen
                    .cell(row, col)
                    .map(|c| c.contents().chars().next().unwrap_or(' '))
                    .unwrap_or(' ')
            })
            .collect();
        let _ = writeln!(out, "{row:>3}: {:?}", line.trim_end());

        // Check for non-default attrs on this row
        let has_attrs = (0..cols).any(|col| {
            screen.cell(row, col).is_some_and(|c| {
                c.bold()
                    || c.italic()
                    || c.underline()
                    || c.inverse()
                    || !matches!(c.fgcolor(), vt100::Color::Default)
                    || !matches!(c.bgcolor(), vt100::Color::Default)
            })
        });

        if has_attrs {
            let mut spans = Vec::new();
            let mut span_start = 0u16;
            let mut span_fg = screen
                .cell(row, 0)
                .map(|c| c.fgcolor())
                .unwrap_or(vt100::Color::Default);
            let mut span_bg = screen
                .cell(row, 0)
                .map(|c| c.bgcolor())
                .unwrap_or(vt100::Color::Default);
            let mut span_bold = screen.cell(row, 0).is_some_and(|c| c.bold());
            let mut span_italic = screen.cell(row, 0).is_some_and(|c| c.italic());
            let mut span_underline = screen.cell(row, 0).is_some_and(|c| c.underline());
            let mut span_inverse = screen.cell(row, 0).is_some_and(|c| c.inverse());

            for col in 1..cols {
                let cell = screen.cell(row, col);
                let fg = cell.map(|c| c.fgcolor()).unwrap_or(vt100::Color::Default);
                let bg = cell.map(|c| c.bgcolor()).unwrap_or(vt100::Color::Default);
                let bold = cell.is_some_and(|c| c.bold());
                let italic = cell.is_some_and(|c| c.italic());
                let underline = cell.is_some_and(|c| c.underline());
                let inverse = cell.is_some_and(|c| c.inverse());

                if fg != span_fg
                    || bg != span_bg
                    || bold != span_bold
                    || italic != span_italic
                    || underline != span_underline
                    || inverse != span_inverse
                {
                    let is_default = matches!(span_fg, vt100::Color::Default)
                        && matches!(span_bg, vt100::Color::Default)
                        && !span_bold
                        && !span_italic
                        && !span_underline
                        && !span_inverse;
                    if !is_default {
                        let mut parts = Vec::new();
                        match span_fg {
                            vt100::Color::Default => {}
                            vt100::Color::Idx(i) => parts.push(format!("fg:Idx({i})")),
                            vt100::Color::Rgb(r, g, b) => {
                                parts.push(format!("fg:Rgb({r},{g},{b})"))
                            }
                        }
                        match span_bg {
                            vt100::Color::Default => {}
                            vt100::Color::Idx(i) => parts.push(format!("bg:Idx({i})")),
                            vt100::Color::Rgb(r, g, b) => {
                                parts.push(format!("bg:Rgb({r},{g},{b})"))
                            }
                        }
                        if span_bold {
                            parts.push("bold".into());
                        }
                        if span_italic {
                            parts.push("italic".into());
                        }
                        if span_underline {
                            parts.push("underline".into());
                        }
                        if span_inverse {
                            parts.push("inverse".into());
                        }
                        spans.push(format!("[{span_start}..{col}]={}", parts.join(",")));
                    }
                    span_start = col;
                    span_fg = fg;
                    span_bg = bg;
                    span_bold = bold;
                    span_italic = italic;
                    span_underline = underline;
                    span_inverse = inverse;
                }
            }
            // Final span
            let is_default = matches!(span_fg, vt100::Color::Default)
                && matches!(span_bg, vt100::Color::Default)
                && !span_bold
                && !span_italic
                && !span_underline
                && !span_inverse;
            if !is_default {
                let mut parts = Vec::new();
                match span_fg {
                    vt100::Color::Default => {}
                    vt100::Color::Idx(i) => parts.push(format!("fg:Idx({i})")),
                    vt100::Color::Rgb(r, g, b) => parts.push(format!("fg:Rgb({r},{g},{b})")),
                }
                match span_bg {
                    vt100::Color::Default => {}
                    vt100::Color::Idx(i) => parts.push(format!("bg:Idx({i})")),
                    vt100::Color::Rgb(r, g, b) => parts.push(format!("bg:Rgb({r},{g},{b})")),
                }
                if span_bold {
                    parts.push("bold".into());
                }
                if span_italic {
                    parts.push("italic".into());
                }
                if span_underline {
                    parts.push("underline".into());
                }
                if span_inverse {
                    parts.push("inverse".into());
                }
                spans.push(format!("[{span_start}..{cols}]={}", parts.join(",")));
            }

            if !spans.is_empty() {
                let _ = writeln!(out, "{row:>3}: attrs {}", spans.join(" "));
            }
        }
    }

    out
}

const TIMEOUT: Duration = Duration::from_secs(3);

fn new_session_manager() -> tui_app::session_manager::SessionManager {
    tui_app::session_manager::SessionManager::new()
}

// ── Colored output through PTY ──────────────────────────────────────

#[test]
fn pty_colored_output_snapshot() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    mgr.spawn_session(
        "s1",
        "/bin/sh",
        &["-c", "printf '\\033[1;31mRed Bold\\033[0m \\033[32mGreen\\033[0m \\033[38;5;214mOrange256\\033[0m'"],
    )
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
    let snapshot = snapshot_vt100_screen(parser.screen());
    insta::assert_snapshot!("pty_colored_output", snapshot);
}

// ── Cursor addressing through PTY ───────────────────────────────────

#[test]
fn pty_cursor_addressing_snapshot() {
    let mut mgr = new_session_manager();
    let store = mgr.screen_store();
    let rx = mgr.render_receiver();

    mgr.spawn_session(
        "s1",
        "/bin/sh",
        &["-c", "printf '\\033[3;10HX\\033[1;1HA\\033[5;20HB'"],
    )
    .unwrap();

    let result = wait_for_screen(
        &store,
        "s1",
        &rx,
        |screen| screen.cell(4, 19).is_some_and(|c| c.contents() == "B"),
        TIMEOUT,
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());

    let store = store.lock().unwrap();
    let parser = store.get("s1").unwrap();
    let snapshot = snapshot_vt100_screen(parser.screen());
    insta::assert_snapshot!("pty_cursor_addressing", snapshot);
}
