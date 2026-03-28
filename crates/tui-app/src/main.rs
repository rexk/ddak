use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use duckdb::{OptionalExt, params};
use orchestrator_core::config::{CliOverrides, Config, RuntimeMode};
use orchestrator_core::diagnostics::{DiagnosticEvent, DiagnosticsBundle};
use rpc_core::{
    ApiService, ApiSnapshot, CommentEntityType, CommentListOrder, CommentRecord, IssueRecord,
    ProjectRecord,
};
use serde_json::{Value, json};
use store_duckdb::open_and_migrate;

mod board_poc;
mod session_manager;

use board_poc::BoardPocApp;

const DEFAULT_STATE_FILE: &str = ".ddak/tickets.duckdb";
const SNAPSHOT_TABLE: &str = "ddak_state_snapshots";

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RuntimeModeArg {
    FatClient,
    DaemonStdio,
}

impl From<RuntimeModeArg> for RuntimeMode {
    fn from(value: RuntimeModeArg) -> Self {
        match value {
            RuntimeModeArg::FatClient => RuntimeMode::FatClient,
            RuntimeModeArg::DaemonStdio => RuntimeMode::DaemonStdio,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum ListOrderArg {
    Asc,
    #[default]
    Desc,
}

impl From<ListOrderArg> for CommentListOrder {
    fn from(value: ListOrderArg) -> Self {
        match value {
            ListOrderArg::Asc => CommentListOrder::Asc,
            ListOrderArg::Desc => CommentListOrder::Desc,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "ddak", about = "Terminal agent orchestrator")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, value_enum)]
    runtime_mode: Option<RuntimeModeArg>,
    #[arg(long)]
    linear_enabled: Option<bool>,
    #[arg(long)]
    linear_api_token: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,

    #[arg(long)]
    no_ui: bool,
    #[arg(long)]
    export_diagnostics: Option<PathBuf>,
    #[arg(long)]
    state_file: Option<PathBuf>,
    #[arg(long)]
    opencode_cmd: Option<String>,
    #[arg(long)]
    claude_cmd: Option<String>,
    #[arg(long)]
    session_cwd: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Issue(IssueArgs),
    Project(ProjectArgs),
    Mcp(McpArgs),
}

#[derive(Debug, Args)]
struct IssueArgs {
    #[arg(long)]
    state_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t)]
    output: OutputFormat,
    #[command(subcommand)]
    command: IssueCommand,
}

#[derive(Debug, Subcommand)]
enum IssueCommand {
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        project: Option<String>,
    },
    Get {
        issue: String,
    },
    Move {
        issue: String,
        #[arg(long)]
        status: String,
    },
    AssignProject {
        issue: String,
        #[arg(long)]
        project: String,
    },
    SetCwd {
        issue: String,
        #[arg(long)]
        path: String,
    },
    ClearCwd {
        issue: String,
    },
    Delete {
        issue: String,
    },
    CommentAdd {
        issue: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<PathBuf>,
        #[arg(long)]
        author: Option<String>,
    },
    CommentList {
        issue: String,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, value_enum, default_value_t)]
        order: ListOrderArg,
    },
}

#[derive(Debug, Args)]
struct ProjectArgs {
    #[arg(long)]
    state_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t)]
    output: OutputFormat,
    #[command(subcommand)]
    command: ProjectCommand,
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    List,
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        key: Option<String>,
    },
    Get {
        project: String,
    },
    SetKey {
        project: String,
        #[arg(long)]
        key: String,
    },
    SetRepoPath {
        project: String,
        #[arg(long)]
        path: String,
    },
    ClearRepoPath {
        project: String,
    },
    CommentAdd {
        project: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<PathBuf>,
        #[arg(long)]
        author: Option<String>,
    },
    CommentList {
        project: String,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, value_enum, default_value_t)]
        order: ListOrderArg,
    },
}

