use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use orchestrator_core::{RenderRequest, ScreenStore, ScreenThread, SessionEvent};
use runtime_pty::PtyConfig;
use runtime_pty::PtySession;

pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 40;

pub struct SessionHandle {
    #[allow(dead_code)]
    pub session_id: String,
    pub session: PtySession,
}

pub struct SessionManager {
    sessions: HashMap<String, SessionHandle>,
    event_tx: Sender<SessionEvent>,
    render_rx: Receiver<RenderRequest>,
    #[allow(dead_code)]
    screen_thread: ScreenThread,
    screen_store: ScreenStore,
    pty_reader_handles: HashMap<String, JoinHandle<()>>,
    workdir: Option<PathBuf>,
    initial_cols: u16,
    initial_rows: u16,
    pty_config: PtyConfig,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::with_config(None, DEFAULT_COLS, DEFAULT_ROWS, PtyConfig::default())
    }

    pub fn with_config(
        workdir: Option<PathBuf>,
        initial_cols: u16,
        initial_rows: u16,
        pty_config: PtyConfig,
    ) -> Self {
        let (screen_thread, event_tx, render_rx) = ScreenThread::spawn(initial_cols, initial_rows);
        let screen_store = screen_thread.screen_store();

        Self {
            sessions: HashMap::new(),
            event_tx,
            render_rx,
            screen_thread,
            screen_store,
            pty_reader_handles: HashMap::new(),
            workdir,
            initial_cols,
            initial_rows,
            pty_config,
        }
    }

    pub fn set_workdir(&mut self, workdir: Option<PathBuf>) {
        self.workdir = workdir;
    }

    pub fn spawn_session(&mut self, session_id: &str, command: &str, args: &[&str]) -> Result<()> {
        let mut session = PtySession::spawn_with_config(
            command,
            args,
            self.initial_cols,
            self.initial_rows,
            self.workdir.as_deref(),
            self.pty_config.clone(),
        )
        .with_context(|| format!("failed to spawn session {}", session_id))?;

        let output_rx = session
            .take_output_receiver()
            .context("output receiver already taken")?;
        let event_tx = self.event_tx.clone();
        let sid = session_id.to_string();

        event_tx
            .send(SessionEvent::SessionStarted {
                session_id: sid.clone(),
            })
            .context("failed to send SessionStarted event")?;

        let reader_handle = thread::spawn(move || {
            while let Ok(bytes) = output_rx.recv() {
                let _ = event_tx.send(SessionEvent::PtyBytes {
                    session_id: sid.clone(),
                    bytes,
                });
            }
        });

        self.sessions.insert(
            session_id.to_string(),
            SessionHandle {
                session_id: session_id.to_string(),
                session,
            },
        );
        self.pty_reader_handles
            .insert(session_id.to_string(), reader_handle);

        Ok(())
    }

    pub fn send_input(&mut self, session_id: &str, input: &str) -> Result<()> {
        let handle = self
            .sessions
            .get_mut(session_id)
            .with_context(|| format!("session not found: {}", session_id))?;
        handle.session.send_input(input)
    }

    pub fn send_bytes(&mut self, session_id: &str, bytes: &[u8]) -> Result<()> {
        let handle = self
            .sessions
            .get_mut(session_id)
            .with_context(|| format!("session not found: {}", session_id))?;
        handle.session.send_bytes(bytes)
    }

    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()> {
        let handle = self
            .sessions
            .get(session_id)
            .with_context(|| format!("session not found: {}", session_id))?;
        handle.session.resize(cols, rows)?;
        self.event_tx
            .send(SessionEvent::Resize {
                session_id: session_id.to_string(),
                cols,
                rows,
            })
            .context("failed to send Resize event")?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn interrupt(&mut self, session_id: &str) -> Result<()> {
        let handle = self
            .sessions
            .get_mut(session_id)
            .with_context(|| format!("session not found: {}", session_id))?;
        handle.session.interrupt()
    }

    pub fn terminate(&mut self, session_id: &str) -> Result<()> {
        if let Some(mut handle) = self.sessions.remove(session_id) {
            handle.session.terminate()?;
            let _ = self.event_tx.send(SessionEvent::SessionExited {
                session_id: session_id.to_string(),
                exit_code: 0,
            });
        }
        if let Some(handle) = self.pty_reader_handles.remove(session_id) {
            drop(handle);
        }
        Ok(())
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    #[allow(dead_code)]
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    pub fn render_receiver(&self) -> Receiver<RenderRequest> {
        self.render_rx.clone()
    }

    #[allow(dead_code)]
    pub fn event_sender(&self) -> Sender<SessionEvent> {
        self.event_tx.clone()
    }

    pub fn screen_store(&self) -> ScreenStore {
        self.screen_store.clone()
    }

    pub fn terminate_all(&mut self) {
        let session_ids: Vec<String> = self.sessions.keys().cloned().collect();
        for session_id in session_ids {
            let _ = self.terminate(&session_id);
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        self.terminate_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn session_manager_spawns_and_terminates() {
        let mut manager = SessionManager::new();

        manager
            .spawn_session("sess-1", "/bin/sh", &["-c", "echo hello"])
            .expect("session should spawn");

        assert!(manager.has_session("sess-1"));

        let render_rx = manager.render_receiver();
        let render = render_rx.recv_timeout(Duration::from_millis(500));
        assert!(render.is_ok());

        manager
            .terminate("sess-1")
            .expect("terminate should succeed");
        assert!(!manager.has_session("sess-1"));
    }

    #[test]
    fn session_manager_sends_input() {
        let mut manager = SessionManager::new();

        manager
            .spawn_session("sess-1", "/bin/sh", &["-c", "cat"])
            .expect("session should spawn");

        manager
            .send_input("sess-1", "test input\n")
            .expect("input should be sent");

        let render_rx = manager.render_receiver();
        let mut found = false;
        for _ in 0..10 {
            if let Ok(render) = render_rx.recv_timeout(Duration::from_millis(100))
                && render.session_id == "sess-1"
            {
                let screen_store = manager.screen_store();
                let store = screen_store.lock().unwrap();
                if let Some(parser) = store.get("sess-1") {
                    let content: Vec<String> = parser
                        .screen()
                        .rows(0, 80)
                        .map(|l| l.trim_end().to_string())
                        .collect();
                    if content.iter().any(|line| line.contains("test input")) {
                        found = true;
                        break;
                    }
                }
            }
        }

        assert!(found, "expected to find 'test input' in screen output");

        manager
            .terminate("sess-1")
            .expect("terminate should succeed");
    }

    #[test]
    fn session_manager_handles_resize() {
        let mut manager = SessionManager::new();

        manager
            .spawn_session("sess-1", "/bin/sh", &["-c", "cat"])
            .expect("session should spawn");

        std::thread::sleep(Duration::from_millis(50));

        manager
            .resize("sess-1", 200, 50)
            .expect("resize should succeed");

        std::thread::sleep(Duration::from_millis(50));

        let screen_store = manager.screen_store();
        let store = screen_store.lock().unwrap();
        let parser = store.get("sess-1").expect("screen should exist");
        assert_eq!(parser.screen().size().0, 50);
        assert_eq!(parser.screen().size().1, 200);

        manager
            .terminate("sess-1")
            .expect("terminate should succeed");
    }
}
