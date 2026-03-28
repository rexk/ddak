use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use orchestrator_core::adapters::{
    AdapterCapabilities, AdapterError, AdapterEvent, AdapterProbe, AdapterSessionHandle,
    AgentAdapter,
};
use runtime_pty::PtySession;

pub const CRATE_NAME: &str = "adapter-opencode";

pub struct OpenCodeAdapter {
    sessions: HashMap<String, PtySession>,
    workdir: Option<PathBuf>,
}

impl OpenCodeAdapter {
    pub fn new() -> Self {
        Self::with_workdir(None)
    }

    pub fn with_workdir(workdir: Option<PathBuf>) -> Self {
        Self {
            sessions: HashMap::new(),
            workdir,
        }
    }

    pub fn set_workdir(&mut self, workdir: Option<PathBuf>) {
        self.workdir = workdir;
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
}

impl Default for OpenCodeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentAdapter for OpenCodeAdapter {
    fn probe(&self) -> Result<AdapterProbe, AdapterError> {
        Ok(AdapterProbe {
            name: "opencode".to_string(),
            available: true,
            version: Some("v1".to_string()),
        })
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            supports_interruption: true,
            supports_resume: true,
            supports_structured_tasks: false,
            supports_cost_metrics: false,
            supports_model_switch: false,
        }
    }

    fn start_session(
        &mut self,
        session_id: &str,
        command: &str,
        args: &[&str],
    ) -> Result<AdapterSessionHandle, AdapterError> {
        let session = PtySession::spawn_in_dir(command, args, 80, 24, self.workdir.as_deref())
            .map_err(|err| AdapterError::CommandFailed(err.to_string()))?;
        self.sessions.insert(session_id.to_string(), session);

        Ok(AdapterSessionHandle {
            session_id: session_id.to_string(),
            adapter_session_ref: Some(format!("opencode:{session_id}")),
        })
    }

    fn write_input(&mut self, session_id: &str, input: &str) -> Result<(), AdapterError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| AdapterError::SessionNotFound(session_id.to_string()))?;
        session
            .send_input(input)
            .map_err(|err| AdapterError::Io(err.to_string()))
    }

    fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), AdapterError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| AdapterError::SessionNotFound(session_id.to_string()))?;
        session
            .resize(cols, rows)
            .map_err(|err| AdapterError::Io(err.to_string()))
    }

    fn interrupt(&mut self, session_id: &str) -> Result<(), AdapterError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| AdapterError::SessionNotFound(session_id.to_string()))?;
        session
            .interrupt()
            .map_err(|err| AdapterError::Io(err.to_string()))
    }

    fn terminate(&mut self, session_id: &str) -> Result<(), AdapterError> {
        let mut session = self
            .sessions
            .remove(session_id)
            .ok_or_else(|| AdapterError::SessionNotFound(session_id.to_string()))?;
        session
            .terminate()
            .map_err(|err| AdapterError::Io(err.to_string()))
    }

    fn read_events(&self, session_id: &str) -> Result<Vec<AdapterEvent>, AdapterError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| AdapterError::SessionNotFound(session_id.to_string()))?;
        let output = session
            .read_output(Duration::from_millis(30))
            .map_err(|err| AdapterError::Io(err.to_string()))?;

        if output.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![AdapterEvent {
            session_id: session_id.to_string(),
            event_type: "output.delta".to_string(),
            payload: String::from_utf8_lossy(&output).into_owned(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use orchestrator_core::adapters::AgentAdapter;

    use super::OpenCodeAdapter;

    #[test]
    fn opencode_session_can_be_created_and_controlled() {
        let mut adapter = OpenCodeAdapter::new();
        let handle = adapter
            .start_session("sess-1", "/bin/sh", &["-c", "cat"])
            .expect("session should start");

        assert_eq!(handle.session_id, "sess-1");
        adapter
            .write_input("sess-1", "hello opencode\n")
            .expect("write should succeed");

        let mut events = Vec::new();
        for _ in 0..5 {
            events = adapter
                .read_events("sess-1")
                .expect("events should be readable");
            if events
                .iter()
                .any(|event| event.payload.contains("hello opencode"))
            {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            events
                .iter()
                .any(|event| event.payload.contains("hello opencode")),
            "expected normalized output event"
        );

        adapter
            .resize("sess-1", 100, 40)
            .expect("resize should succeed");
        adapter
            .terminate("sess-1")
            .expect("terminate should succeed");
    }
}
