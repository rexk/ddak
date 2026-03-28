use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const ENV_CONFIG_PATH: &str = "DDAK_CONFIG_PATH";
const ENV_RUNTIME_MODE: &str = "DDAK_RUNTIME_MODE";
const ENV_LINEAR_ENABLED: &str = "DDAK_LINEAR_ENABLED";
const ENV_LINEAR_API_TOKEN: &str = "DDAK_LINEAR_API_TOKEN";
const DEFAULT_CONFIG_RELATIVE_PATH: &str = "ddak/config.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub runtime: RuntimeConfig,
    pub integration: IntegrationConfig,
    pub tui: TuiConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub mode: RuntimeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    FatClient,
    DaemonStdio,
}

impl RuntimeMode {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value {
            "fat_client" => Ok(Self::FatClient),
            "daemon_stdio" => Ok(Self::DaemonStdio),
            other => Err(ConfigError::InvalidRuntimeMode(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationConfig {
    pub linear: LinearConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearConfig {
    pub enabled: bool,
    pub api_token: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiConfig {
    #[serde(default)]
    pub key_bindings: TuiKeyBindingsConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiKeyBindingsConfig {
    pub quit: Option<String>,
    pub new_issue: Option<String>,
    pub move_issue: Option<String>,
    pub launch_opencode: Option<String>,
    pub launch_claude: Option<String>,
    pub launch_shell: Option<String>,
    pub send_input: Option<String>,
    pub set_project_path: Option<String>,
    pub set_issue_cwd: Option<String>,
    pub close_session: Option<String>,
    pub delete_issue: Option<String>,
    pub refresh_output: Option<String>,
    pub resize_left: Option<String>,
    pub resize_right: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            runtime: RuntimeConfig {
                mode: RuntimeMode::FatClient,
            },
            integration: IntegrationConfig {
                linear: LinearConfig {
                    enabled: false,
                    api_token: None,
                },
            },
            tui: TuiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOverrides {
    pub config_path: Option<PathBuf>,
    pub runtime_mode: Option<RuntimeMode>,
    pub linear_enabled: Option<bool>,
    pub linear_api_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialConfig {
    #[serde(default)]
    runtime: PartialRuntimeConfig,
    #[serde(default)]
    integration: PartialIntegrationConfig,
    #[serde(default)]
    tui: PartialTuiConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialRuntimeConfig {
    mode: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialIntegrationConfig {
    #[serde(default)]
    linear: PartialLinearConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialLinearConfig {
    enabled: Option<bool>,
    api_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialTuiConfig {
    #[serde(default)]
    key_bindings: PartialTuiKeyBindingsConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialTuiKeyBindingsConfig {
    quit: Option<String>,
    new_issue: Option<String>,
    move_issue: Option<String>,
    launch_opencode: Option<String>,
    launch_claude: Option<String>,
    launch_shell: Option<String>,
    send_input: Option<String>,
    set_project_path: Option<String>,
    set_issue_cwd: Option<String>,
    close_session: Option<String>,
    delete_issue: Option<String>,
    refresh_output: Option<String>,
    resize_left: Option<String>,
    resize_right: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid runtime mode '{0}', expected fat_client or daemon_stdio")]
    InvalidRuntimeMode(String),
    #[error("invalid boolean value for {name}: '{value}'")]
    InvalidBoolean { name: String, value: String },
    #[error("failed to read config file '{path}': {source}")]
    ReadConfigFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config file '{path}': {source}")]
    ParseConfigFile {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("integration.linear.enabled is true but no API token was provided")]
    MissingLinearApiToken,
}

impl Config {
    pub fn load(cli: &CliOverrides) -> Result<Self, ConfigError> {
        let env_vars = collect_env();
        let config_path = cli
            .config_path
            .clone()
            .or_else(|| env_vars.get(ENV_CONFIG_PATH).map(PathBuf::from))
            .or_else(|| discover_default_config_path(&env_vars));

        let file_overrides = match config_path {
            Some(path) => Some(load_file(&path)?),
            None => None,
        };

        Self::resolve(file_overrides.as_ref(), &env_vars, cli)
    }

    fn resolve(
        file_overrides: Option<&PartialConfig>,
        env_vars: &HashMap<String, String>,
        cli: &CliOverrides,
    ) -> Result<Self, ConfigError> {
        let mut cfg = Self::default();

        if let Some(file) = file_overrides {
            if let Some(mode) = &file.runtime.mode {
                cfg.runtime.mode = RuntimeMode::parse(mode)?;
            }
            if let Some(enabled) = file.integration.linear.enabled {
                cfg.integration.linear.enabled = enabled;
            }
            if let Some(token) = file.integration.linear.api_token.as_ref() {
                cfg.integration.linear.api_token = Some(token.clone());
            }
            merge_key_bindings(&mut cfg.tui.key_bindings, &file.tui.key_bindings);
        }

        if let Some(mode) = env_vars.get(ENV_RUNTIME_MODE) {
            cfg.runtime.mode = RuntimeMode::parse(mode)?;
        }
        if let Some(enabled) = env_vars.get(ENV_LINEAR_ENABLED) {
            cfg.integration.linear.enabled = parse_bool(enabled, ENV_LINEAR_ENABLED)?;
        }
        if let Some(token) = env_vars.get(ENV_LINEAR_API_TOKEN) {
            cfg.integration.linear.api_token = Some(token.clone());
        }

        if let Some(mode) = cli.runtime_mode {
            cfg.runtime.mode = mode;
        }
        if let Some(enabled) = cli.linear_enabled {
            cfg.integration.linear.enabled = enabled;
        }
        if let Some(token) = cli.linear_api_token.as_ref() {
            cfg.integration.linear.api_token = Some(token.clone());
        }

        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.integration.linear.enabled
            && self
                .integration
                .linear
                .api_token
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(ConfigError::MissingLinearApiToken);
        }
        Ok(())
    }
}

fn merge_key_bindings(target: &mut TuiKeyBindingsConfig, source: &PartialTuiKeyBindingsConfig) {
    copy_opt(&mut target.quit, &source.quit);
    copy_opt(&mut target.new_issue, &source.new_issue);
    copy_opt(&mut target.move_issue, &source.move_issue);
    copy_opt(&mut target.launch_opencode, &source.launch_opencode);
    copy_opt(&mut target.launch_claude, &source.launch_claude);
    copy_opt(&mut target.launch_shell, &source.launch_shell);
    copy_opt(&mut target.send_input, &source.send_input);
    copy_opt(&mut target.set_project_path, &source.set_project_path);
    copy_opt(&mut target.set_issue_cwd, &source.set_issue_cwd);
    copy_opt(&mut target.close_session, &source.close_session);
    copy_opt(&mut target.delete_issue, &source.delete_issue);
    copy_opt(&mut target.refresh_output, &source.refresh_output);
    copy_opt(&mut target.resize_left, &source.resize_left);
    copy_opt(&mut target.resize_right, &source.resize_right);
}

fn copy_opt(target: &mut Option<String>, source: &Option<String>) {
    if let Some(value) = source.as_ref() {
        *target = Some(value.clone());
    }
}

fn discover_default_config_path(env_vars: &HashMap<String, String>) -> Option<PathBuf> {
    let xdg = env_vars
        .get("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .map(|dir| dir.join(DEFAULT_CONFIG_RELATIVE_PATH));
    if let Some(path) = xdg
        && path.exists()
    {
        return Some(path);
    }

    env_vars
        .get("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config").join(DEFAULT_CONFIG_RELATIVE_PATH))
        .filter(|path| path.exists())
}

fn load_file(path: &Path) -> Result<PartialConfig, ConfigError> {
    let data = fs::read_to_string(path).map_err(|source| ConfigError::ReadConfigFile {
        path: path.to_path_buf(),
        source,
    })?;

    toml::from_str(&data).map_err(|source| ConfigError::ParseConfigFile {
        path: path.to_path_buf(),
        source,
    })
}

fn collect_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn parse_bool(value: &str, name: &str) -> Result<bool, ConfigError> {
    match value {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::InvalidBoolean {
            name: name.to_string(),
            value: value.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_local_and_secure() {
        let cfg = Config::default();
        assert_eq!(cfg.runtime.mode, RuntimeMode::FatClient);
        assert!(!cfg.integration.linear.enabled);
        assert_eq!(cfg.integration.linear.api_token, None);
        assert_eq!(cfg.tui.key_bindings.launch_shell, None);
    }

    #[test]
    fn parses_supported_runtime_modes() {
        assert_eq!(
            RuntimeMode::parse("fat_client").unwrap(),
            RuntimeMode::FatClient
        );
        assert_eq!(
            RuntimeMode::parse("daemon_stdio").unwrap(),
            RuntimeMode::DaemonStdio
        );
    }

    #[test]
    fn rejects_unknown_runtime_mode_with_clear_error() {
        let err = RuntimeMode::parse("daemon_http").unwrap_err();
        assert!(matches!(err, ConfigError::InvalidRuntimeMode(_)));
        assert!(
            err.to_string()
                .contains("expected fat_client or daemon_stdio")
        );
    }

    #[test]
    fn applies_precedence_cli_over_env_over_file_over_defaults() {
        let file = PartialConfig {
            runtime: PartialRuntimeConfig {
                mode: Some("fat_client".to_string()),
            },
            integration: PartialIntegrationConfig {
                linear: PartialLinearConfig {
                    enabled: Some(false),
                    api_token: Some("file-token".to_string()),
                },
            },
            tui: PartialTuiConfig {
                key_bindings: PartialTuiKeyBindingsConfig {
                    launch_shell: Some("z".to_string()),
                    ..PartialTuiKeyBindingsConfig::default()
                },
            },
        };
        let env = HashMap::from([
            (ENV_RUNTIME_MODE.to_string(), "daemon_stdio".to_string()),
            (ENV_LINEAR_ENABLED.to_string(), "true".to_string()),
            (ENV_LINEAR_API_TOKEN.to_string(), "env-token".to_string()),
        ]);
        let cli = CliOverrides {
            runtime_mode: Some(RuntimeMode::FatClient),
            linear_enabled: Some(true),
            linear_api_token: Some("cli-token".to_string()),
            ..CliOverrides::default()
        };

        let cfg = Config::resolve(Some(&file), &env, &cli).unwrap();
        assert_eq!(cfg.runtime.mode, RuntimeMode::FatClient);
        assert!(cfg.integration.linear.enabled);
        assert_eq!(
            cfg.integration.linear.api_token.as_deref(),
            Some("cli-token")
        );
        assert_eq!(cfg.tui.key_bindings.launch_shell.as_deref(), Some("z"));
    }

    #[test]
    fn env_overrides_file_when_cli_absent() {
        let file = PartialConfig {
            runtime: PartialRuntimeConfig {
                mode: Some("fat_client".to_string()),
            },
            integration: PartialIntegrationConfig {
                linear: PartialLinearConfig {
                    enabled: Some(false),
                    api_token: Some("file-token".to_string()),
                },
            },
            tui: PartialTuiConfig::default(),
        };
        let env = HashMap::from([
            (ENV_RUNTIME_MODE.to_string(), "daemon_stdio".to_string()),
            (ENV_LINEAR_ENABLED.to_string(), "true".to_string()),
            (ENV_LINEAR_API_TOKEN.to_string(), "env-token".to_string()),
        ]);

        let cfg = Config::resolve(Some(&file), &env, &CliOverrides::default()).unwrap();
        assert_eq!(cfg.runtime.mode, RuntimeMode::DaemonStdio);
        assert!(cfg.integration.linear.enabled);
        assert_eq!(
            cfg.integration.linear.api_token.as_deref(),
            Some("env-token")
        );
    }

    #[test]
    fn linear_enabled_requires_api_token() {
        let env = HashMap::from([(ENV_LINEAR_ENABLED.to_string(), "true".to_string())]);
        let err = Config::resolve(None, &env, &CliOverrides::default()).unwrap_err();
        assert!(matches!(err, ConfigError::MissingLinearApiToken));
    }

    #[test]
    fn default_config_path_uses_xdg_then_home() {
        let temp_root = std::env::temp_dir().join(format!(
            "ddak-config-discovery-test-{}",
            uuid::Uuid::now_v7()
        ));
        let xdg_home = temp_root.join("xdg");
        let user_home = temp_root.join("home");
        let xdg_path = xdg_home.join(DEFAULT_CONFIG_RELATIVE_PATH);
        let home_path = user_home.join(".config").join(DEFAULT_CONFIG_RELATIVE_PATH);

        let _ = fs::create_dir_all(xdg_path.parent().expect("xdg parent"));
        let _ = fs::create_dir_all(home_path.parent().expect("home parent"));
        fs::write(&home_path, "[runtime]\nmode='fat_client'\n").expect("home config write");

        let env = HashMap::from([
            (
                "XDG_CONFIG_HOME".to_string(),
                xdg_home.to_string_lossy().into_owned(),
            ),
            ("HOME".to_string(), user_home.to_string_lossy().into_owned()),
        ]);

        let discovered = discover_default_config_path(&env).expect("path should be discovered");
        assert_eq!(discovered, home_path);

        fs::write(&xdg_path, "[runtime]\nmode='fat_client'\n").expect("xdg config write");
        let discovered = discover_default_config_path(&env).expect("xdg path should be discovered");
        assert_eq!(discovered, xdg_path);
    }
}
