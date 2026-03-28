use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, bounded};

pub const DEFAULT_CHANNEL_SIZE: usize = 50;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    PtyBytes {
        session_id: String,
        bytes: Vec<u8>,
    },
    Render {
        session_id: String,
    },
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    SessionExited {
        session_id: String,
        exit_code: i32,
    },
    SessionStarted {
        session_id: String,
    },
    Input {
        session_id: String,
        bytes: Vec<u8>,
    },
}

impl SessionEvent {
    pub fn session_id(&self) -> &str {
        match self {
            SessionEvent::PtyBytes { session_id, .. } => session_id,
            SessionEvent::Render { session_id } => session_id,
            SessionEvent::Resize { session_id, .. } => session_id,
            SessionEvent::SessionExited { session_id, .. } => session_id,
            SessionEvent::SessionStarted { session_id } => session_id,
            SessionEvent::Input { session_id, .. } => session_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub session_id: String,
}

pub type ScreenStore = Arc<std::sync::Mutex<std::collections::HashMap<String, vt100::Parser>>>;

pub struct ScreenThread {
    handle: Option<JoinHandle<()>>,
    screen_store: ScreenStore,
}

impl ScreenThread {
    pub fn spawn(
        initial_cols: u16,
        initial_rows: u16,
    ) -> (Self, Sender<SessionEvent>, Receiver<RenderRequest>) {
        let (event_tx, event_rx): (Sender<SessionEvent>, Receiver<SessionEvent>) =
            bounded(DEFAULT_CHANNEL_SIZE);
        let (render_tx, render_rx): (Sender<RenderRequest>, Receiver<RenderRequest>) =
            bounded(DEFAULT_CHANNEL_SIZE);

        let screen_store: ScreenStore =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let store = screen_store.clone();

        let handle = std::thread::spawn(move || {
            while let Ok(event) = event_rx.recv() {
                let session_id = event.session_id().to_string();
                match event {
                    SessionEvent::PtyBytes { bytes, .. } => {
                        let mut store = store.lock().unwrap();
                        let parser = store.entry(session_id.clone()).or_insert_with(|| {
                            vt100::Parser::new(initial_rows, initial_cols, 1000)
                        });
                        parser.process(&bytes);
                        drop(store);
                        let _ = render_tx.send(RenderRequest { session_id });
                    }
                    SessionEvent::Resize { cols, rows, .. } => {
                        let mut store = store.lock().unwrap();
                        if let Some(parser) = store.get_mut(&session_id) {
                            parser.screen_mut().set_size(rows, cols);
                        }
                    }
                    SessionEvent::SessionExited { .. } => {
                        let mut store = store.lock().unwrap();
                        store.remove(&session_id);
                    }
                    SessionEvent::SessionStarted { .. } => {
                        let mut store = store.lock().unwrap();
                        store.entry(session_id).or_insert_with(|| {
                            vt100::Parser::new(initial_rows, initial_cols, 1000)
                        });
                    }
                    SessionEvent::Render { .. } => {
                        let _ = render_tx.send(RenderRequest { session_id });
                    }
                    SessionEvent::Input { .. } => {}
                }
            }
        });

        (
            Self {
                handle: Some(handle),
                screen_store,
            },
            event_tx,
            render_rx,
        )
    }

    pub fn screen_store(&self) -> ScreenStore {
        self.screen_store.clone()
    }

    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Drain `render_rx` until the predicate is satisfied on the session's screen,
/// or until timeout. Returns `Err` with a screen dump on failure.
pub fn wait_for_screen_content(
    store: &ScreenStore,
    session_id: &str,
    render_rx: &Receiver<RenderRequest>,
    predicate: impl Fn(&vt100::Screen) -> bool,
    timeout: std::time::Duration,
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
            let dump = match store.get(session_id) {
                Some(parser) => screen_dump_with_attrs(parser.screen()),
                None => format!("session '{session_id}' not found in store"),
            };
            return Err(format!("timed out waiting for screen predicate.\n{dump}"));
        }

        let remaining = deadline - std::time::Instant::now();
        let _ = render_rx.recv_timeout(remaining.min(std::time::Duration::from_millis(50)));
    }
}

/// Dump screen content showing `[char|fg|bold]` per non-empty cell for debugging.
pub fn screen_dump_with_attrs(screen: &vt100::Screen) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn session_event_session_id_works() {
        let event = SessionEvent::PtyBytes {
            session_id: "sess-1".to_string(),
            bytes: vec![1, 2, 3],
        };
        assert_eq!(event.session_id(), "sess-1");
    }