#[derive(Debug, Args)]
struct McpArgs {
    #[command(subcommand)]
    command: McpCommand,
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    Serve {
        #[arg(long)]
        state_file: Option<PathBuf>,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let config = Config::load(&to_cli_overrides(&cli)).map_err(|err| err.to_string())?;

    match cli.command {
        Some(Command::Issue(args)) => run_issue_command(args),
        Some(Command::Project(args)) => run_project_command(args),
        Some(Command::Mcp(args)) => run_mcp_command(args),
        None => run_tui(config, &cli),
    }
}

fn to_cli_overrides(cli: &Cli) -> CliOverrides {
    CliOverrides {
        config_path: cli.config.clone(),
        runtime_mode: cli.runtime_mode.map(Into::into),
        linear_enabled: cli.linear_enabled,
        linear_api_token: cli.linear_api_token.clone(),
    }
}

fn run_tui(cfg: Config, cli: &Cli) -> Result<(), String> {
    println!("ddak bootstrap");
    println!("runtime_mode={:?}", cfg.runtime.mode);

    let api = ApiService::new();
    println!("api_health={}", api.system_health());

    if let Some(path) = cli.export_diagnostics.as_ref() {
        let bundle = DiagnosticsBundle {
            app_version: api.system_version().to_string(),
            runtime_mode: format!("{:?}", cfg.runtime.mode),
            session_snapshot_json: serde_json::to_string(&api.session_list())
                .unwrap_or_else(|_| "[]".to_string()),
            events: vec![DiagnosticEvent {
                correlation_id: "corr-startup".to_string(),
                event_type: "system.start".to_string(),
                payload: "token=demo-token".to_string(),
            }],
        };

        let json = bundle
            .to_redacted_json()
            .map_err(|err| format!("failed to serialize diagnostics bundle: {err}"))?;
        std::fs::write(path, json)
            .map_err(|err| format!("failed to write diagnostics bundle: {err}"))?;
        println!("diagnostics_exported={}", path.display());
    }

    if cli.no_ui {
        return Ok(());
    }

    let mut app = BoardPocApp::new_with_key_bindings(
        cli.state_file.clone(),
        cli.opencode_cmd.clone(),
        cli.claude_cmd.clone(),
        cli.session_cwd.clone(),
        Some(cfg.tui.key_bindings.clone()),
    );
    app.run().map_err(|err| format!("poc app failed: {err}"))
}

fn run_issue_command(args: IssueArgs) -> Result<(), String> {
    let state_path = resolve_state_path(args.state_file.as_deref());
    let mut api = load_api(&state_path)?;

    match args.command {
        IssueCommand::List { status, project } => {
            let issues = op_issue_list(&api, status.as_deref(), project.as_deref())?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&issues)
                            .map_err(|err| format!("failed to encode issues: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    for issue in issues {
                        println!("{} [{}] {}", issue_label(&issue), issue.status, issue.title);
                    }
                }
            }
            Ok(())
        }
        IssueCommand::Create { title, project } => {
            let issue = op_issue_create(&mut api, &title, project.as_deref())?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&issue)
                            .map_err(|err| format!("failed to encode issue: {err}"))?
                    );
                }
                OutputFormat::Text => println!("created issue {}", issue_label(&issue)),
            }
            Ok(())
        }
        IssueCommand::Get { issue } => {
            let issue = op_issue_get(&api, &issue)?;
            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&issue)
                            .map_err(|err| format!("failed to encode issue: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("{} [{}] {}", issue_label(&issue), issue.status, issue.title);
                }
            }
            Ok(())
        }
        IssueCommand::Move { issue, status } => {
            let updated = op_issue_move(&mut api, &issue, &status)?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode issue: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!(
                        "moved issue {} to {}",
                        issue_label(&updated),
                        updated.status
                    )
                }
            }
            Ok(())
        }
        IssueCommand::AssignProject { issue, project } => {
            let updated = op_issue_assign_project(&mut api, &issue, &project)?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode issue: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("assigned issue {} to {project}", issue_label(&updated));
                }
            }
            Ok(())
        }
        IssueCommand::SetCwd { issue, path } => {
            let updated = op_issue_set_cwd(&mut api, &issue, Some(path))?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode issue: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("set cwd override for issue {}", issue_label(&updated));
                }
            }
            Ok(())
        }
        IssueCommand::ClearCwd { issue } => {
            let updated = op_issue_set_cwd(&mut api, &issue, None)?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode issue: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("cleared cwd override for issue {}", issue_label(&updated));
                }
            }
            Ok(())
        }
        IssueCommand::Delete { issue } => {
            op_issue_delete(&mut api, &issue)?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!("{}", json!({ "deleted": issue }));
                }
                OutputFormat::Text => {
                    println!("deleted issue {issue}");
                }
            }
            Ok(())
        }
        IssueCommand::CommentAdd {
            issue,
            body,
            body_file,
            author,
        } => {
            let body_markdown = load_comment_body(body, body_file)?;
            let comment = op_comment_add(
                &mut api,
                CommentEntityType::Issue,
                &issue,
                &body_markdown,
                author.as_deref(),
            )?;
            save_api(&api, &state_path)?;
            print_comment_output(args.output, &comment)?;
            Ok(())
        }
        IssueCommand::CommentList {
            issue,
            all,
            cursor,
            limit,
            order,
        } => {
            let page = op_comment_list(
                &api,
                CommentEntityType::Issue,
                &issue,
                all,
                cursor.as_deref(),
                limit,
                order.into(),
            )?;
            print_comment_list_output(args.output, &page.items, page.next_cursor.as_deref())?;
            Ok(())
        }
    }
}

fn run_project_command(args: ProjectArgs) -> Result<(), String> {
    let state_path = resolve_state_path(args.state_file.as_deref());
    let mut api = load_api(&state_path)?;

    match args.command {
        ProjectCommand::List => {
            let projects = api.project_list();
            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&projects)
                            .map_err(|err| format!("failed to encode projects: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    for project in projects {
                        println!("{} {}", project_label(&project), project.name);
                    }
                }
            }
            Ok(())
        }
        ProjectCommand::Create { name, key } => {
            let project = op_project_create(&mut api, &name, key.as_deref())?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&project)
                            .map_err(|err| format!("failed to encode project: {err}"))?
                    );
                }
                OutputFormat::Text => println!("created project {}", project_label(&project)),
            }
            Ok(())
        }
        ProjectCommand::Get { project } => {
            let project = op_project_get(&api, &project)?;
            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&project)
                            .map_err(|err| format!("failed to encode project: {err}"))?
                    );
                }
                OutputFormat::Text => println!("{} {}", project_label(&project), project.name),
            }
            Ok(())
        }
        ProjectCommand::SetKey { project, key } => {
            let updated = op_project_set_key(&mut api, &project, &key)?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode project: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("updated project key to {}", project_label(&updated))
                }
            }
            Ok(())
        }
        ProjectCommand::SetRepoPath { project, path } => {
            let updated = op_project_set_repo_path(&mut api, &project, Some(path))?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode project: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("updated repo path for project {}", project_label(&updated));
                }
            }
            Ok(())
        }
        ProjectCommand::ClearRepoPath { project } => {
            let updated = op_project_set_repo_path(&mut api, &project, None)?;
            save_api(&api, &state_path)?;

            match args.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&updated)
                            .map_err(|err| format!("failed to encode project: {err}"))?
                    );
                }
                OutputFormat::Text => {
                    println!("cleared repo path for project {}", project_label(&updated));
                }
            }
            Ok(())
        }
        ProjectCommand::CommentAdd {
            project,
            body,
            body_file,
            author,
        } => {
            let body_markdown = load_comment_body(body, body_file)?;
            let comment = op_comment_add(
                &mut api,
                CommentEntityType::Project,
                &project,
                &body_markdown,
                author.as_deref(),
            )?;
            save_api(&api, &state_path)?;
            print_comment_output(args.output, &comment)?;
            Ok(())
        }
        ProjectCommand::CommentList {
            project,
            all,
            cursor,
            limit,
            order,
        } => {
            let page = op_comment_list(
                &api,
                CommentEntityType::Project,
                &project,
                all,
                cursor.as_deref(),
                limit,
                order.into(),
            )?;
            print_comment_list_output(args.output, &page.items, page.next_cursor.as_deref())?;
            Ok(())
        }
    }
}

