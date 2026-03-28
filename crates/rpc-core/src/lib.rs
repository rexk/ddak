use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use orchestrator_core::SessionState;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const CRATE_NAME: &str = "rpc-core";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcRequest {
    pub id: String,
    pub method: String,
    pub params_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcResponse {
    pub id: String,
    pub ok: bool,
    pub result_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub status: SessionState,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueRecord {
    pub id: String,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub issue_number: Option<u64>,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub cwd_override_path: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repo_local_path: Option<String>,
    #[serde(default)]
    pub repo_remote_url: Option<String>,
    #[serde(default)]
    pub repo_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommentEntityType {
    Issue,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommentRecord {
    pub id: String,
    pub entity_type: CommentEntityType,
    pub entity_id: String,
    pub body_markdown: String,
    pub author: String,
    pub created_at_epoch_ms: u64,
    pub updated_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommentListOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommentListPage {
    pub items: Vec<CommentRecord>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationProfileRecord {
    pub id: String,
    pub integration_type: String,
    pub project_id: String,
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("conflict on {resource}: expected version {expected}, got {actual}")]
    Conflict {
        resource: String,
        expected: u64,
        actual: u64,
    },
    #[error("I/O error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ApiSnapshot {
    pub sessions: HashMap<String, SessionRecord>,
    pub issues: HashMap<String, IssueRecord>,
    pub projects: HashMap<String, ProjectRecord>,
    pub integrations: HashMap<String, IntegrationProfileRecord>,
    pub issue_primary_sessions: HashMap<String, String>,
    #[serde(default)]
    pub comments: HashMap<String, CommentRecord>,
}

#[derive(Debug, Default)]
pub struct ApiService {
    sessions: HashMap<String, SessionRecord>,
    issues: HashMap<String, IssueRecord>,
    projects: HashMap<String, ProjectRecord>,
    integrations: HashMap<String, IntegrationProfileRecord>,
    issue_primary_sessions: HashMap<String, String>,
    comments: HashMap<String, CommentRecord>,
}

impl ApiService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn system_health(&self) -> &'static str {
        "ok"
    }

    pub fn system_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn system_capabilities(&self) -> Vec<&'static str> {
        vec![
            "session",
            "issue",
            "board",
            "project",
            "comment",
            "integration",
            "system",
        ]
    }

    pub fn session_create(&mut self) -> SessionRecord {
        let id = Uuid::now_v7().to_string();
        let session = SessionRecord {
            id: id.clone(),
            status: SessionState::Created,
            version: 1,
        };
        self.sessions.insert(id.clone(), session.clone());
        session
    }

    pub fn session_list(&self) -> Vec<SessionRecord> {
        let mut sessions: Vec<_> = self.sessions.values().cloned().collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        sessions
    }

    pub fn session_get(&self, session_id: &str) -> Result<SessionRecord, ApiError> {
        self.sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("session:{session_id}")))
    }

    pub fn session_set_status(
        &mut self,
        session_id: &str,
        status: SessionState,
    ) -> Result<SessionRecord, ApiError> {
        let current = self
            .sessions
            .get(session_id)
            .ok_or_else(|| ApiError::NotFound(format!("session:{session_id}")))?
            .version;
        self.session_set_status_with_version(session_id, status, current)
    }

    pub fn session_set_status_with_version(
        &mut self,
        session_id: &str,
        status: SessionState,
        expected_version: u64,
    ) -> Result<SessionRecord, ApiError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| ApiError::NotFound(format!("session:{session_id}")))?;

        if session.version != expected_version {
            return Err(ApiError::Conflict {
                resource: format!("session:{session_id}"),
                expected: expected_version,
                actual: session.version,
            });
        }

        session.status = status;
        session.version += 1;
        Ok(session.clone())
    }

    pub fn issue_create(&mut self, title: &str) -> IssueRecord {
        let id = Uuid::now_v7().to_string();
        let issue = IssueRecord {
            id: id.clone(),
            identifier: None,
            issue_number: None,
            title: title.to_string(),
            status: "backlog".to_string(),
            project_id: None,
            cwd_override_path: None,
            version: 1,
        };
        self.issues.insert(id.clone(), issue.clone());
        issue
    }

    pub fn issue_update_title(
        &mut self,
        issue_id: &str,
        title: &str,
    ) -> Result<IssueRecord, ApiError> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(ApiError::BadRequest(
                "issue title cannot be empty".to_string(),
            ));
        }

        let issue = self
            .issues
            .get_mut(issue_id)
            .ok_or_else(|| ApiError::NotFound(format!("issue:{issue_id}")))?;
        issue.title = trimmed.to_string();
        issue.version += 1;
        Ok(issue.clone())
    }

    pub fn issue_list(&self) -> Vec<IssueRecord> {
        let mut issues: Vec<_> = self.issues.values().cloned().collect();
        issues.sort_by(|a, b| {
            a.identifier
                .cmp(&b.identifier)
                .then_with(|| a.id.cmp(&b.id))
        });
        issues
    }

    pub fn issue_update_status(
        &mut self,
        issue_id: &str,
        status: &str,
    ) -> Result<IssueRecord, ApiError> {
        let issue = self
            .issues
            .get_mut(issue_id)
            .ok_or_else(|| ApiError::NotFound(format!("issue:{issue_id}")))?;
        issue.status = status.to_string();
        issue.version += 1;
        Ok(issue.clone())
    }

    pub fn issue_get(&self, issue_id: &str) -> Result<IssueRecord, ApiError> {
        self.issues
            .get(issue_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("issue:{issue_id}")))
    }

    pub fn issue_assign_project(
        &mut self,
        issue_id: &str,
        project_id: &str,
    ) -> Result<IssueRecord, ApiError> {
        let project_key = self
            .projects
            .get(project_id)
            .ok_or_else(|| ApiError::NotFound(format!("project:{project_id}")))?
            .identifier
            .clone();
        if project_key.is_empty() {
            return Err(ApiError::NotFound(format!("project:{project_id}")));
        }
        let next_number = self.next_issue_number(project_id);
        let issue = self
            .issues
            .get_mut(issue_id)
            .ok_or_else(|| ApiError::NotFound(format!("issue:{issue_id}")))?;
        let project_changed = issue.project_id.as_deref() != Some(project_id);
        issue.project_id = Some(project_id.to_string());
        if project_changed || issue.issue_number.is_none() {
            issue.issue_number = Some(next_number);
        }
        issue.identifier = issue
            .issue_number
            .map(|issue_number| format!("{project_key}-{issue_number:04}"));
        issue.version += 1;
        Ok(issue.clone())
    }

    pub fn issue_set_cwd_override(
        &mut self,
        issue_id: &str,
        cwd_override_path: Option<String>,
    ) -> Result<IssueRecord, ApiError> {
        let issue = self
            .issues
            .get_mut(issue_id)
            .ok_or_else(|| ApiError::NotFound(format!("issue:{issue_id}")))?;
        issue.cwd_override_path = cwd_override_path.map(|value| value.trim().to_string());
        issue.version += 1;
        Ok(issue.clone())
    }

    pub fn issue_delete(&mut self, issue_id: &str) -> Result<(), ApiError> {
        if self.issues.remove(issue_id).is_none() {
            return Err(ApiError::NotFound(format!("issue:{issue_id}")));
        }
        self.issue_primary_sessions.remove(issue_id);
        self.comments.retain(|_, comment| {
            !(comment.entity_type == CommentEntityType::Issue && comment.entity_id == issue_id)
        });
        Ok(())
    }

    pub fn comment_add(
        &mut self,
        entity_type: CommentEntityType,
        entity_ref: &str,
        body_markdown: &str,
        author: &str,
    ) -> Result<CommentRecord, ApiError> {
        let entity_id = self.resolve_comment_entity_id(entity_type.clone(), entity_ref)?;
        let trimmed_body = body_markdown.trim();
        if trimmed_body.is_empty() {
            return Err(ApiError::BadRequest(
                "comment body cannot be empty".to_string(),
            ));
        }

        let now_ms = current_epoch_ms();
        let id = Uuid::now_v7().to_string();
        let comment = CommentRecord {
            id: id.clone(),
            entity_type,
            entity_id,
            body_markdown: trimmed_body.to_string(),
            author: normalize_author(author),
            created_at_epoch_ms: now_ms,
            updated_at_epoch_ms: now_ms,
        };
        self.comments.insert(id, comment.clone());
        Ok(comment)
    }

    pub fn comment_list(
        &self,
        entity_type: CommentEntityType,
        entity_ref: &str,
        order: CommentListOrder,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<CommentListPage, ApiError> {
        let entity_id = self.resolve_comment_entity_id(entity_type.clone(), entity_ref)?;
        let mut comments: Vec<_> = self
            .comments
            .values()
            .filter(|comment| comment.entity_type == entity_type && comment.entity_id == entity_id)
            .cloned()
            .collect();
        comments.sort_by(|a, b| a.id.cmp(&b.id));
        if matches!(order, CommentListOrder::Desc) {
            comments.reverse();
        }

        let start = if let Some(cursor) = cursor {
            comments
                .iter()
                .position(|comment| comment.id == cursor)
                .map(|idx| idx + 1)
                .ok_or_else(|| ApiError::BadRequest(format!("invalid comment cursor: {cursor}")))?
        } else {
            0
        };

        if start >= comments.len() {
            return Ok(CommentListPage {
                items: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        }

        let end = start.saturating_add(limit.max(1)).min(comments.len());
        let items = comments[start..end].to_vec();
        let has_more = end < comments.len();
        let next_cursor = if has_more {
            items.last().map(|comment| comment.id.clone())
        } else {
            None
        };

        Ok(CommentListPage {
            items,
            next_cursor,
            has_more,
        })
    }

    pub fn comment_count_for(&self, entity_type: CommentEntityType, entity_id: &str) -> usize {
        self.comments
            .values()
            .filter(|comment| comment.entity_type == entity_type && comment.entity_id == entity_id)
            .count()
    }

    pub fn board_issue_move(
        &mut self,
        issue_id: &str,
        status: &str,
    ) -> Result<IssueRecord, ApiError> {
        self.issue_update_status(issue_id, status)
    }

    pub fn issue_link_primary_session(
        &mut self,
        issue_id: &str,
        session_id: &str,
    ) -> Result<(), ApiError> {
        if !self.issues.contains_key(issue_id) {
            return Err(ApiError::NotFound(format!("issue:{issue_id}")));
        }
        if !self.sessions.contains_key(session_id) {
            return Err(ApiError::NotFound(format!("session:{session_id}")));
        }
        self.issue_primary_sessions
            .insert(issue_id.to_string(), session_id.to_string());
        Ok(())
    }

    pub fn issue_primary_session(&self, issue_id: &str) -> Option<&str> {
        self.issue_primary_sessions
            .get(issue_id)
            .map(std::string::String::as_str)
    }

    pub fn issue_unlink_primary_session(&mut self, issue_id: &str) -> Result<(), ApiError> {
        if !self.issues.contains_key(issue_id) {
            return Err(ApiError::NotFound(format!("issue:{issue_id}")));
        }
        self.issue_primary_sessions.remove(issue_id);
        Ok(())
    }

    pub fn project_create(&mut self, name: &str) -> ProjectRecord {
        let id = Uuid::now_v7().to_string();
        let identifier = self.next_project_identifier(name);
        let project = ProjectRecord {
            id: id.clone(),
            name: name.to_string(),
            identifier,
            description: None,
            repo_local_path: None,
            repo_remote_url: None,
            repo_provider: None,
        };
        self.projects.insert(id.clone(), project.clone());
        project
    }

    pub fn project_list(&self) -> Vec<ProjectRecord> {
        let mut projects: Vec<_> = self.projects.values().cloned().collect();
        projects.sort_by(|a, b| {
            a.identifier
                .cmp(&b.identifier)
                .then_with(|| a.id.cmp(&b.id))
        });
        projects
    }

    pub fn project_set_identifier(
        &mut self,
        project_id: &str,
        identifier: &str,
    ) -> Result<ProjectRecord, ApiError> {
        let normalized = normalize_project_identifier(identifier)
            .ok_or_else(|| ApiError::BadRequest("invalid project identifier".to_string()))?;

        let has_issues = self
            .issues
            .values()
            .any(|issue| issue.project_id.as_deref() == Some(project_id));
        if has_issues {
            return Err(ApiError::BadRequest(
                "project identifier is immutable after first issue".to_string(),
            ));
        }

        let duplicate = self
            .projects
            .values()
            .any(|project| project.id != project_id && project.identifier == normalized);
        if duplicate {
            return Err(ApiError::Conflict {
                resource: format!("project_identifier:{normalized}"),
                expected: 0,
                actual: 1,
            });
        }

        let project = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| ApiError::NotFound(format!("project:{project_id}")))?;
        project.identifier = normalized;
        Ok(project.clone())
    }

    pub fn project_get(&self, project_id: &str) -> Result<ProjectRecord, ApiError> {
        self.projects
            .get(project_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("project:{project_id}")))
    }

    pub fn project_find_by_identifier(&self, identifier: &str) -> Option<ProjectRecord> {
        let key = identifier.trim().to_ascii_uppercase();
        self.projects
            .values()
            .find(|project| project.identifier == key)
            .cloned()
    }

    pub fn project_set_repo_local_path(
        &mut self,
        project_id: &str,
        repo_local_path: Option<String>,
    ) -> Result<ProjectRecord, ApiError> {
        let project = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| ApiError::NotFound(format!("project:{project_id}")))?;
        project.repo_local_path = repo_local_path.map(|value| value.trim().to_string());
        Ok(project.clone())
    }

    pub fn integration_connect(
        &mut self,
        integration_type: &str,
        project_id: &str,
    ) -> IntegrationProfileRecord {
        let id = Uuid::now_v7().to_string();
        let record = IntegrationProfileRecord {
            id: id.clone(),
            integration_type: integration_type.to_string(),
            project_id: project_id.to_string(),
        };
        self.integrations.insert(id.clone(), record.clone());
        record
    }

    pub fn integration_profiles(&self) -> Vec<IntegrationProfileRecord> {
        self.integrations.values().cloned().collect()
    }

    pub fn snapshot(&self) -> ApiSnapshot {
        ApiSnapshot {
            sessions: self.sessions.clone(),
            issues: self.issues.clone(),
            projects: self.projects.clone(),
            integrations: self.integrations.clone(),
            issue_primary_sessions: self.issue_primary_sessions.clone(),
            comments: self.comments.clone(),
        }
    }

    pub fn from_snapshot(snapshot: ApiSnapshot) -> Self {
        let mut service = Self {
            sessions: snapshot.sessions,
            issues: snapshot.issues,
            projects: snapshot.projects,
            integrations: snapshot.integrations,
            issue_primary_sessions: snapshot.issue_primary_sessions,
            comments: snapshot.comments,
        };
        service.normalize_identifiers();
        service
    }

    fn resolve_comment_entity_id(
        &self,
        entity_type: CommentEntityType,
        entity_ref: &str,
    ) -> Result<String, ApiError> {
        match entity_type {
            CommentEntityType::Issue => {
                if self.issue_get(entity_ref).is_ok() {
                    return Ok(entity_ref.to_string());
                }
                let upper = entity_ref.trim().to_ascii_uppercase();
                self.issues
                    .values()
                    .find(|issue| issue.identifier.as_deref() == Some(upper.as_str()))
                    .map(|issue| issue.id.clone())
                    .ok_or_else(|| ApiError::NotFound(format!("issue:{entity_ref}")))
            }
            CommentEntityType::Project => {
                if self.project_get(entity_ref).is_ok() {
                    return Ok(entity_ref.to_string());
                }
                if let Some(project) = self.project_find_by_identifier(entity_ref) {
                    return Ok(project.id);
                }
                let lower = entity_ref.to_ascii_lowercase();
                self.projects
                    .values()
                    .find(|project| project.name.to_ascii_lowercase() == lower)
                    .map(|project| project.id.clone())
                    .ok_or_else(|| ApiError::NotFound(format!("project:{entity_ref}")))
            }
        }
    }

    fn next_issue_number(&self, project_id: &str) -> u64 {
        self.issues
            .values()
            .filter(|issue| issue.project_id.as_deref() == Some(project_id))
            .filter_map(|issue| issue.issue_number)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    fn normalize_identifiers(&mut self) {
        let mut used_project_identifiers = HashSet::new();
        let mut project_ids: Vec<String> = self.projects.keys().cloned().collect();
        project_ids.sort();

        for project_id in project_ids {
            if let Some(project) = self.projects.get_mut(&project_id) {
                let base = normalize_project_identifier(&project.identifier)
                    .or_else(|| derive_project_identifier(&project.name))
                    .unwrap_or_else(|| "PRJ".to_string());
                let unique = dedupe_project_identifier(&base, &used_project_identifiers);
                project.identifier = unique.clone();
                used_project_identifiers.insert(unique);
            }
        }

        let mut project_to_max_issue_number: HashMap<String, u64> = HashMap::new();
        let mut issue_ids: Vec<String> = self.issues.keys().cloned().collect();
        issue_ids.sort();

        for issue_id in &issue_ids {
            let Some(issue) = self.issues.get(issue_id) else {
                continue;
            };
            let Some(project_id) = issue.project_id.as_ref() else {
                continue;
            };
            let Some(issue_number) = issue.issue_number else {
                continue;
            };
            let entry = project_to_max_issue_number
                .entry(project_id.clone())
                .or_insert(0);
            *entry = (*entry).max(issue_number);
        }

        for issue_id in issue_ids {
            let Some(issue) = self.issues.get_mut(&issue_id) else {
                continue;
            };
            let Some(project_id) = issue.project_id.clone() else {
                issue.identifier = None;
                issue.issue_number = None;
                continue;
            };
            let Some(project) = self.projects.get(&project_id) else {
                issue.identifier = None;
                issue.issue_number = None;
                continue;
            };
            if issue.issue_number.is_none() {
                let next = project_to_max_issue_number
                    .entry(project_id.clone())
                    .or_insert(0)
                    .saturating_add(1);
                project_to_max_issue_number.insert(project_id.clone(), next);
                issue.issue_number = Some(next);
            }
            issue.identifier = issue
                .issue_number
                .map(|issue_number| format!("{}-{issue_number:04}", project.identifier));
        }
    }

    fn next_project_identifier(&self, name: &str) -> String {
        let base = derive_project_identifier(name).unwrap_or_else(|| "PRJ".to_string());
        let used: HashSet<String> = self
            .projects
            .values()
            .map(|project| project.identifier.clone())
            .collect();
        dedupe_project_identifier(&base, &used)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), ApiError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| ApiError::Io(err.to_string()))?;
        }

        let json = serde_json::to_string_pretty(&self.snapshot())
            .map_err(|err| ApiError::Serialization(err.to_string()))?;
        fs::write(path, json).map_err(|err| ApiError::Io(err.to_string()))
    }

    pub fn load_from_file(path: &Path) -> Result<Self, ApiError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let json = fs::read_to_string(path).map_err(|err| ApiError::Io(err.to_string()))?;
        let snapshot: ApiSnapshot =
            serde_json::from_str(&json).map_err(|err| ApiError::Serialization(err.to_string()))?;
        Ok(Self::from_snapshot(snapshot))
    }
}

