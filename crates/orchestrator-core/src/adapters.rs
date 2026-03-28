use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterProbe {
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterCapabilities {
    pub supports_interruption: bool,
    pub supports_resume: bool,
    pub supports_structured_tasks: bool,
    pub supports_cost_metrics: bool,
    pub supports_model_switch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterSessionHandle {
    pub session_id: String,
    pub adapter_session_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterEvent {
    pub session_id: String,
    pub event_type: String,
    pub payload: String,
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("adapter command failed: {0}")]
    CommandFailed(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("adapter I/O error: {0}")]
    Io(String),
}

pub trait AgentAdapter {
    fn probe(&self) -> Result<AdapterProbe, AdapterError>;
    fn capabilities(&self) -> AdapterCapabilities;

    fn start_session(
        &mut self,
        session_id: &str,
        command: &str,
        args: &[&str],
    ) -> Result<AdapterSessionHandle, AdapterError>;

    fn write_input(&mut self, session_id: &str, input: &str) -> Result<(), AdapterError>;
    fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), AdapterError>;
    fn interrupt(&mut self, session_id: &str) -> Result<(), AdapterError>;
    fn terminate(&mut self, session_id: &str) -> Result<(), AdapterError>;
    fn read_events(&self, session_id: &str) -> Result<Vec<AdapterEvent>, AdapterError>;
}