fn run_mcp_command(args: McpArgs) -> Result<(), String> {
    match args.command {
        McpCommand::Serve { state_file } => {
            let state_path = resolve_state_path(state_file.as_deref());
            run_mcp_stdio_server(&state_path)
        }
    }
}

fn run_mcp_stdio_server(state_path: &Path) -> Result<(), String> {
    let mut api = load_api(state_path)?;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.map_err(|err| format!("failed reading stdin: {err}"))?;
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                let response = mcp_error_response(None, -32700, &format!("parse error: {err}"));
                writeln!(stdout, "{response}")
                    .map_err(|write_err| format!("failed writing stdout: {write_err}"))?;
                stdout
                    .flush()
                    .map_err(|flush_err| format!("failed flushing stdout: {flush_err}"))?;
                continue;
            }
        };

        let (response, mutated) = handle_mcp_request(&mut api, &request);
        if mutated {
            save_api(&api, state_path)?;
        }
        if let Some(response) = response {
            writeln!(stdout, "{response}")
                .map_err(|err| format!("failed writing stdout: {err}"))?;
            stdout
                .flush()
                .map_err(|err| format!("failed flushing stdout: {err}"))?;
        }
    }

    Ok(())
}

fn handle_mcp_request(api: &mut ApiService, request: &Value) -> (Option<Value>, bool) {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);

    let Some(method) = method else {
        return (
            Some(mcp_error_response(
                id,
                -32600,
                "invalid request: missing method",
            )),
            false,
        );
    };

    match method {
        "initialize" => (
            Some(mcp_success_response(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "ddak",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )),
            false,
        ),
        "notifications/initialized" => (None, false),
        "tools/list" => (
            Some(mcp_success_response(
                id,
                json!({ "tools": mcp_tools_schema() }),
            )),
            false,
        ),
        "resources/list" => (
            Some(mcp_success_response(
                id,
                json!({ "resources": mcp_resources_schema() }),
            )),
            false,
        ),
        "resources/read" => {
            let Some(params) = request.get("params") else {
                return (
                    Some(mcp_error_response(
                        id,
                        -32602,
                        "missing params for resources/read",
                    )),
                    false,
                );
            };
            let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                return (
                    Some(mcp_error_response(id, -32602, "missing resource uri")),
                    false,
                );
            };
            match mcp_read_resource(api, uri) {
                Ok(payload) => (
                    Some(mcp_success_response(
                        id,
                        json!({
                            "contents": [{
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
                            }]
                        }),
                    )),
                    false,
                ),
                Err(err) => (Some(mcp_error_response(id, -32000, &err)), false),
            }
        }
        "tools/call" => {
            let Some(params) = request.get("params") else {
                return (
                    Some(mcp_error_response(
                        id,
                        -32602,
                        "missing params for tools/call",
                    )),
                    false,
                );
            };
            let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
                return (
                    Some(mcp_error_response(id, -32602, "missing tool name")),
                    false,
                );
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            if is_deprecated_mcp_tool(tool_name) {
                return (
                    Some(mcp_error_response(
                        id,
                        -32010,
                        &format!(
                            "tool '{tool_name}' is deprecated for MCP; use the corresponding CLI subcommand"
                        ),
                    )),
                    false,
                );
            }

            match call_mcp_tool(api, tool_name, &arguments) {
                Ok((payload, mutated)) => (
                    Some(mcp_success_response(
                        id,
                        json!({
                            "content": [{
                                "type": "text",
                                "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
                            }],
                            "structuredContent": payload
                        }),
                    )),
                    mutated,
                ),
                Err(err) => (Some(mcp_error_response(id, -32000, &err)), false),
            }
        }
        _ => (
            Some(mcp_error_response(
                id,
                -32601,
                &format!("method not found: {method}"),
            )),
            false,
        ),
    }
}

