use std::collections::BTreeMap;

use crate::SessionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionEvent {
    SessionTransition {
        session_id: String,
        state: SessionState,
    },
    IssueStatusChanged {
        issue_id: String,
        status: String,
    },
    IssuePrimaryLinked {
        issue_id: String,
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionSnapshot {
    pub session_states: BTreeMap<String, SessionState>,
    pub issue_statuses: BTreeMap<String, String>,
    pub issue_primary_sessions: BTreeMap<String, String>,
}

impl ProjectionSnapshot {
    pub fn checksum(&self) -> String {
        let mut canonical = String::new();

        for (session_id, state) in &self.session_states {
            canonical.push_str(&format!("session:{session_id}:{state:?}\n"));
        }

        for (issue_id, status) in &self.issue_statuses {
            canonical.push_str(&format!("issue_status:{issue_id}:{status}\n"));
        }

        for (issue_id, session_id) in &self.issue_primary_sessions {
            canonical.push_str(&format!("issue_primary:{issue_id}:{session_id}\n"));
        }

        format!("{:016x}", fnv1a_64(canonical.as_bytes()))
    }
}

#[derive(Debug, Default)]
pub struct ProjectionEngine;

impl ProjectionEngine {
    pub fn rebuild(events: &[ProjectionEvent]) -> ProjectionSnapshot {
        let mut snapshot = ProjectionSnapshot::default();

        for event in events {
            match event {
                ProjectionEvent::SessionTransition { session_id, state } => {
                    snapshot.session_states.insert(session_id.clone(), *state);
                }
                ProjectionEvent::IssueStatusChanged { issue_id, status } => {
                    snapshot
                        .issue_statuses
                        .insert(issue_id.clone(), status.clone());
                }
                ProjectionEvent::IssuePrimaryLinked {
                    issue_id,
                    session_id,
                } => {
                    snapshot
                        .issue_primary_sessions
                        .insert(issue_id.clone(), session_id.clone());
                }
            }
        }

        snapshot
    }
}

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use crate::SessionState;

    use super::{ProjectionEngine, ProjectionEvent};

    #[test]
    fn rebuilding_same_log_yields_same_checksum() {
        let log = vec![
            ProjectionEvent::SessionTransition {
                session_id: "sess-1".to_string(),
                state: SessionState::Running,
            },
            ProjectionEvent::IssueStatusChanged {
                issue_id: "issue-1".to_string(),
                status: "in_progress".to_string(),
            },
            ProjectionEvent::IssuePrimaryLinked {
                issue_id: "issue-1".to_string(),
                session_id: "sess-1".to_string(),
            },
        ];

        let first = ProjectionEngine::rebuild(&log);
        let second = ProjectionEngine::rebuild(&log);

        assert_eq!(first.checksum(), second.checksum());
        assert_eq!(first, second);
    }

    #[test]
    fn checksum_changes_when_projection_changes() {
        let log_a = vec![ProjectionEvent::IssueStatusChanged {
            issue_id: "issue-1".to_string(),
            status: "in_progress".to_string(),
        }];
        let log_b = vec![ProjectionEvent::IssueStatusChanged {
            issue_id: "issue-1".to_string(),
            status: "done".to_string(),
        }];

        let a = ProjectionEngine::rebuild(&log_a);
        let b = ProjectionEngine::rebuild(&log_b);

        assert_ne!(a.checksum(), b.checksum());
    }
}
