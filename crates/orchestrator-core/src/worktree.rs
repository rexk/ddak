use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("jj binary not found in PATH")]
    JjNotAvailable,
    #[error("repository at {0} is not jj-managed (no .jj/ directory)")]
    NotJjRepo(PathBuf),
    #[error("jj command failed: {0}")]
    CommandFailed(String),
    #[error("workspace creation failed: {0}")]
    WorkspaceCreateFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Check if `jj` binary is available in PATH.
pub fn is_jj_available() -> bool {
    Command::new("jj")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Check if `repo_path` is a jj-managed repository (has `.jj/` directory).
pub fn is_jj_repo(repo_path: &Path) -> bool {
    repo_path.join(".jj").is_dir()
}

/// Derive workspace name from a session ID (first 8 chars).
pub fn workspace_name_for_session(session_id: &str) -> String {
    let short = &session_id[..session_id.len().min(8)];
    format!("ddak-{short}")
}

/// Derive the workspace directory path.
///
/// Workspaces are stored at `{repo_parent}/.ddak-workspaces/{workspace_name}`.
fn workspace_dir(repo_path: &Path, workspace_name: &str) -> PathBuf {
    let parent = repo_path.parent().unwrap_or(repo_path);
    parent.join(".ddak-workspaces").join(workspace_name)
}

/// Create a jj workspace for a session.
///
/// Runs `jj workspace add --name=<name> <path>` with `cwd=repo_path`.
/// Returns the absolute path to the new workspace.
pub fn create_session_workspace(
    repo_path: &Path,
    session_id: &str,
) -> Result<PathBuf, WorktreeError> {
    if !is_jj_available() {
        return Err(WorktreeError::JjNotAvailable);
    }
    if !is_jj_repo(repo_path) {
        return Err(WorktreeError::NotJjRepo(repo_path.to_path_buf()));
    }

    let ws_name = workspace_name_for_session(session_id);
    let ws_dir = workspace_dir(repo_path, &ws_name);

    let output = Command::new("jj")
        .arg("workspace")
        .arg("add")
        .arg(format!("--name={ws_name}"))
        .arg(&ws_dir)
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::WorkspaceCreateFailed(
            stderr.trim().to_string(),
        ));
    }

    // Return the canonical path if possible, otherwise the computed path
    Ok(ws_dir.canonicalize().unwrap_or(ws_dir))
}

/// Remove a jj workspace.
///
/// Runs `jj workspace forget <name>` with `cwd=repo_path`, then removes the
/// workspace directory from disk.
pub fn remove_session_workspace(
    repo_path: &Path,
    workspace_name: &str,
) -> Result<(), WorktreeError> {
    let output = Command::new("jj")
        .arg("workspace")
        .arg("forget")
        .arg(workspace_name)
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::CommandFailed(stderr.trim().to_string()));
    }

    let ws_dir = workspace_dir(repo_path, workspace_name);
    if ws_dir.exists() {
        std::fs::remove_dir_all(&ws_dir)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn workspace_name_for_session_uses_first_8_chars() {
        let name = workspace_name_for_session("abcdef1234567890");
        assert_eq!(name, "ddak-abcdef12");
    }

    #[test]
    fn workspace_name_for_short_session_id() {
        let name = workspace_name_for_session("abc");
        assert_eq!(name, "ddak-abc");
    }

    #[test]
    fn workspace_dir_derives_path_correctly() {
        let repo = PathBuf::from("/home/user/my-repo");
        let dir = workspace_dir(&repo, "ddak-abc123");
        assert_eq!(
            dir,
            PathBuf::from("/home/user/.ddak-workspaces/ddak-abc123")
        );
    }

    #[test]
    fn is_jj_repo_returns_false_for_plain_dir() {
        let dir = std::env::temp_dir().join("ddak_test_no_jj");
        let _ = fs::create_dir_all(&dir);
        assert!(!is_jj_repo(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_jj_repo_returns_true_when_jj_dir_exists() {
        let dir = std::env::temp_dir().join("ddak_test_with_jj");
        let _ = fs::create_dir_all(dir.join(".jj"));
        assert!(is_jj_repo(&dir));
        let _ = fs::remove_dir_all(&dir);
    }
}
