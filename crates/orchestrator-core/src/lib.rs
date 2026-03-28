use serde::{Deserialize, Serialize};

pub mod adapters;
pub mod config;
pub mod diagnostics;
pub mod events;
pub mod fanout;
pub mod projection;
pub mod resume;
pub mod secrets;
pub mod session_bus;
pub mod session_fsm;

pub use session_bus::{
    RenderRequest, ScreenStore, ScreenThread, SessionEvent, screen_dump_with_attrs,
    wait_for_screen_content,
};

pub const CRATE_NAME: &str = "orchestrator-core";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Created,
    Starting,
    Running,
    AwaitingInput,
    Busy,
    Suspended,
    Completed,
    Failed,
    Terminated,
}
