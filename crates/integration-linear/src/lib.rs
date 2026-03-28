use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashSet};

use duckdb::{Connection, OptionalExt, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CRATE_NAME: &str = "integration-linear";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinearMappingProfile {
    pub profile_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub external_project_id: String,
    pub status_map: BTreeMap<String, String>,
    pub reverse_status_map: BTreeMap<String, String>,
    pub field_map: BTreeMap<String, String>,
    pub sync_policy: String,
    pub mapping_version: i64,
    pub last_validated_at: Option<String>,
}

#[derive(Debug, Error)]
pub enum LinearIntegrationError {
    #[error("database error: {0}")]
    Database(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("profile not found: {0}")]
    NotFound(String),
}

pub struct LinearProfileStore<'a> {
    conn: &'a Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIssue {
    pub external_id: String,
    pub status: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncCursor {
    pub last_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRecord {
    pub external_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncOutcome {
    pub pulled_count: usize,
    pub pushed_count: usize,
    pub quarantined: Vec<QuarantineRecord>,
}

#[derive(Debug, Default)]
pub struct LinearSyncEngine {
    processed_operation_ids: HashSet<String>,
}

impl LinearSyncEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pull(
        &mut self,
        profile: &LinearMappingProfile,
        cursor: &mut SyncCursor,
        external_issues: &[ExternalIssue],
    ) -> SyncOutcome {
        let mut outcome = SyncOutcome::default();
        let known_external_states: BTreeSet<String> =
            profile.reverse_status_map.keys().cloned().collect();

        for issue in external_issues.iter().skip(cursor.last_offset) {
            let op_id = format!("pull:{}:{}", profile.profile_id, issue.external_id);
            if !self.processed_operation_ids.insert(op_id) {
                continue;
            }

            if !known_external_states.contains(&issue.status) {
                outcome.quarantined.push(QuarantineRecord {
                    external_id: issue.external_id.clone(),
                    reason: format!("unknown external status: {}", issue.status),
                });
                continue;
            }

            outcome.pulled_count += 1;
        }

        cursor.last_offset = external_issues.len();
        outcome
    }

    pub fn push(
        &mut self,
        profile: &LinearMappingProfile,
        local_issue_statuses: &BTreeMap<String, String>,
    ) -> SyncOutcome {
        let mut outcome = SyncOutcome::default();
        let known_local_states: BTreeSet<String> = profile.status_map.keys().cloned().collect();

        for (issue_id, status) in local_issue_statuses {
            let op_id = format!("push:{}:{}:{}", profile.profile_id, issue_id, status);
            if !self.processed_operation_ids.insert(op_id) {
                continue;
            }

            if !known_local_states.contains(status) {
                outcome.quarantined.push(QuarantineRecord {
                    external_id: issue_id.clone(),
                    reason: format!("unknown local status: {status}"),
                });
                continue;
            }

            outcome.pushed_count += 1;
        }

        outcome
    }
}

impl<'a> LinearProfileStore<'a> {
    pub fn new(conn: &'a Connection) -> Result<Self, LinearIntegrationError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS linear_mapping_profiles (
              profile_id TEXT PRIMARY KEY,
              workspace_id TEXT NOT NULL,
              project_id TEXT NOT NULL,
              external_project_id TEXT NOT NULL,
              status_map_json TEXT NOT NULL,
              reverse_status_map_json TEXT NOT NULL,
              field_map_json TEXT NOT NULL,
              sync_policy TEXT NOT NULL,
              mapping_version BIGINT NOT NULL,
              last_validated_at TEXT
            );
            "#,
        )
        .map_err(|err| LinearIntegrationError::Database(err.to_string()))?;

        Ok(Self { conn })
    }

    pub fn save_profile(
        &self,
        profile: &LinearMappingProfile,
    ) -> Result<(), LinearIntegrationError> {
        let status_map_json = serde_json::to_string(&profile.status_map)
            .map_err(|err| LinearIntegrationError::Serialization(err.to_string()))?;
        let reverse_status_map_json = serde_json::to_string(&profile.reverse_status_map)
            .map_err(|err| LinearIntegrationError::Serialization(err.to_string()))?;
        let field_map_json = serde_json::to_string(&profile.field_map)
            .map_err(|err| LinearIntegrationError::Serialization(err.to_string()))?;

        self.conn
            .execute(
                r#"
                INSERT INTO linear_mapping_profiles(
                  profile_id,
                  workspace_id,
                  project_id,
                  external_project_id,
                  status_map_json,
                  reverse_status_map_json,
                  field_map_json,
                  sync_policy,
                  mapping_version,
                  last_validated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(profile_id)
                DO UPDATE SET
                  workspace_id = excluded.workspace_id,
                  project_id = excluded.project_id,
                  external_project_id = excluded.external_project_id,
                  status_map_json = excluded.status_map_json,
                  reverse_status_map_json = excluded.reverse_status_map_json,
                  field_map_json = excluded.field_map_json,
                  sync_policy = excluded.sync_policy,
                  mapping_version = excluded.mapping_version,
                  last_validated_at = excluded.last_validated_at
                "#,
                params![
                    profile.profile_id,
                    profile.workspace_id,
                    profile.project_id,
                    profile.external_project_id,
                    status_map_json,
                    reverse_status_map_json,
                    field_map_json,
                    profile.sync_policy,
                    profile.mapping_version,
                    profile.last_validated_at
                ],
            )
            .map_err(|err| LinearIntegrationError::Database(err.to_string()))?;

        Ok(())
    }

    pub fn get_profile(
        &self,
        profile_id: &str,
    ) -> Result<LinearMappingProfile, LinearIntegrationError> {
        self.conn
            .query_row(
                r#"
                SELECT
                  profile_id,
                  workspace_id,
                  project_id,
                  external_project_id,
                  status_map_json,
                  reverse_status_map_json,
                  field_map_json,
                  sync_policy,
                  mapping_version,
                  last_validated_at
                FROM linear_mapping_profiles
                WHERE profile_id = ?
                "#,
                params![profile_id],
                |row| {
                    let status_map_json: String = row.get(4)?;
                    let reverse_status_map_json: String = row.get(5)?;
                    let field_map_json: String = row.get(6)?;

                    let status_map = serde_json::from_str(&status_map_json)
                        .map_err(|_| duckdb::Error::InvalidQuery)?;
                    let reverse_status_map = serde_json::from_str(&reverse_status_map_json)
                        .map_err(|_| duckdb::Error::InvalidQuery)?;
                    let field_map = serde_json::from_str(&field_map_json)
                        .map_err(|_| duckdb::Error::InvalidQuery)?;

                    Ok(LinearMappingProfile {
                        profile_id: row.get(0)?,
                        workspace_id: row.get(1)?,
                        project_id: row.get(2)?,
                        external_project_id: row.get(3)?,
                        status_map,
                        reverse_status_map,
                        field_map,
                        sync_policy: row.get(7)?,
                        mapping_version: row.get(8)?,
                        last_validated_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|err| LinearIntegrationError::Database(err.to_string()))?
            .ok_or_else(|| LinearIntegrationError::NotFound(profile_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use duckdb::Connection;

    use super::{
        ExternalIssue, LinearMappingProfile, LinearProfileStore, LinearSyncEngine, SyncCursor,
    };

    #[test]
    fn saves_and_loads_mapping_profile() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        let store = LinearProfileStore::new(&conn).expect("store should initialize");

        let mut status_map = BTreeMap::new();
        status_map.insert("in_progress".to_string(), "In Progress".to_string());
        status_map.insert("done".to_string(), "Done".to_string());

        let mut reverse = BTreeMap::new();
        reverse.insert("In Progress".to_string(), "in_progress".to_string());
        reverse.insert("Done".to_string(), "done".to_string());

        let mut fields = BTreeMap::new();
        fields.insert("title".to_string(), "title".to_string());
        fields.insert("description".to_string(), "description".to_string());

        let profile = LinearMappingProfile {
            profile_id: "profile-1".to_string(),
            workspace_id: "workspace-1".to_string(),
            project_id: "project-1".to_string(),
            external_project_id: "linear-project-1".to_string(),
            status_map,
            reverse_status_map: reverse,
            field_map: fields,
            sync_policy: "bidirectional".to_string(),
            mapping_version: 2,
            last_validated_at: Some("2026-02-11T00:00:00Z".to_string()),
        };

        store.save_profile(&profile).expect("profile should save");
        let loaded = store
            .get_profile("profile-1")
            .expect("profile should load back");

        assert_eq!(loaded, profile);
    }

    #[test]
    fn unknown_external_status_enters_quarantine() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        let store = LinearProfileStore::new(&conn).expect("store should initialize");

        let mut status_map = BTreeMap::new();
        status_map.insert("in_progress".to_string(), "In Progress".to_string());
        let mut reverse = BTreeMap::new();
        reverse.insert("In Progress".to_string(), "in_progress".to_string());

        let profile = LinearMappingProfile {
            profile_id: "profile-sync".to_string(),
            workspace_id: "workspace-1".to_string(),
            project_id: "project-1".to_string(),
            external_project_id: "linear-project-1".to_string(),
            status_map,
            reverse_status_map: reverse,
            field_map: BTreeMap::new(),
            sync_policy: "bidirectional".to_string(),
            mapping_version: 1,
            last_validated_at: None,
        };
        store.save_profile(&profile).expect("profile should save");

        let mut engine = LinearSyncEngine::new();
        let mut cursor = SyncCursor::default();
        let external = vec![ExternalIssue {
            external_id: "LIN-1".to_string(),
            status: "Unknown State".to_string(),
            title: "Drifted issue".to_string(),
        }];

        let outcome = engine.pull(&profile, &mut cursor, &external);
        assert_eq!(outcome.pulled_count, 0);
        assert_eq!(outcome.quarantined.len(), 1);
        assert!(
            outcome.quarantined[0]
                .reason
                .contains("unknown external status")
        );
    }

    #[test]
    fn sync_retries_are_idempotent_by_operation_id() {
        let mut status_map = BTreeMap::new();
        status_map.insert("in_progress".to_string(), "In Progress".to_string());
        let mut reverse = BTreeMap::new();
        reverse.insert("In Progress".to_string(), "in_progress".to_string());

        let profile = LinearMappingProfile {
            profile_id: "profile-sync".to_string(),
            workspace_id: "workspace-1".to_string(),
            project_id: "project-1".to_string(),
            external_project_id: "linear-project-1".to_string(),
            status_map,
            reverse_status_map: reverse,
            field_map: BTreeMap::new(),
            sync_policy: "bidirectional".to_string(),
            mapping_version: 1,
            last_validated_at: None,
        };

        let mut engine = LinearSyncEngine::new();
        let mut local = BTreeMap::new();
        local.insert("issue-1".to_string(), "in_progress".to_string());

        let first = engine.push(&profile, &local);
        let second = engine.push(&profile, &local);

        assert_eq!(first.pushed_count, 1);
        assert_eq!(second.pushed_count, 0);
    }
}