fn call_mcp_tool(
    api: &mut ApiService,
    tool_name: &str,
    arguments: &Value,
) -> Result<(Value, bool), String> {
    let args = arguments
        .as_object()
        .ok_or_else(|| "tool arguments must be an object".to_string())?;

    match tool_name {
        "issue_list" => {
            let issues = op_issue_list(
                api,
                args.get("status").and_then(Value::as_str),
                args.get("project").and_then(Value::as_str),
            )?;
            Ok((json!({ "issues": issues }), false))
        }
        "issue_get" => {
            let issue_ref = args
                .get("issue")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_get requires 'issue'".to_string())?;
            let issue = op_issue_get(api, issue_ref)?;
            Ok((json!({ "issue": issue }), false))
        }
        "issue_create" => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_create requires 'title'".to_string())?;
            let issue = op_issue_create(api, title, args.get("project").and_then(Value::as_str))?;
            Ok((json!({ "issue": issue }), true))
        }
        "issue_move" => {
            let issue_ref = args
                .get("issue")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_move requires 'issue'".to_string())?;
            let status = args
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_move requires 'status'".to_string())?;
            let issue = op_issue_move(api, issue_ref, status)?;
            Ok((json!({ "issue": issue }), true))
        }
        "issue_assign_project" => {
            let issue_ref = args
                .get("issue")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_assign_project requires 'issue'".to_string())?;
            let project_ref = args
                .get("project")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_assign_project requires 'project'".to_string())?;
            let issue = op_issue_assign_project(api, issue_ref, project_ref)?;
            Ok((json!({ "issue": issue }), true))
        }
        "issue_set_cwd" => {
            let issue_ref = args
                .get("issue")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_set_cwd requires 'issue'".to_string())?;
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_set_cwd requires 'path'".to_string())?;
            let issue = op_issue_set_cwd(api, issue_ref, Some(path.to_string()))?;
            Ok((json!({ "issue": issue }), true))
        }
        "issue_clear_cwd" => {
            let issue_ref = args
                .get("issue")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_clear_cwd requires 'issue'".to_string())?;
            let issue = op_issue_set_cwd(api, issue_ref, None)?;
            Ok((json!({ "issue": issue }), true))
        }
        "issue_delete" => {
            let issue_ref = args
                .get("issue")
                .and_then(Value::as_str)
                .ok_or_else(|| "issue_delete requires 'issue'".to_string())?;
            op_issue_delete(api, issue_ref)?;
            Ok((json!({ "deleted": issue_ref }), true))
        }
        "project_list" => Ok((json!({ "projects": api.project_list() }), false)),
        "project_get" => {
            let project_ref = args
                .get("project")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_get requires 'project'".to_string())?;
            let project = op_project_get(api, project_ref)?;
            Ok((json!({ "project": project }), false))
        }
        "project_create" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_create requires 'name'".to_string())?;
            let project = op_project_create(api, name, args.get("key").and_then(Value::as_str))?;
            Ok((json!({ "project": project }), true))
        }
        "project_set_key" => {
            let project_ref = args
                .get("project")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_set_key requires 'project'".to_string())?;
            let key = args
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_set_key requires 'key'".to_string())?;
            let project = op_project_set_key(api, project_ref, key)?;
            Ok((json!({ "project": project }), true))
        }
        "comment_add" => {
            let entity_type = parse_comment_entity_type(
                args.get("entity_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "comment_add requires 'entity_type'".to_string())?,
            )?;
            let entity = args
                .get("entity")
                .and_then(Value::as_str)
                .ok_or_else(|| "comment_add requires 'entity'".to_string())?;
            let body_markdown = args
                .get("body_markdown")
                .and_then(Value::as_str)
                .ok_or_else(|| "comment_add requires 'body_markdown'".to_string())?;
            let comment = op_comment_add(
                api,
                entity_type,
                entity,
                body_markdown,
                args.get("author").and_then(Value::as_str),
            )?;
            Ok((json!({ "comment": comment }), true))
        }
        "comment_list" => {
            let entity_type = parse_comment_entity_type(
                args.get("entity_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "comment_list requires 'entity_type'".to_string())?,
            )?;
            let entity = args
                .get("entity")
                .and_then(Value::as_str)
                .ok_or_else(|| "comment_list requires 'entity'".to_string())?;
            let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
            let cursor = args.get("cursor").and_then(Value::as_str);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let order = parse_comment_list_order(
                args.get("order").and_then(Value::as_str).unwrap_or("desc"),
            )?;
            let page = op_comment_list(api, entity_type, entity, all, cursor, limit, order)?;
            Ok((
                json!({
                    "items": page.items,
                    "next_cursor": page.next_cursor,
                    "has_more": page.has_more,
                }),
                false,
            ))
        }
        "project_set_repo_path" => {
            let project_ref = args
                .get("project")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_set_repo_path requires 'project'".to_string())?;
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_set_repo_path requires 'path'".to_string())?;
            let project = op_project_set_repo_path(api, project_ref, Some(path.to_string()))?;
            Ok((json!({ "project": project }), true))
        }
        "project_clear_repo_path" => {
            let project_ref = args
                .get("project")
                .and_then(Value::as_str)
                .ok_or_else(|| "project_clear_repo_path requires 'project'".to_string())?;
            let project = op_project_set_repo_path(api, project_ref, None)?;
            Ok((json!({ "project": project }), true))
        }
        _ => Err(format!("unknown tool: {tool_name}")),
    }
}

