use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("wkm binary not found in PATH")]
    WkmNotAvailable,
    #[error("repository at {0} is not wkm-managed (no .git/wkm.toml)")]
    NotWkmRepo(PathBuf),
    #[error("wkm command failed: {0}")]
    CommandFailed(String),
    #[error("workspace creation failed: {0}")]
    WorkspaceCreateFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse wkm.toml: {0}")]
    ParseFailed(String),
}

// ---------------------------------------------------------------------------
// wkm.toml read-path structs (deserialize only, never write)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct WkmState {
    pub version: u32,
    pub config: WkmConfig,
    #[serde(default)]
    pub branches: std::collections::BTreeMap<String, BranchEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WkmConfig {
    pub base_branch: String,
    #[serde(default)]
    pub storage_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BranchEntry {
    pub parent: Option<String>,
    pub worktree_path: Option<String>,
    pub description: Option<String>,
    pub created_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Detection (read path — fast, no process spawn)
// ---------------------------------------------------------------------------

/// Check if `wkm` binary is available in PATH.
pub fn is_wkm_available() -> bool {
    Command::new("wkm")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Check if `repo_path` is a wkm-managed repository (has `.git/wkm.toml`).
pub fn is_wkm_repo(repo_path: &Path) -> bool {
    repo_path.join(".git").join("wkm.toml").is_file()
}

/// Parse `.git/wkm.toml` from a repo path.
/// Returns `None` if the file doesn't exist or the version is unsupported.
pub fn read_wkm_state(repo_path: &Path) -> Option<WkmState> {
    let toml_path = repo_path.join(".git").join("wkm.toml");
    let content = std::fs::read_to_string(toml_path).ok()?;
    let state: WkmState = toml::from_str(&content).ok()?;
    if state.version != 1 {
        return None;
    }
    Some(state)
}

// ---------------------------------------------------------------------------
// Mutation (shell out to wkm CLI)
// ---------------------------------------------------------------------------

/// Derive branch name from a session ID (first 8 chars).
pub fn branch_name_for_session(session_id: &str) -> String {
    let short = &session_id[..session_id.len().min(8)];
    format!("ddak/{short}")
}

/// Create a new branch + worktree for a session.
///
/// Runs `wkm checkout -b <branch>` then `wkm worktree create <branch>`,
/// both with `cwd=repo_path`.
/// Returns the absolute path to the new worktree.
pub fn create_session_worktree(
    repo_path: &Path,
    session_id: &str,
) -> Result<PathBuf, WorktreeError> {
    if !is_wkm_available() {
        return Err(WorktreeError::WkmNotAvailable);
    }
    if !is_wkm_repo(repo_path) {
        return Err(WorktreeError::NotWkmRepo(repo_path.to_path_buf()));
    }

    let branch = branch_name_for_session(session_id);

    // Step 1: create the branch
    let output = Command::new("wkm")
        .args(["checkout", "-b", &branch])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::WorkspaceCreateFailed(format!(
            "wkm checkout -b {branch} failed: {}",
            stderr.trim()
        )));
    }

    // Step 2: create the worktree
    let output = Command::new("wkm")
        .args(["worktree", "create", &branch])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::WorkspaceCreateFailed(format!(
            "wkm worktree create {branch} failed: {}",
            stderr.trim()
        )));
    }

    // Read wkm.toml to find the worktree path
    let state = read_wkm_state(repo_path).ok_or_else(|| {
        WorktreeError::ParseFailed("could not read wkm.toml after worktree creation".to_string())
    })?;

    let entry = state.branches.get(&branch).ok_or_else(|| {
        WorktreeError::ParseFailed(format!("branch {branch} not found in wkm.toml"))
    })?;

    let worktree_path = entry.worktree_path.as_ref().ok_or_else(|| {
        WorktreeError::ParseFailed(format!("branch {branch} has no worktree_path in wkm.toml"))
    })?;

    let path = PathBuf::from(worktree_path);
    Ok(path.canonicalize().unwrap_or(path))
}

/// Remove a worktree for a session.
///
/// Runs `wkm worktree remove <branch>` with `cwd=repo_path`.
pub fn remove_session_worktree(repo_path: &Path, branch_name: &str) -> Result<(), WorktreeError> {
    let output = Command::new("wkm")
        .args(["worktree", "remove", branch_name])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::CommandFailed(format!(
            "wkm worktree remove {branch_name} failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn branch_name_for_session_uses_first_8_chars() {
        let name = branch_name_for_session("abcdef1234567890");
        assert_eq!(name, "ddak/abcdef12");
    }

    #[test]
    fn branch_name_for_short_session_id() {
        let name = branch_name_for_session("abc");
        assert_eq!(name, "ddak/abc");
    }

    #[test]
    fn is_wkm_repo_returns_false_for_plain_dir() {
        let dir = std::env::temp_dir().join("ddak_test_no_wkm");
        let _ = fs::create_dir_all(&dir);
        assert!(!is_wkm_repo(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_wkm_repo_returns_true_when_wkm_toml_exists() {
        let dir = std::env::temp_dir().join("ddak_test_with_wkm");
        let git_dir = dir.join(".git");
        let _ = fs::create_dir_all(&git_dir);
        fs::write(
            git_dir.join("wkm.toml"),
            "version = 1\n[config]\nbase_branch = \"main\"\n",
        )
        .expect("write wkm.toml");
        assert!(is_wkm_repo(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_wkm_state_parses_valid_toml() {
        let dir = std::env::temp_dir().join("ddak_test_parse_wkm");
        let git_dir = dir.join(".git");
        let _ = fs::create_dir_all(&git_dir);
        fs::write(
            git_dir.join("wkm.toml"),
            r#"
version = 1

[config]
base_branch = "main"
storage_dir = "/tmp/worktrees"

[branches.feature-xyz]
parent = "main"
worktree_path = "/tmp/worktrees/feature-xyz"
created_at = "2024-01-15T10:30:45Z"
"#,
        )
        .expect("write wkm.toml");

        let state = read_wkm_state(&dir).expect("should parse");
        assert_eq!(state.version, 1);
        assert_eq!(state.config.base_branch, "main");
        assert_eq!(state.config.storage_dir.as_deref(), Some("/tmp/worktrees"));
        assert!(state.branches.contains_key("feature-xyz"));
        let entry = &state.branches["feature-xyz"];
        assert_eq!(entry.parent.as_deref(), Some("main"));
        assert_eq!(
            entry.worktree_path.as_deref(),
            Some("/tmp/worktrees/feature-xyz")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_wkm_state_returns_none_for_unsupported_version() {
        let dir = std::env::temp_dir().join("ddak_test_wkm_v2");
        let git_dir = dir.join(".git");
        let _ = fs::create_dir_all(&git_dir);
        fs::write(
            git_dir.join("wkm.toml"),
            "version = 2\n[config]\nbase_branch = \"main\"\n",
        )
        .expect("write wkm.toml");
        assert!(read_wkm_state(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_wkm_state_returns_none_for_missing_file() {
        let dir = std::env::temp_dir().join("ddak_test_wkm_missing");
        let _ = fs::create_dir_all(&dir);
        assert!(read_wkm_state(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
