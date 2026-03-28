use std::collections::HashMap;

use thiserror::Error;

use crate::SessionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTransitionEvent {
    pub session_id: String,
    pub seq: u64,
    pub from: SessionState,
    pub to: SessionState,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionTransitionError {
    #[error("invalid transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },
}

#[derive(Debug, Default)]
pub struct SessionLifecycle {
    current_state: HashMap<String, SessionState>,
    current_seq: HashMap<String, u64>,
    events: Vec<SessionTransitionEvent>,
}

impl SessionLifecycle {
    pub fn transition(
        &mut self,
        session_id: &str,
        to: SessionState,
    ) -> Result<SessionTransitionEvent, SessionTransitionError> {
        let from = self
            .current_state
            .get(session_id)
            .copied()
            .unwrap_or(SessionState::Created);

        if !is_transition_allowed(from, to) {
            return Err(SessionTransitionError::InvalidTransition { from, to });
        }

        let seq = self.current_seq.get(session_id).copied().unwrap_or(0) + 1;
        let event = SessionTransitionEvent {
            session_id: session_id.to_string(),
            seq,
            from,
            to,
        };

        self.current_state.insert(session_id.to_string(), to);
        self.current_seq.insert(session_id.to_string(), seq);
        self.events.push(event.clone());
        Ok(event)
    }

    pub fn state_of(&self, session_id: &str) -> SessionState {
        self.current_state
            .get(session_id)
            .copied()
            .unwrap_or(SessionState::Created)
    }

    pub fn events(&self) -> &[SessionTransitionEvent] {
        &self.events
    }

    pub fn replay(events: &[SessionTransitionEvent]) -> Result<Self, SessionTransitionError> {
        let mut lifecycle = Self::default();

        for event in events {
            let from = lifecycle.state_of(&event.session_id);
            if !is_transition_allowed(from, event.to) {
                return Err(SessionTransitionError::InvalidTransition { from, to: event.to });
            }

            lifecycle
                .current_state
                .insert(event.session_id.clone(), event.to);
            lifecycle
                .current_seq
                .insert(event.session_id.clone(), event.seq);
            lifecycle.events.push(event.clone());
        }

        Ok(lifecycle)
    }
}

fn is_transition_allowed(from: SessionState, to: SessionState) -> bool {
    use SessionState::{
        AwaitingInput, Busy, Completed, Created, Failed, Running, Starting, Suspended, Terminated,
    };

    match from {
        Created => matches!(to, Starting | Terminated),
        Starting => matches!(to, Running | Failed | Terminated),
        Running => matches!(
            to,
            AwaitingInput | Busy | Suspended | Completed | Failed | Terminated
        ),
        AwaitingInput => matches!(to, Busy | Suspended | Completed | Failed | Terminated),
        Busy => matches!(
            to,
            AwaitingInput | Suspended | Completed | Failed | Terminated
        ),
        Suspended => matches!(to, Running | Failed | Terminated),
        Failed => matches!(to, Starting | Terminated),
        Completed => false,
        Terminated => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionLifecycle, SessionState, SessionTransitionError};

    #[test]
    fn valid_transitions_are_persisted() {
        let mut lifecycle = SessionLifecycle::default();

        lifecycle
            .transition("sess-1", SessionState::Starting)
            .expect("created -> starting should be valid");
        lifecycle
            .transition("sess-1", SessionState::Running)
            .expect("starting -> running should be valid");
        lifecycle
            .transition("sess-1", SessionState::Completed)
            .expect("running -> completed should be valid");

        assert_eq!(lifecycle.state_of("sess-1"), SessionState::Completed);
        assert_eq!(lifecycle.events().len(), 3);
    }

    #[test]
    fn invalid_transition_is_rejected_with_reason() {
        let mut lifecycle = SessionLifecycle::default();
        lifecycle
            .transition("sess-1", SessionState::Starting)
            .expect("created -> starting should be valid");

        let err = lifecycle
            .transition("sess-1", SessionState::Completed)
            .expect_err("starting -> completed should be invalid");

        assert_eq!(
            err,
            SessionTransitionError::InvalidTransition {
                from: SessionState::Starting,
                to: SessionState::Completed,
            }
        );
    }

    #[test]
    fn replay_reconstructs_current_state_from_events() {
        let mut lifecycle = SessionLifecycle::default();
        lifecycle
            .transition("sess-1", SessionState::Starting)
            .expect("created -> starting should be valid");
        lifecycle
            .transition("sess-1", SessionState::Running)
            .expect("starting -> running should be valid");
        lifecycle
            .transition("sess-1", SessionState::AwaitingInput)
            .expect("running -> awaiting_input should be valid");

        let replayed = SessionLifecycle::replay(lifecycle.events())
            .expect("replay should reconstruct state from persisted events");

        assert_eq!(
            replayed.state_of("sess-1"),
            SessionState::AwaitingInput,
            "replayed lifecycle should restore final session state"
        );
    }
}