fn mcp_tools_schema() -> Vec<Value> {
    vec![
        json!({
            "name": "issue_get",
            "description": "Get issue by key or internal id",
            "inputSchema": {
                "type": "object",
                "required": ["issue"],
                "properties": {
                    "issue": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "issue_create",
            "description": "Create a new issue",
            "inputSchema": {
                "type": "object",
                "required": ["title"],
                "properties": {
                    "title": {"type": "string"},
                    "project": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "issue_move",
            "description": "Move issue to a new status",
            "inputSchema": {
                "type": "object",
                "required": ["issue", "status"],
                "properties": {
                    "issue": {"type": "string"},
                    "status": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "issue_assign_project",
            "description": "Assign issue to a project",
            "inputSchema": {
                "type": "object",
                "required": ["issue", "project"],
                "properties": {
                    "issue": {"type": "string"},
                    "project": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "project_get",
            "description": "Get project by key/name/id",
            "inputSchema": {
                "type": "object",
                "required": ["project"],
                "properties": {
                    "project": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "project_create",
            "description": "Create project with optional key",
            "inputSchema": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "key": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "project_set_key",
            "description": "Set project key before first issue exists",
            "inputSchema": {
                "type": "object",
                "required": ["project", "key"],
                "properties": {
                    "project": {"type": "string"},
                    "key": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "comment_add",
            "description": "Add markdown comment to issue or project",
            "inputSchema": {
                "type": "object",
                "required": ["entity_type", "entity", "body_markdown"],
                "properties": {
                    "entity_type": {"type": "string", "enum": ["issue", "project"]},
                    "entity": {"type": "string"},
                    "body_markdown": {"type": "string"},
                    "author": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "comment_list",
            "description": "List comments for issue or project with cursor pagination",
            "inputSchema": {
                "type": "object",
                "required": ["entity_type", "entity"],
                "properties": {
                    "entity_type": {"type": "string", "enum": ["issue", "project"]},
                    "entity": {"type": "string"},
                    "all": {"type": "boolean"},
                    "cursor": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1},
                    "order": {"type": "string", "enum": ["asc", "desc"]}
                }
            }
        }),
    ]
}

fn is_deprecated_mcp_tool(name: &str) -> bool {
    matches!(
        name,
        "issue_list"
            | "issue_set_cwd"
            | "issue_clear_cwd"
            | "issue_delete"
            | "project_list"
            | "project_set_repo_path"
            | "project_clear_repo_path"
    )
}

fn mcp_resources_schema() -> Vec<Value> {
    vec![
        json!({
            "uri": "ddak://projects",
            "name": "Projects",
            "description": "All projects in ddak state",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "ddak://issues",
            "name": "Issues",
            "description": "All issues in ddak state",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "ddak://health",
            "name": "System health",
            "description": "Current API health/version/capabilities",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "ddak://issues/{issue}/comments",
            "name": "Issue comments",
            "description": "Comments for issue id or key",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "ddak://projects/{project}/comments",
            "name": "Project comments",
            "description": "Comments for project id, key, or name",
            "mimeType": "application/json"
        }),
    ]
}

fn mcp_read_resource(api: &ApiService, uri: &str) -> Result<Value, String> {
    match uri {
        "ddak://projects" => Ok(json!({ "projects": api.project_list() })),
        "ddak://issues" => Ok(json!({ "issues": api.issue_list() })),
        "ddak://health" => Ok(json!({
            "health": api.system_health(),
            "version": api.system_version(),
            "capabilities": api.system_capabilities(),
        })),
        _ => {
            if let Some(issue_ref) = uri
                .strip_prefix("ddak://issues/")
                .and_then(|rest| rest.strip_suffix("/comments"))
            {
                let page = op_comment_list(
                    api,
                    CommentEntityType::Issue,
                    issue_ref,
                    true,
                    None,
                    usize::MAX,
                    CommentListOrder::Desc,
                )?;
                return Ok(json!({
                    "items": page.items,
                    "next_cursor": page.next_cursor,
                    "has_more": page.has_more,
                }));
            }
            if let Some(project_ref) = uri
                .strip_prefix("ddak://projects/")
                .and_then(|rest| rest.strip_suffix("/comments"))
            {
                let page = op_comment_list(
                    api,
                    CommentEntityType::Project,
                    project_ref,
                    true,
                    None,
                    usize::MAX,
                    CommentListOrder::Desc,
                )?;
                return Ok(json!({
                    "items": page.items,
                    "next_cursor": page.next_cursor,
                    "has_more": page.has_more,
                }));
            }
            Err(format!("unknown resource uri: {uri}"))
        }
    }
}

fn mcp_success_response(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result
    })
}

fn mcp_error_response(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn op_issue_list(
    api: &ApiService,
    status: Option<&str>,
    project: Option<&str>,
) -> Result<Vec<IssueRecord>, String> {
    let project_id = project
        .map(|reference| resolve_project_id(api, reference))
        .transpose()?;
    let mut issues = api.issue_list();
    if let Some(status_filter) = status {
        issues.retain(|issue| issue.status == status_filter);
    }
    if let Some(project_id) = project_id.as_deref() {
        issues.retain(|issue| issue.project_id.as_deref() == Some(project_id));
    }
    Ok(issues)
}

fn op_issue_get(api: &ApiService, issue_ref: &str) -> Result<IssueRecord, String> {
    let issue_id = resolve_issue_id(api, issue_ref)?;
    api.issue_get(&issue_id).map_err(|err| err.to_string())
}

fn op_issue_create(
    api: &mut ApiService,
    title: &str,
    project: Option<&str>,
) -> Result<IssueRecord, String> {
    let mut issue = api.issue_create(title);
    if let Some(project_ref) = project {
        let project_id = resolve_project_id(api, project_ref)?;
        issue = api
            .issue_assign_project(&issue.id, &project_id)
            .map_err(|err| err.to_string())?;
    }
    Ok(issue)
}

fn op_issue_move(
    api: &mut ApiService,
    issue_ref: &str,
    status: &str,
) -> Result<IssueRecord, String> {
    let issue_id = resolve_issue_id(api, issue_ref)?;
    api.board_issue_move(&issue_id, status)
        .map_err(|err| err.to_string())
}

fn op_issue_assign_project(
    api: &mut ApiService,
    issue_ref: &str,
    project_ref: &str,
) -> Result<IssueRecord, String> {
    let issue_id = resolve_issue_id(api, issue_ref)?;
    let project_id = resolve_project_id(api, project_ref)?;
    api.issue_assign_project(&issue_id, &project_id)
        .map_err(|err| err.to_string())
}

fn op_issue_set_cwd(
    api: &mut ApiService,
    issue_ref: &str,
    path: Option<String>,
) -> Result<IssueRecord, String> {
    let issue_id = resolve_issue_id(api, issue_ref)?;
    api.issue_set_cwd_override(&issue_id, path)
        .map_err(|err| err.to_string())
}

fn op_issue_delete(api: &mut ApiService, issue_ref: &str) -> Result<(), String> {
    let issue_id = resolve_issue_id(api, issue_ref)?;
    api.issue_delete(&issue_id).map_err(|err| err.to_string())
}

fn op_project_get(api: &ApiService, project_ref: &str) -> Result<ProjectRecord, String> {
    let project_id = resolve_project_id(api, project_ref)?;
    api.project_get(&project_id).map_err(|err| err.to_string())
}

fn op_project_create(
    api: &mut ApiService,
    name: &str,
    key: Option<&str>,
) -> Result<ProjectRecord, String> {
    let mut project = api.project_create(name);
    if let Some(key) = key {
        project = api
            .project_set_identifier(&project.id, key)
            .map_err(|err| err.to_string())?;
    }
    Ok(project)
}

fn op_project_set_key(
    api: &mut ApiService,
    project_ref: &str,
    key: &str,
) -> Result<ProjectRecord, String> {
    let project_id = resolve_project_id(api, project_ref)?;
    api.project_set_identifier(&project_id, key)
        .map_err(|err| err.to_string())
}

fn op_project_set_repo_path(
    api: &mut ApiService,
    project_ref: &str,
    path: Option<String>,
) -> Result<ProjectRecord, String> {
    let project_id = resolve_project_id(api, project_ref)?;
    api.project_set_repo_local_path(&project_id, path)
        .map_err(|err| err.to_string())
}

fn op_comment_add(
    api: &mut ApiService,
    entity_type: CommentEntityType,
    entity_ref: &str,
    body_markdown: &str,
    author: Option<&str>,
) -> Result<CommentRecord, String> {
    let author = author
        .map(str::to_string)
        .unwrap_or_else(default_comment_author);
    api.comment_add(entity_type, entity_ref, body_markdown, &author)
        .map_err(|err| err.to_string())
}

fn op_comment_list(
    api: &ApiService,
    entity_type: CommentEntityType,
    entity_ref: &str,
    all: bool,
    cursor: Option<&str>,
    limit: usize,
    order: CommentListOrder,
) -> Result<rpc_core::CommentListPage, String> {
    let page_limit = if all { usize::MAX } else { limit.max(1) };
    api.comment_list(entity_type, entity_ref, order, cursor, page_limit)
        .map_err(|err| err.to_string())
}

fn load_comment_body(body: Option<String>, body_file: Option<PathBuf>) -> Result<String, String> {
    match (body, body_file) {
        (Some(body), None) => {
            if body.trim().is_empty() {
                Err("comment body cannot be empty".to_string())
            } else {
                Ok(body)
            }
        }
        (None, Some(path)) => {
            let content = std::fs::read_to_string(&path)
                .map_err(|err| format!("failed reading comment body file: {err}"))?;
            if content.trim().is_empty() {
                Err("comment body cannot be empty".to_string())
            } else {
                Ok(content)
            }
        }
        (Some(_), Some(_)) => Err("provide either --body or --body-file, not both".to_string()),
        (None, None) => Err("comment body required via --body or --body-file".to_string()),
    }
}

fn print_comment_output(output: OutputFormat, comment: &CommentRecord) -> Result<(), String> {
    match output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(comment)
                .map_err(|err| format!("failed to encode comment: {err}"))?
        ),
        OutputFormat::Text => {
            println!(
                "{} {}",
                comment.author,
                trim_single_line(&comment.body_markdown, 96)
            );
        }
    }
    Ok(())
}

fn print_comment_list_output(
    output: OutputFormat,
    comments: &[CommentRecord],
    next_cursor: Option<&str>,
) -> Result<(), String> {
    match output {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "items": comments,
                    "next_cursor": next_cursor,
                    "has_more": next_cursor.is_some(),
                }))
                .map_err(|err| format!("failed to encode comments page: {err}"))?
            );
        }
        OutputFormat::Text => {
            for comment in comments {
                println!(
                    "{} {}",
                    comment.author,
                    trim_single_line(&comment.body_markdown, 96)
                );
            }
            if let Some(cursor) = next_cursor {
                println!("next_cursor={cursor}");
            }
        }
    }
    Ok(())
}

fn default_comment_author() -> String {
    std::env::var("USER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn trim_single_line(input: &str, max_len: usize) -> String {
    let one_line = input.lines().next().unwrap_or_default().trim();
    if one_line.chars().count() <= max_len {
        return one_line.to_string();
    }
    one_line
        .chars()
        .take(max_len.saturating_sub(1))
        .collect::<String>()
        + "~"
}

fn parse_comment_entity_type(raw: &str) -> Result<CommentEntityType, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "issue" => Ok(CommentEntityType::Issue),
        "project" => Ok(CommentEntityType::Project),
        _ => Err(format!(
            "invalid comment entity_type '{raw}', expected 'issue' or 'project'"
        )),
    }
}

fn parse_comment_list_order(raw: &str) -> Result<CommentListOrder, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "asc" => Ok(CommentListOrder::Asc),
        "desc" => Ok(CommentListOrder::Desc),
        _ => Err(format!(
            "invalid comment order '{raw}', expected 'asc' or 'desc'"
        )),
    }
}

fn resolve_state_path(path: Option<&Path>) -> PathBuf {
    path.map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_FILE))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateBackend {
    Json,
    DuckDb,
}

fn detect_state_backend(path: &Path) -> StateBackend {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("duckdb") || ext.eq_ignore_ascii_case("db") => {
            StateBackend::DuckDb
        }
        _ => StateBackend::Json,
    }
}

