use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use orchestrator_core::SessionState;
use rpc_core::{ApiService, RpcRequest, RpcResponse, SessionRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

pub const CRATE_NAME: &str = "transport-stdio";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DaemonEvent {
    SessionCreated(String),
    SessionUpdated {
        session_id: String,
        status: SessionState,
    },
}

#[derive(Debug, Error)]
pub enum StdioTransportError {
    #[error("request parse failed: {0}")]
    RequestParse(String),
    #[error("request handling failed: {0}")]
    RequestHandling(String),
    #[error("daemon state poisoned")]
    Poisoned,
}

#[derive(Clone)]
pub struct StdioDaemon {
    api: Arc<Mutex<ApiService>>,
    subscribers: Arc<Mutex<Vec<Sender<DaemonEvent>>>>,
    session_locks: Arc<Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>>,
}

impl StdioDaemon {
    pub fn new(api: ApiService) -> Self {
        Self {
            api: Arc::new(Mutex::new(api)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            session_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn attach_client(&self) -> StdioClient {
        StdioClient {
            daemon: self.clone(),
        }
    }

    pub fn subscribe(&self) -> Result<Receiver<DaemonEvent>, StdioTransportError> {
        let (tx, rx) = mpsc::channel();
        let mut subs = self
            .subscribers
            .lock()
            .map_err(|_| StdioTransportError::Poisoned)?;
        subs.push(tx);
        Ok(rx)
    }

    fn broadcast(&self, event: DaemonEvent) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|sender| sender.send(event.clone()).is_ok());
        }
    }

    pub fn handle_json_line(&self, json_line: &str) -> Result<String, StdioTransportError> {
        let request: RpcRequest = serde_json::from_str(json_line)
            .map_err(|err| StdioTransportError::RequestParse(err.to_string()))?;

        let mut api = self.api.lock().map_err(|_| StdioTransportError::Poisoned)?;

        let response = match request.method.as_str() {
            "system.health" => RpcResponse {
                id: request.id,
                ok: true,
                result_json: json!({ "status": api.system_health() }).to_string(),
            },
            "session.create" => {
                let session = api.session_create();
                self.broadcast(DaemonEvent::SessionCreated(session.id.clone()));
                RpcResponse {
                    id: request.id,
                    ok: true,
                    result_json: serde_json::to_string(&session)
                        .map_err(|err| StdioTransportError::RequestHandling(err.to_string()))?,
                }
            }
            "session.list" => RpcResponse {
                id: request.id,
                ok: true,
                result_json: serde_json::to_string(&api.session_list())
                    .map_err(|err| StdioTransportError::RequestHandling(err.to_string()))?,
            },
            _ => RpcResponse {
                id: request.id,
                ok: false,
                result_json: json!({"error": "unsupported method"}).to_string(),
            },
        };

        serde_json::to_string(&response)
            .map_err(|err| StdioTransportError::RequestHandling(err.to_string()))
    }
}

#[derive(Clone)]
pub struct StdioClient {
    daemon: StdioDaemon,
}

impl StdioClient {
    pub fn session_create(&self) -> Result<SessionRecord, StdioTransportError> {
        let mut api = self
            .daemon
            .api
            .lock()
            .map_err(|_| StdioTransportError::Poisoned)?;
        let session = api.session_create();
        self.daemon
            .broadcast(DaemonEvent::SessionCreated(session.id.clone()));
        Ok(session)
    }

    pub fn session_list(&self) -> Result<Vec<SessionRecord>, StdioTransportError> {
        let api = self
            .daemon
            .api
            .lock()
            .map_err(|_| StdioTransportError::Poisoned)?;
        Ok(api.session_list())
    }

    pub fn session_set_status(
        &self,
        session_id: &str,
        status: SessionState,
    ) -> Result<SessionRecord, StdioTransportError> {
        self.session_set_status_with_version(session_id, status, None)
    }

    pub fn session_set_status_with_version(
        &self,
        session_id: &str,
        status: SessionState,
        expected_version: Option<u64>,
    ) -> Result<SessionRecord, StdioTransportError> {
        let session_lock = {
            let mut locks = self
                .daemon
                .session_locks
                .lock()
                .map_err(|_| StdioTransportError::Poisoned)?;
            locks
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };

        let _guard = session_lock
            .lock()
            .map_err(|_| StdioTransportError::Poisoned)?;
        let mut api = self
            .daemon
            .api
            .lock()
            .map_err(|_| StdioTransportError::Poisoned)?;
        let current = api
            .session_get(session_id)
            .map_err(|err| StdioTransportError::RequestHandling(err.to_string()))?;
        let version = expected_version.unwrap_or(current.version);
        let updated = api
            .session_set_status_with_version(session_id, status, version)
            .map_err(|err| StdioTransportError::RequestHandling(err.to_string()))?;
        self.daemon.broadcast(DaemonEvent::SessionUpdated {
            session_id: session_id.to_string(),
            status,
        });
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use orchestrator_core::SessionState;
    use serde_json::json;

    use super::{DaemonEvent, StdioDaemon};
    use rpc_core::{ApiService, RpcRequest};

    #[test]
    fn two_clients_share_same_daemon_state() {
        let daemon = StdioDaemon::new(ApiService::new());
        let client_a = daemon.attach_client();
        let client_b = daemon.attach_client();

        let created = client_a.session_create().expect("create should succeed");
        let from_b = client_b.session_list().expect("list should succeed");

        assert_eq!(from_b.len(), 1);
        assert_eq!(from_b[0].id, created.id);
    }

    #[test]
    fn subscription_receives_daemon_events() {
        let daemon = StdioDaemon::new(ApiService::new());
        let client = daemon.attach_client();
        let rx = daemon.subscribe().expect("subscribe should work");

        let created = client.session_create().expect("create should succeed");
        let event = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("subscriber should get event");

        assert_eq!(event, DaemonEvent::SessionCreated(created.id));
    }

    #[test]
    fn json_line_request_flow_works() {
        let daemon = StdioDaemon::new(ApiService::new());

        let req = RpcRequest {
            id: "req-1".to_string(),
            method: "system.health".to_string(),
            params_json: json!({}).to_string(),
        };
        let req_json = serde_json::to_string(&req).expect("request should serialize");
        let response = daemon
            .handle_json_line(&req_json)
            .expect("request should be handled");
        let parsed: rpc_core::RpcResponse =
            serde_json::from_str(&response).expect("response should parse");
        assert!(parsed.ok);

        let result: serde_json::Value =
            serde_json::from_str(&parsed.result_json).expect("result payload should parse");
        assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));
    }

    #[test]
    fn stale_writes_conflict_and_session_updates_are_serialized() {
        let daemon = StdioDaemon::new(ApiService::new());
        let client_a = daemon.attach_client();
        let client_b = daemon.attach_client();

        let created = client_a.session_create().expect("create should succeed");
        let stale_version = created.version;

        let updated = client_a
            .session_set_status_with_version(
                &created.id,
                SessionState::Running,
                Some(stale_version),
            )
            .expect("first update should succeed");
        assert_eq!(updated.version, stale_version + 1);

        let conflict = client_b
            .session_set_status_with_version(&created.id, SessionState::Busy, Some(stale_version))
            .expect_err("stale write should conflict");
        assert!(conflict.to_string().contains("conflict"));

        let session_id = created.id.clone();
        let c1 = client_a.clone();
        let c2 = client_b.clone();

        let t1 =
            thread::spawn(move || c1.session_set_status(&session_id, SessionState::AwaitingInput));
        let session_id = created.id.clone();
        let t2 = thread::spawn(move || c2.session_set_status(&session_id, SessionState::Busy));

        let r1 = t1.join().expect("thread 1 should join");
        let r2 = t2.join().expect("thread 2 should join");
        assert!(r1.is_ok());
        assert!(r2.is_ok());
    }

    #[test]
    fn slo_no_action_loss_under_concurrent_writes() {
        let daemon = StdioDaemon::new(ApiService::new());
        let client_a = daemon.attach_client();
        let client_b = daemon.attach_client();
        let session = client_a
            .session_create()
            .expect("session create should succeed");

        let attempts_per_client = 40_u64;
        let mut success = 0_u64;
        let mut conflicts = 0_u64;

        for idx in 0..attempts_per_client {
            let stale_version = session.version + idx;

            let a = client_a.session_set_status_with_version(
                &session.id,
                SessionState::Running,
                Some(stale_version),
            );
            let b = client_b.session_set_status_with_version(
                &session.id,
                SessionState::Busy,
                Some(stale_version),
            );

            for result in [a, b] {
                match result {
                    Ok(_) => success += 1,
                    Err(err) => {
                        if err.to_string().contains("conflict") {
                            conflicts += 1;
                        } else {
                            panic!("unexpected error during SLO check: {err}");
                        }
                    }
                }
            }
        }

        let submitted = attempts_per_client * 2;
        assert_eq!(submitted, success + conflicts, "action accounting mismatch");

        let final_state = client_a
            .session_list()
            .expect("session list should succeed")
            .into_iter()
            .find(|s| s.id == session.id)
            .expect("session should exist in final list");
        assert_eq!(final_state.version, 1 + success);
    }
}