fn normalize_project_identifier(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.len() < 2 || upper.len() > 8 {
        return None;
    }
    let mut chars = upper.chars();
    let first = chars.next()?;
    if !first.is_ascii_uppercase() {
        return None;
    }
    if !chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
        return None;
    }
    Some(upper)
}

fn derive_project_identifier(name: &str) -> Option<String> {
    let letters: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        return None;
    }
    let mut key: String = letters.chars().take(4).collect();
    if key.len() < 2 {
        key.push('X');
    }
    normalize_project_identifier(&key)
}

fn dedupe_project_identifier(base: &str, used: &HashSet<String>) -> String {
    if !used.contains(base) {
        return base.to_string();
    }

    for suffix in 2..=9999 {
        let suffix_text = suffix.to_string();
        let keep_len = 8usize.saturating_sub(suffix_text.len());
        let prefix: String = base.chars().take(keep_len).collect();
        let candidate = format!("{prefix}{suffix_text}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }

    format!("{}{}", &base[..4.min(base.len())], Uuid::now_v7().simple())
        .chars()
        .take(8)
        .collect()
}

fn normalize_author(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn current_epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use orchestrator_core::SessionState;

    use super::{ApiService, CommentEntityType, CommentListOrder};

    #[test]
    fn handlers_create_and_fetch_session() {
        let mut api = ApiService::new();
        let session = api.session_create();

        let fetched = api.session_get(&session.id).expect("session should exist");
        assert_eq!(fetched.id, session.id);

        let updated = api
            .session_set_status(&session.id, SessionState::Running)
            .expect("status update should succeed");
        assert_eq!(updated.status, SessionState::Running);
        assert!(updated.version > session.version);
    }

    #[test]
    fn stale_write_returns_conflict() {
        let mut api = ApiService::new();
        let session = api.session_create();

        let _ = api
            .session_set_status_with_version(&session.id, SessionState::Running, session.version)
            .expect("first versioned write should succeed");

        let err = api
            .session_set_status_with_version(&session.id, SessionState::Busy, session.version)
            .expect_err("stale write should conflict");
        assert!(matches!(err, super::ApiError::Conflict { .. }));
    }

    #[test]
    fn handlers_create_and_move_issue() {
        let mut api = ApiService::new();
        let issue = api.issue_create("test issue");

        let moved = api
            .board_issue_move(&issue.id, "in_progress")
            .expect("issue move should succeed");
        assert_eq!(moved.status, "in_progress");

        api.issue_delete(&issue.id)
            .expect("issue delete should succeed");
        assert!(api.issue_get(&issue.id).is_err());
    }

    #[test]
    fn issue_can_link_to_primary_session() {
        let mut api = ApiService::new();
        let issue = api.issue_create("issue");
        let session = api.session_create();

        api.issue_link_primary_session(&issue.id, &session.id)
            .expect("link should succeed");
        assert_eq!(
            api.issue_primary_session(&issue.id),
            Some(session.id.as_str())
        );

        api.issue_unlink_primary_session(&issue.id)
            .expect("unlink should succeed");
        assert_eq!(api.issue_primary_session(&issue.id), None);
    }

    #[test]
    fn issue_launch_path_fields_roundtrip() {
        let mut api = ApiService::new();
        let project = api.project_create("proj");
        let issue = api.issue_create("issue");

        let updated_project = api
            .project_set_repo_local_path(&project.id, Some("/tmp/repo".to_string()))
            .expect("project path update should succeed");
        assert_eq!(
            updated_project.repo_local_path.as_deref(),
            Some("/tmp/repo")
        );

        let linked_issue = api
            .issue_assign_project(&issue.id, &project.id)
            .expect("issue assign should succeed");
        assert_eq!(
            linked_issue.project_id.as_deref(),
            Some(project.id.as_str())
        );
        assert_eq!(linked_issue.identifier.as_deref(), Some("PROJ-0001"));

        let overridden_issue = api
            .issue_set_cwd_override(&issue.id, Some("/tmp/repo-ticket".to_string()))
            .expect("issue cwd override should succeed");
        assert_eq!(
            overridden_issue.cwd_override_path.as_deref(),
            Some("/tmp/repo-ticket")
        );
    }

    #[test]
    fn project_identifiers_are_unique_and_normalized() {
        let mut api = ApiService::new();
        let a = api.project_create("Development");
        let b = api.project_create("DevOps");
        assert_eq!(a.identifier, "DEVE");
        assert_eq!(b.identifier, "DEVO");

        let c = api.project_create("Development");
        assert_ne!(c.identifier, a.identifier);
        assert!(c.identifier.starts_with("DEVE"));
    }

    #[test]
    fn issue_identifier_uses_project_key_and_sequence() {
        let mut api = ApiService::new();
        let project = api.project_create("Attribution");
        let a = api.issue_create("one");
        let b = api.issue_create("two");

        let a = api
            .issue_assign_project(&a.id, &project.id)
            .expect("assign first issue");
        let b = api
            .issue_assign_project(&b.id, &project.id)
            .expect("assign second issue");

        assert_eq!(a.identifier.as_deref(), Some("ATTR-0001"));
        assert_eq!(b.identifier.as_deref(), Some("ATTR-0002"));
        assert_eq!(a.issue_number, Some(1));
        assert_eq!(b.issue_number, Some(2));
    }

    #[test]
    fn project_identifier_is_immutable_after_first_issue() {
        let mut api = ApiService::new();
        let project = api.project_create("Development");
        let issue = api.issue_create("frozen key issue");
        api.issue_assign_project(&issue.id, &project.id)
            .expect("assign issue to project");

        let err = api
            .project_set_identifier(&project.id, "CORE")
            .expect_err("identifier should be immutable");
        assert!(matches!(err, super::ApiError::BadRequest(_)));
    }

    #[test]
    fn project_identifier_can_be_set_before_any_issue() {
        let mut api = ApiService::new();
        let project = api.project_create("Development");

        let updated = api
            .project_set_identifier(&project.id, "DEV")
            .expect("project key should be editable before issues");
        assert_eq!(updated.identifier, "DEV");
        assert_eq!(
            api.project_find_by_identifier("dev")
                .expect("lookup by identifier should work")
                .id,
            project.id
        );
    }

    #[test]
    fn comments_can_be_added_and_listed_newest_first_with_cursor() {
        let mut api = ApiService::new();
        let issue = api.issue_create("commented issue");

        let first = api
            .comment_add(
                CommentEntityType::Issue,
                &issue.id,
                "first comment",
                "alice",
            )
            .expect("first comment should be created");
        let second = api
            .comment_add(CommentEntityType::Issue, &issue.id, "second comment", "bob")
            .expect("second comment should be created");

        let page = api
            .comment_list(
                CommentEntityType::Issue,
                &issue.id,
                CommentListOrder::Desc,
                None,
                1,
            )
            .expect("comment listing should succeed");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, second.id);
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some(second.id.as_str()));

        let next_page = api
            .comment_list(
                CommentEntityType::Issue,
                &issue.id,
                CommentListOrder::Desc,
                page.next_cursor.as_deref(),
                10,
            )
            .expect("paged listing should succeed");
        assert_eq!(next_page.items.len(), 1);
        assert_eq!(next_page.items[0].id, first.id);
        assert!(!next_page.has_more);
    }

    #[test]
    fn deleting_issue_removes_linked_issue_comments() {
        let mut api = ApiService::new();
        let issue = api.issue_create("delete with comments");
        api.comment_add(
            CommentEntityType::Issue,
            &issue.id,
            "comment to be removed",
            "alice",
        )
        .expect("comment should be created");

        api.issue_delete(&issue.id)
            .expect("issue delete should succeed");

        let page = api
            .comment_list(
                CommentEntityType::Issue,
                &issue.id,
                CommentListOrder::Desc,
                None,
                20,
            )
            .expect_err("listing comments for deleted issue should fail");
        assert!(matches!(page, super::ApiError::NotFound(_)));
    }

    #[test]
    fn issue_title_can_be_updated() {
        let mut api = ApiService::new();
        let issue = api.issue_create("old title");

        let updated = api
            .issue_update_title(&issue.id, "new title")
            .expect("title update should succeed");
        assert_eq!(updated.title, "new title");
    }

    #[test]
    fn save_and_load_snapshot_roundtrip() {
        let temp_path = std::env::temp_dir().join("ddak-api-snapshot-test.json");
        let _ = std::fs::remove_file(&temp_path);

        let mut api = ApiService::new();
        let issue = api.issue_create("persisted issue");
        let session = api.session_create();
        api.issue_link_primary_session(&issue.id, &session.id)
            .expect("link should succeed");
        api.save_to_file(&temp_path)
            .expect("snapshot save should succeed");

        let loaded = ApiService::load_from_file(&temp_path).expect("snapshot load should succeed");
        assert_eq!(loaded.issue_list().len(), 1);
        assert_eq!(loaded.session_list().len(), 1);
        assert_eq!(
            loaded.issue_primary_session(&issue.id),
            Some(session.id.as_str())
        );
    }

    #[test]
    fn system_handlers_return_expected_values() {
        let api = ApiService::new();
        assert_eq!(api.system_health(), "ok");
        assert!(!api.system_version().is_empty());
        assert!(api.system_capabilities().contains(&"session"));
    }
}