fn ensure_state_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed creating state directory: {err}"))?;
    }
    Ok(())
}

fn duckdb_connection(path: &Path) -> Result<duckdb::Connection, String> {
    ensure_state_parent_dir(path)?;
    let db_path = path
        .to_str()
        .ok_or_else(|| format!("invalid state path: {}", path.display()))?;
    let conn = open_and_migrate(db_path).map_err(|err| format!("failed opening duckdb: {err}"))?;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {SNAPSHOT_TABLE} (id INTEGER PRIMARY KEY CHECK (id = 1), snapshot_json TEXT NOT NULL, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);"
    ))
    .map_err(|err| format!("failed ensuring state snapshot table: {err}"))?;
    Ok(conn)
}

fn load_api(path: &Path) -> Result<ApiService, String> {
    match detect_state_backend(path) {
        StateBackend::Json => {
            ApiService::load_from_file(path).map_err(|err| format!("failed loading state: {err}"))
        }
        StateBackend::DuckDb => {
            let conn = duckdb_connection(path)?;
            let snapshot_json: Option<String> = conn
                .query_row(
                    &format!("SELECT snapshot_json FROM {SNAPSHOT_TABLE} WHERE id = 1"),
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|err| format!("failed reading state snapshot: {err}"))?;
            let Some(snapshot_json) = snapshot_json else {
                return Ok(ApiService::new());
            };
            let snapshot: ApiSnapshot = serde_json::from_str(&snapshot_json)
                .map_err(|err| format!("failed decoding duckdb state snapshot: {err}"))?;
            Ok(ApiService::from_snapshot(snapshot))
        }
    }
}