    #[test]
    fn screen_thread_processes_bytes() {
        let (screen_thread, event_tx, render_rx) = ScreenThread::spawn(80, 24);

        event_tx
            .send(SessionEvent::PtyBytes {
                session_id: "sess-1".to_string(),
                bytes: b"Hello".to_vec(),
            })
            .unwrap();

        let render = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        assert_eq!(render.session_id, "sess-1");

        let store = screen_thread.screen_store();
        let screen = store.lock().unwrap();
        let parser = screen.get("sess-1").unwrap();
        let content: Vec<String> = parser
            .screen()
            .rows(0, 80)
            .map(|l| l.trim_end().to_string())
            .collect();
        assert!(content[0].contains("Hello"));

        std::mem::drop(screen_thread);
    }

    #[test]
    fn screen_thread_handles_resize() {
        let (screen_thread, event_tx, _render_rx) = ScreenThread::spawn(80, 24);

        event_tx
            .send(SessionEvent::SessionStarted {
                session_id: "sess-1".to_string(),
            })
            .unwrap();

        event_tx
            .send(SessionEvent::Resize {
                session_id: "sess-1".to_string(),
                cols: 120,
                rows: 40,
            })
            .unwrap();

        std::thread::sleep(Duration::from_millis(50));

        let store = screen_thread.screen_store();
        let screen = store.lock().unwrap();
        let parser = screen.get("sess-1").unwrap();
        assert_eq!(parser.screen().size().0, 40);
        assert_eq!(parser.screen().size().1, 120);

        std::mem::drop(screen_thread);
    }

    #[test]
    fn alt_screen_content_visible_via_rows() {
        let (screen_thread, event_tx, render_rx) = ScreenThread::spawn(80, 24);

        // Simulate a TUI app: enter alt-screen, write content
        let mut seq = Vec::new();
        seq.extend_from_slice(b"\x1b[?1049h"); // enter alt-screen
        seq.extend_from_slice(b"\x1b[H"); // cursor home
        seq.extend_from_slice(b"Claude Code v1.0.0");
        seq.extend_from_slice(b"\x1b[2;1H"); // row 2
        seq.extend_from_slice(b"> Ready");

        event_tx
            .send(SessionEvent::PtyBytes {
                session_id: "sess-1".to_string(),
                bytes: seq,
            })
            .unwrap();

        let _ = render_rx.recv_timeout(Duration::from_millis(100));

        let store = screen_thread.screen_store();
        let lock = store.lock().unwrap();
        let parser = lock.get("sess-1").unwrap();
        let screen = parser.screen();

        assert!(screen.alternate_screen(), "should be in alt-screen mode");

        let rows: Vec<String> = screen
            .rows(0, 80)
            .map(|l| l.trim_end().to_string())
            .collect();

        eprintln!("Alt-screen rows[0]: {:?}", rows[0]);
        eprintln!("Alt-screen rows[1]: {:?}", rows[1]);
        eprintln!("Total rows: {}", rows.len());

        assert!(
            rows[0].contains("Claude Code"),
            "row 0 should have content, got: {:?}",
            rows[0]
        );
        assert!(
            rows[1].contains("Ready"),
            "row 1 should have content, got: {:?}",
            rows[1]
        );

        std::mem::drop(screen_thread);
    }
}
