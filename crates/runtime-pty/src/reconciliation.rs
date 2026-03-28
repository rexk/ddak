use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistedSessionState {
    Starting,
    Running,
    AwaitingInput,
    Busy,
    Suspended,
    Completed,
    Failed,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedSessionRecord {
    pub session_id: String,
    pub state: PersistedSessionState,
    pub runtime_pid: Option<u32>,
    pub has_resume_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    ReattachedByPid,
    ReattachedByHint,
    TerminatedOrphaned { reason_code: &'static str },
    NoAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationDecision {
    pub session_id: String,
    pub outcome: ReconciliationOutcome,
}

pub struct StartupReconciler;

impl StartupReconciler {
    pub fn reconcile(
        persisted: &[PersistedSessionRecord],
        live_pids: &[u32],
    ) -> Vec<ReconciliationDecision> {
        let live: HashSet<u32> = live_pids.iter().copied().collect();

        persisted
            .iter()
            .map(|record| {
                let outcome = if !is_running_state(record.state) {
                    ReconciliationOutcome::NoAction
                } else if let Some(pid) = record.runtime_pid {
                    if live.contains(&pid) {
                        ReconciliationOutcome::ReattachedByPid
                    } else if record.has_resume_hint {
                        ReconciliationOutcome::ReattachedByHint
                    } else {
                        ReconciliationOutcome::TerminatedOrphaned {
                            reason_code: "missing_runtime_pid",
                        }
                    }
                } else if record.has_resume_hint {
                    ReconciliationOutcome::ReattachedByHint
                } else {
                    ReconciliationOutcome::TerminatedOrphaned {
                        reason_code: "no_pid_or_resume_hint",
                    }
                };

                ReconciliationDecision {
                    session_id: record.session_id.clone(),
                    outcome,
                }
            })
            .collect()
    }
}

fn is_running_state(state: PersistedSessionState) -> bool {
    matches!(
        state,
        PersistedSessionState::Starting
            | PersistedSessionState::Running
            | PersistedSessionState::AwaitingInput
            | PersistedSessionState::Busy
            | PersistedSessionState::Suspended
    )
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{
        PersistedSessionRecord, PersistedSessionState, ReconciliationOutcome, StartupReconciler,
    };

    #[test]
    fn running_session_with_live_pid_reattaches() {
        let records = vec![PersistedSessionRecord {
            session_id: "sess-1".to_string(),
            state: PersistedSessionState::Running,
            runtime_pid: Some(101),
            has_resume_hint: false,
        }];

        let decisions = StartupReconciler::reconcile(&records, &[101]);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].session_id, "sess-1");
        assert_eq!(decisions[0].outcome, ReconciliationOutcome::ReattachedByPid);
    }

    #[test]
    fn missing_pid_with_resume_hint_reattaches_by_hint() {
        let records = vec![PersistedSessionRecord {
            session_id: "sess-2".to_string(),
            state: PersistedSessionState::Busy,
            runtime_pid: Some(202),
            has_resume_hint: true,
        }];

        let decisions = StartupReconciler::reconcile(&records, &[999]);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].session_id, "sess-2");
        assert_eq!(
            decisions[0].outcome,
            ReconciliationOutcome::ReattachedByHint
        );
    }

    #[test]
    fn missing_pid_without_hint_marks_terminated_orphaned() {
        let records = vec![PersistedSessionRecord {
            session_id: "sess-3".to_string(),
            state: PersistedSessionState::AwaitingInput,
            runtime_pid: Some(303),
            has_resume_hint: false,
        }];

        let decisions = StartupReconciler::reconcile(&records, &[404]);
        assert_eq!(decisions.len(), 1);
        assert_eq!(
            decisions[0].outcome,
            ReconciliationOutcome::TerminatedOrphaned {
                reason_code: "missing_runtime_pid"
            }
        );
    }

    #[test]
    fn non_running_states_require_no_action() {
        let records = vec![PersistedSessionRecord {
            session_id: "sess-4".to_string(),
            state: PersistedSessionState::Completed,
            runtime_pid: None,
            has_resume_hint: false,
        }];

        let decisions = StartupReconciler::reconcile(&records, &[]);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].outcome, ReconciliationOutcome::NoAction);
    }

    #[test]
    fn slo_restart_recovery_time_under_budget() {
        let mut records = Vec::new();
        for i in 0..500 {
            records.push(PersistedSessionRecord {
                session_id: format!("sess-{i}"),
                state: PersistedSessionState::Running,
                runtime_pid: Some(i as u32),
                has_resume_hint: i % 2 == 0,
            });
        }
        let live_pids: Vec<u32> = (0..500).step_by(2).map(|i| i as u32).collect();

        let start = Instant::now();
        let decisions = StartupReconciler::reconcile(&records, &live_pids);
        let elapsed_ms = start.elapsed().as_millis() as u64;

        assert_eq!(decisions.len(), records.len());
        assert!(
            elapsed_ms <= 250,
            "restart reconciliation exceeded budget: {elapsed_ms}ms"
        );
    }
}