fn save_api(api: &ApiService, path: &Path) -> Result<(), String> {
    match detect_state_backend(path) {
        StateBackend::Json => api
            .save_to_file(path)
            .map_err(|err| format!("failed saving state: {err}")),
        StateBackend::DuckDb => {
            let conn = duckdb_connection(path)?;
            let snapshot_json = serde_json::to_string_pretty(&api.snapshot())
                .map_err(|err| format!("failed serializing state snapshot: {err}"))?;
            conn.execute(
                &format!(
                    "INSERT INTO {SNAPSHOT_TABLE}(id, snapshot_json) VALUES (1, ?) ON CONFLICT(id) DO UPDATE SET snapshot_json = excluded.snapshot_json, updated_at = now()"
                ),
                params![snapshot_json],
            )
            .map_err(|err| format!("failed writing duckdb state snapshot: {err}"))?;
            Ok(())
        }
    }
}

fn resolve_project_id(api: &ApiService, project_ref: &str) -> Result<String, String> {
    if api.project_get(project_ref).is_ok() {
        return Ok(project_ref.to_string());
    }
    if let Some(project) = api.project_find_by_identifier(project_ref) {
        return Ok(project.id);
    }

    let lower = project_ref.to_ascii_lowercase();
    if let Some(project) = api
        .project_list()
        .into_iter()
        .find(|project| project.name.to_ascii_lowercase() == lower)
    {
        return Ok(project.id);
    }

    Err(format!("unknown project: {project_ref}"))
}

fn resolve_issue_id(api: &ApiService, issue_ref: &str) -> Result<String, String> {
    if api.issue_get(issue_ref).is_ok() {
        return Ok(issue_ref.to_string());
    }

    let upper = issue_ref.trim().to_ascii_uppercase();
    if let Some(issue) = api
        .issue_list()
        .into_iter()
        .find(|issue| issue.identifier.as_deref() == Some(upper.as_str()))
    {
        return Ok(issue.id);
    }

    Err(format!("unknown issue: {issue_ref}"))
}

fn issue_label(issue: &IssueRecord) -> String {
    issue
        .identifier
        .clone()
        .unwrap_or_else(|| issue.id.chars().take(8).collect())
}

fn project_label(project: &ProjectRecord) -> String {
    if project.identifier.is_empty() {
        project.id.chars().take(8).collect()
    } else {
        project.identifier.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, IssueCommand, ProjectCommand, handle_mcp_request};
    use clap::Parser;
    use rpc_core::ApiService;
    use serde_json::json;

    #[test]
    fn parses_legacy_tui_flags_without_subcommand() {
        let cli = Cli::parse_from(["ddak", "--no-ui", "--state-file", "state.json"]);
        assert!(cli.no_ui);
        assert_eq!(
            cli.state_file
                .expect("state file should parse")
                .to_string_lossy(),
            "state.json"
        );
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_issue_subcommands() {
        let cli = Cli::parse_from([
            "ddak",
            "issue",
            "create",
            "--title",
            "ship cli",
            "--project",
            "DEV",
        ]);
        match cli.command {
            Some(Command::Issue(args)) => match args.command {
                IssueCommand::Create { title, project } => {
                    assert_eq!(title, "ship cli");
                    assert_eq!(project.as_deref(), Some("DEV"));
                }
                other => panic!("unexpected issue command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_issue_assign_project_subcommand() {
        let cli = Cli::parse_from([
            "ddak",
            "issue",
            "assign-project",
            "DEV-0001",
            "--project",
            "DEV",
        ]);
        match cli.command {
            Some(Command::Issue(args)) => match args.command {
                IssueCommand::AssignProject { issue, project } => {
                    assert_eq!(issue, "DEV-0001");
                    assert_eq!(project, "DEV");
                }
                other => panic!("unexpected issue command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_issue_comment_list_subcommand() {
        let cli = Cli::parse_from([
            "ddak",
            "issue",
            "comment-list",
            "DEV-0001",
            "--limit",
            "50",
            "--order",
            "asc",
        ]);
        match cli.command {
            Some(Command::Issue(args)) => match args.command {
                IssueCommand::CommentList {
                    issue,
                    limit,
                    order,
                    ..
                } => {
                    assert_eq!(issue, "DEV-0001");
                    assert_eq!(limit, 50);
                    assert!(matches!(order, super::ListOrderArg::Asc));
                }
                other => panic!("unexpected issue command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_project_subcommands() {
        let cli = Cli::parse_from(["ddak", "project", "set-key", "DEV", "--key", "CORE"]);
        match cli.command {
            Some(Command::Project(args)) => match args.command {
                ProjectCommand::SetKey { project, key } => {
                    assert_eq!(project, "DEV");
                    assert_eq!(key, "CORE");
                }
                other => panic!("unexpected project command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_project_set_repo_path_subcommand() {
        let cli = Cli::parse_from([
            "ddak",
            "project",
            "set-repo-path",
            "DEV",
            "--path",
            "/tmp/repo",
        ]);
        match cli.command {
            Some(Command::Project(args)) => match args.command {
                ProjectCommand::SetRepoPath { project, path } => {
                    assert_eq!(project, "DEV");
                    assert_eq!(path, "/tmp/repo");
                }
                other => panic!("unexpected project command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_project_comment_add_subcommand() {
        let cli = Cli::parse_from([
            "ddak",
            "project",
            "comment-add",
            "DEV",
            "--body",
            "markdown note",
        ]);
        match cli.command {
            Some(Command::Project(args)) => match args.command {
                ProjectCommand::CommentAdd { project, body, .. } => {
                    assert_eq!(project, "DEV");
                    assert_eq!(body.as_deref(), Some("markdown note"));
                }
                other => panic!("unexpected project command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_mcp_serve_subcommand() {
        let cli = Cli::parse_from(["ddak", "mcp", "serve", "--state-file", "test-state.json"]);
        match cli.command {
            Some(Command::Mcp(_)) => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn mcp_tools_call_creates_issue() {
        let mut api = ApiService::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "issue_create",
                "arguments": {
                    "title": "Ship MCP"
                }
            }
        });

        let (response, mutated) = handle_mcp_request(&mut api, &req);
        assert!(mutated);
        let response = response.expect("tools/call should return response");
        assert!(response.get("result").is_some());
        assert_eq!(api.issue_list().len(), 1);
    }

    #[test]
    fn mcp_tools_list_is_curated_and_minimal() {
        let mut api = ApiService::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });

        let (response, mutated) = handle_mcp_request(&mut api, &req);
        assert!(!mutated);
        let response = response.expect("tools/list should return response");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools should be an array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|name| name.as_str()))
            .collect();
        assert!(names.contains(&"issue_get"));
        assert!(names.contains(&"issue_create"));
        assert!(names.contains(&"issue_move"));
        assert!(names.contains(&"issue_assign_project"));
        assert!(names.contains(&"project_get"));
        assert!(names.contains(&"project_create"));
        assert!(names.contains(&"project_set_key"));
        assert!(names.contains(&"comment_add"));
        assert!(names.contains(&"comment_list"));
        assert!(!names.contains(&"issue_delete"));
        assert!(!names.contains(&"project_set_repo_path"));
    }

    #[test]
    fn mcp_deprecated_tool_is_rejected_with_cli_guidance() {
        let mut api = ApiService::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "issue_delete",
                "arguments": {
                    "issue": "DEV-0001"
                }
            }
        });

        let (response, mutated) = handle_mcp_request(&mut api, &req);
        assert!(!mutated);
        let response = response.expect("deprecated tool should return response");
        assert_eq!(response["error"]["code"], json!(-32010));
    }

    #[test]
    fn mcp_comment_add_and_list_work() {
        let mut api = ApiService::new();
        let issue = api.issue_create("comment target");

        let add_req = json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "comment_add",
                "arguments": {
                    "entity_type": "issue",
                    "entity": issue.id,
                    "body_markdown": "hello **markdown**"
                }
            }
        });
        let (add_response, mutated) = handle_mcp_request(&mut api, &add_req);
        assert!(mutated);
        assert!(add_response.is_some());

        let list_req = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "comment_list",
                "arguments": {
                    "entity_type": "issue",
                    "entity": issue.id,
                    "order": "desc",
                    "limit": 10
                }
            }
        });
        let (list_response, mutated) = handle_mcp_request(&mut api, &list_req);
        assert!(!mutated);
        let list_response = list_response.expect("comment list should return response");
        let items = list_response["result"]["structuredContent"]["items"]
            .as_array()
            .expect("items should be array");
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn mcp_resources_list_and_read_work() {
        let mut api = ApiService::new();

        let list_req = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/list"
        });
        let (list_response, mutated) = handle_mcp_request(&mut api, &list_req);
        assert!(!mutated);
        let list_response = list_response.expect("resources/list should return response");
        let resources = list_response["result"]["resources"]
            .as_array()
            .expect("resources should be an array");
        assert!(
            resources
                .iter()
                .any(|resource| resource.get("uri") == Some(&json!("ddak://issues")))
        );
        assert!(resources.iter().any(|resource| {
            resource.get("uri") == Some(&json!("ddak://issues/{issue}/comments"))
        }));

        let read_req = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "resources/read",
            "params": { "uri": "ddak://health" }
        });
        let (read_response, mutated) = handle_mcp_request(&mut api, &read_req);
        assert!(!mutated);
        let read_response = read_response.expect("resources/read should return response");
        let contents = read_response["result"]["contents"]
            .as_array()
            .expect("contents should be an array");
        assert!(!contents.is_empty());
    }
}
