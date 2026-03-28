use std::fs;
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crossbeam_channel::Receiver;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::queue;
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, terminal::size};
use duckdb::{OptionalExt, params};
use orchestrator_core::config::TuiKeyBindingsConfig;
use orchestrator_core::{RenderRequest, ScreenStore, SessionState};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Modifier as TuiModifier, Style as TuiStyle};
use ratatui::text::Line as TuiLine;
use ratatui::widgets::{Block, BorderType, Borders, Clear as TuiClear, Paragraph};
use rpc_core::{ApiService, ApiSnapshot, CommentEntityType, IssueRecord};
use runtime_pty::PtyConfig;
use store_duckdb::open_and_migrate;

use crate::session_manager::SessionManager;

const DEFAULT_STATE_FILE: &str = ".ddak/tickets.duckdb";
const SNAPSHOT_TABLE: &str = "ddak_state_snapshots";
const DEFAULT_PROJECT_NAME: &str = "default";
const DEFAULT_BOARD_RATIO_PERCENT: u16 = 45;
const MIN_PANE_COLS: u16 = 24;
const ACTION_BAR_Y: u16 = 1;
const BOARD_START_Y: u16 = 3;
const STATUSES: [&str; 6] = [
    "backlog",
    "ready",
    "in_progress",
    "review",
    "done",
    "blocked",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputMode {
    Normal,
    Terminal,
    NewIssue(String),
    NewIssueDescription {
        title: String,
        description: String,
        active_field: FormField,
    },
    SelectIssueProject {
        title: String,
        description: String,
        query: String,
        selected_index: usize,
    },
    EditIssue {
        title: String,
        description: String,
        active_field: FormField,
    },
    NewProject(String),
    AddComment(String),
    SendInput(String),
    Command(String),
    Filter(String),
    MoveStatus,
    DeleteIssueConfirm,
    SetProjectIdentifier(String),
    SetProjectRepoPath(String),
    SetIssueCwdOverride(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftSubview {
    Issues,
    Projects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightPaneMode {
    Session,
    Comments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    Title,
    Description,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeMode {
    Terminal,
    Session,
    Ocean,
    Amber,
    Mono,
}

#[derive(Clone, Copy)]
struct ThemePalette {
    background: ratatui::style::Color,
    foreground: ratatui::style::Color,
    border: ratatui::style::Color,
    focus: ratatui::style::Color,
    muted: ratatui::style::Color,
    pane_bg: Color,
    header_bg: Color,
    header_fg: Color,
    selection_bg: Color,
    selection_fg: Color,
    status_bar_bg: Color,
    status_bar_fg: Color,
    accent: Color,
    dim: Color,
}

#[derive(Clone, Copy)]
enum MouseAction {
    ViewIssues,
    ViewProjects,
    NewIssue,
    NewProject,
    SetProjectKey,
    AddComment,
    EditIssue,
    LaunchOpenCode,
    LaunchClaude,
    LaunchShell,
    SetProjectPath,
    SetIssueCwd,
    CloseSession,
    DeleteIssue,
}

#[derive(Debug, Clone)]
struct KeyBindings {
    quit: char,
    new_issue: char,
    move_issue: char,
    launch_opencode: char,
    launch_claude: char,
    launch_shell: char,
    send_input: char,
    set_project_path: char,
    set_issue_cwd: char,
    close_session: char,
    delete_issue: char,
    refresh_output: char,
    resize_left: char,
    resize_right: char,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            quit: 'q',
            new_issue: 'n',
            move_issue: 'm',
            launch_opencode: 'o',
            launch_claude: 'c',
            launch_shell: 'h',
            send_input: 's',
            set_project_path: 'p',
            set_issue_cwd: 'v',
            close_session: 'x',
            delete_issue: 'D',
            refresh_output: 'r',
            resize_left: '<',
            resize_right: '>',
        }
    }
}

impl KeyBindings {
    fn from_config(config: Option<TuiKeyBindingsConfig>) -> Self {
        let mut key_bindings = Self::default();
        if let Some(config) = config {
            apply_key_override(&mut key_bindings.quit, config.quit);
            apply_key_override(&mut key_bindings.new_issue, config.new_issue);
            apply_key_override(&mut key_bindings.move_issue, config.move_issue);
            apply_key_override(&mut key_bindings.launch_opencode, config.launch_opencode);
            apply_key_override(&mut key_bindings.launch_claude, config.launch_claude);
            apply_key_override(&mut key_bindings.launch_shell, config.launch_shell);
            apply_key_override(&mut key_bindings.send_input, config.send_input);
            apply_key_override(&mut key_bindings.set_project_path, config.set_project_path);
            apply_key_override(&mut key_bindings.set_issue_cwd, config.set_issue_cwd);
            apply_key_override(&mut key_bindings.close_session, config.close_session);
            apply_key_override(&mut key_bindings.delete_issue, config.delete_issue);
            apply_key_override(&mut key_bindings.refresh_output, config.refresh_output);
            apply_key_override(&mut key_bindings.resize_left, config.resize_left);
            apply_key_override(&mut key_bindings.resize_right, config.resize_right);
        }
        key_bindings
    }
}

pub struct BoardPocApp {
    pub api: ApiService,
    session_manager: SessionManager,
    render_rx: Receiver<RenderRequest>,
    screen_store: ScreenStore,
    pub selected_issue_id: Option<String>,
    pub selected_project_id: Option<String>,
    pub state_path: PathBuf,
    pub opencode_cmd: String,
    pub claude_cmd: String,
    pub app_cwd: PathBuf,
    pub runtime_cwd_override: Option<PathBuf>,
    input_mode: InputMode,
    left_subview: LeftSubview,
    right_pane_mode: RightPaneMode,
    issues_filter: Option<String>,
    key_bindings: KeyBindings,
    status_line: String,
    board_ratio_percent: u16,
    divider_dragging: bool,
    form_scroll: usize,
    theme_mode: ThemeMode,
}

impl BoardPocApp {
    #[cfg(test)]
    pub fn new(
        state_path: Option<PathBuf>,
        opencode_cmd: Option<String>,
        claude_cmd: Option<String>,
        runtime_cwd_override: Option<PathBuf>,
    ) -> Self {
        Self::new_with_key_bindings(
            state_path,
            opencode_cmd,
            claude_cmd,
            runtime_cwd_override,
            None,
        )
    }

    pub fn new_with_key_bindings(
        state_path: Option<PathBuf>,
        opencode_cmd: Option<String>,
        claude_cmd: Option<String>,
        runtime_cwd_override: Option<PathBuf>,
        key_bindings: Option<TuiKeyBindingsConfig>,
    ) -> Self {
        let state_path = state_path.unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_FILE));
        let api = load_api_from_path(&state_path).unwrap_or_default();
        let board_ratio_percent = load_board_ratio_percent(&state_path);
        let app_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let opencode_cmd = opencode_cmd
            .or_else(|| std::env::var("OPENCODE_CMD").ok())
            .unwrap_or_else(|| "opencode".to_string());
        let claude_cmd = claude_cmd
            .or_else(|| std::env::var("CLAUDE_CMD").ok())
            .unwrap_or_else(|| "claude".to_string());
        let initial_status = if let Some(cwd_override) = runtime_cwd_override.as_ref() {
            format!(
                "Ready. app_cwd={} runtime_cwd_override={} | mouse: action bar + issue select | Enter launch/focus",
                app_cwd.display(),
                cwd_override.display()
            )
        } else {
            format!(
                "Ready. app_cwd={} | mouse: action bar + issue select | Enter launch/focus",
                app_cwd.display()
            )
        };

        let pty_config = PtyConfig {
            read_buffer_size: 65536,
            channel_size: 50,
        };
        let session_manager =
            SessionManager::with_config(runtime_cwd_override.clone(), 120, 40, pty_config);
        let render_rx = session_manager.render_receiver();
        let screen_store = session_manager.screen_store();

        Self {
            api,
            session_manager,
            render_rx,
            screen_store,
            selected_issue_id: None,
            selected_project_id: None,
            state_path,
            opencode_cmd,
            claude_cmd,
            app_cwd,
            runtime_cwd_override,
            input_mode: InputMode::Normal,
            left_subview: LeftSubview::Issues,
            right_pane_mode: RightPaneMode::Session,
            issues_filter: None,
            key_bindings: KeyBindings::from_config(key_bindings),
            status_line: initial_status,
            board_ratio_percent,
            divider_dragging: false,
            form_scroll: 0,
            theme_mode: theme_mode_from_env(),
        }
    }

    pub fn run(&mut self) -> Result<(), String> {
        enable_raw_mode().map_err(|err| format!("enable raw mode failed: {err}"))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide, EnableMouseCapture)
            .map_err(|err| format!("enter alt screen failed: {err}"))?;

        let run_result = self.event_loop(&mut stdout);

        self.shutdown_live_sessions();
        let _ = execute!(stdout, DisableMouseCapture, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();

        run_result
    }

    fn event_loop(&mut self, stdout: &mut Stdout) -> Result<(), String> {
        let mut should_quit = false;
        let mut needs_render = true;

        while !should_quit {
            self.ensure_selection();

            while self.render_rx.try_recv().is_ok() {
                needs_render = true;
            }

            if needs_render {
                self.render(stdout)?;
                needs_render = false;
            }

            if event::poll(Duration::from_millis(16))
                .map_err(|err| format!("event poll failed: {err}"))?
            {
                let ev = event::read().map_err(|err| format!("event read failed: {err}"))?;
                match ev {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Release {
                            continue;
                        }
                        should_quit = self.handle_key(key)?;
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse(mouse)?;
                    }
                    _ => {}
                }
                needs_render = true;
            }
        }

        self.persist()?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool, String> {
        if let InputMode::SelectIssueProject {
            title,
            description,
            query,
            selected_index,
        } = self.input_mode.clone()
        {
            return self.handle_select_issue_project_key(
                key,
                title,
                description,
                query,
                selected_index,
            );
        }

        match &mut self.input_mode {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Terminal => self.handle_terminal_key(key),
            InputMode::NewIssue(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled issue creation".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let title = buf.trim().to_string();
                    if title.is_empty() {
                        self.status_line = "Issue title cannot be empty".to_string();
                    } else {
                        self.input_mode = InputMode::NewIssueDescription {
                            title,
                            description: String::new(),
                            active_field: FormField::Description,
                        };
                        self.form_scroll = 0;
                        self.status_line =
                            "Issue form: Tab switches fields, Enter continues, Ctrl-E edits description in $EDITOR"
                                .to_string();
                        return Ok(false);
                    }
                    self.input_mode = InputMode::Normal;
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::NewIssueDescription {
                title,
                description,
                active_field,
            } => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled issue creation".to_string();
                    Ok(false)
                }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match open_external_editor(description, "Issue description") {
                        Ok(edited) => {
                            *description = edited;
                            self.status_line = "Description updated from $EDITOR".to_string();
                        }
                        Err(err) => {
                            self.status_line = format!("Editor cancelled: {err}");
                        }
                    }
                    Ok(false)
                }
                KeyCode::Tab => {
                    *active_field = match *active_field {
                        FormField::Title => FormField::Description,
                        FormField::Description => FormField::Title,
                    };
                    self.status_line =
                        "Issue form: Tab switches fields, Enter continues".to_string();
                    Ok(false)
                }
                KeyCode::BackTab => {
                    *active_field = match *active_field {
                        FormField::Title => FormField::Description,
                        FormField::Description => FormField::Title,
                    };
                    self.status_line =
                        "Issue form: Tab switches fields, Enter continues".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let title_value = title.clone();
                    let description_value = description.trim().to_string();
                    if title_value.trim().is_empty() {
                        self.status_line = "Issue title cannot be empty".to_string();
                        return Ok(false);
                    }
                    self.input_mode = InputMode::SelectIssueProject {
                        title: title_value,
                        description: description_value,
                        query: String::new(),
                        selected_index: 0,
                    };
                    self.status_line =
                        "Select project: type to filter, j/k to choose, Enter to create"
                            .to_string();
                    Ok(false)
                }
                KeyCode::Backspace => {
                    match *active_field {
                        FormField::Title => {
                            title.pop();
                        }
                        FormField::Description => {
                            self.status_line =
                                "Description uses external editor. Press Ctrl-E".to_string();
                        }
                    }
                    Ok(false)
                }
                KeyCode::Up => {
                    self.scroll_form(-1);
                    Ok(false)
                }
                KeyCode::Down => {
                    self.scroll_form(1);
                    Ok(false)
                }
                KeyCode::PageUp => {
                    self.scroll_form(-8);
                    Ok(false)
                }
                KeyCode::PageDown => {
                    self.scroll_form(8);
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    match *active_field {
                        FormField::Title => {
                            title.push(ch);
                        }
                        FormField::Description => {
                            self.status_line =
                                "Description uses external editor. Press Ctrl-E".to_string();
                        }
                    }
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::SelectIssueProject { .. } => Ok(false),
            InputMode::EditIssue {
                title,
                description,
                active_field,
            } => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled issue edit".to_string();
                    Ok(false)
                }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match open_external_editor(description, "Issue description") {
                        Ok(edited) => {
                            *description = edited;
                            self.status_line = "Description updated from $EDITOR".to_string();
                        }
                        Err(err) => {
                            self.status_line = format!("Editor cancelled: {err}");
                        }
                    }
                    Ok(false)
                }
                KeyCode::Tab => {
                    *active_field = match *active_field {
                        FormField::Title => FormField::Description,
                        FormField::Description => FormField::Title,
                    };
                    self.status_line = "Edit form: Tab switches fields, Enter saves".to_string();
                    Ok(false)
                }
                KeyCode::BackTab => {
                    *active_field = match *active_field {
                        FormField::Title => FormField::Description,
                        FormField::Description => FormField::Title,
                    };
                    self.status_line = "Edit form: Tab switches fields, Enter saves".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let title_value = title.trim().to_string();
                    if title_value.is_empty() {
                        self.status_line = "Issue title cannot be empty".to_string();
                        return Ok(false);
                    }
                    let description_value = if description.trim().is_empty() {
                        None
                    } else {
                        Some(description.trim().to_string())
                    };
                    self.input_mode = InputMode::Normal;
                    self.edit_selected_issue(&title_value, description_value.as_deref())?;
                    Ok(false)
                }
                KeyCode::Backspace => {
                    match *active_field {
                        FormField::Title => {
                            title.pop();
                        }
                        FormField::Description => {
                            self.status_line =
                                "Description uses external editor. Press Ctrl-E".to_string();
                        }
                    }
                    Ok(false)
                }
                KeyCode::Up => {
                    self.scroll_form(-1);
                    Ok(false)
                }
                KeyCode::Down => {
                    self.scroll_form(1);
                    Ok(false)
                }
                KeyCode::PageUp => {
                    self.scroll_form(-8);
                    Ok(false)
                }
                KeyCode::PageDown => {
                    self.scroll_form(8);
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    match *active_field {
                        FormField::Title => {
                            title.push(ch);
                        }
                        FormField::Description => {
                            self.status_line =
                                "Description uses external editor. Press Ctrl-E".to_string();
                        }
                    }
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::NewProject(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled project creation".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let raw = buf.trim().to_string();
                    self.input_mode = InputMode::Normal;
                    if raw.is_empty() {
                        self.status_line = "Project input cannot be empty".to_string();
                        return Ok(false);
                    }
                    let (key, name) = parse_new_project_input(&raw);
                    if name.is_empty() {
                        self.status_line =
                            "Use: <KEY> | <NAME> (example: DEV | Development)".to_string();
                        return Ok(false);
                    }
                    if !is_valid_project_key(&key) {
                        self.status_line =
                            "Project key must be 2-8 chars, start letter, A-Z0-9 only".to_string();
                        return Ok(false);
                    }
                    if self.api.project_find_by_identifier(&key).is_some() {
                        self.status_line = format!("Project key '{}' already exists", key);
                        return Ok(false);
                    }
                    let created = self.api.project_create(&name);
                    match self.api.project_set_identifier(&created.id, &key) {
                        Ok(updated) => {
                            self.selected_project_id = Some(updated.id.clone());
                            self.left_subview = LeftSubview::Projects;
                            self.status_line = format!(
                                "Created project {} ({})",
                                project_display_label(&updated),
                                updated.name
                            );
                            self.persist()?;
                        }
                        Err(err) => {
                            self.status_line = format!("Failed creating project: {err}");
                        }
                    }
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::AddComment(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled comment".to_string();
                    Ok(false)
                }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match open_external_editor(buf, "Comment") {
                        Ok(edited) => {
                            *buf = edited;
                            self.status_line = "Comment updated from $EDITOR".to_string();
                        }
                        Err(err) => {
                            self.status_line = format!("Editor cancelled: {err}");
                        }
                    }
                    Ok(false)
                }
                KeyCode::Enter => {
                    let mut body = buf.trim().to_string();
                    if body.is_empty()
                        && let Ok(edited) = open_external_editor("", "Comment")
                    {
                        body = edited.trim().to_string();
                    }
                    if body.is_empty() {
                        self.status_line =
                            "Comment cannot be empty; use Ctrl-E for multiline".to_string();
                        return Ok(false);
                    }
                    self.input_mode = InputMode::Normal;
                    self.add_comment_to_selection(&body)?;
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::SendInput(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled send".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let text = buf.trim().to_string();
                    self.input_mode = InputMode::Normal;
                    if text.is_empty() {
                        self.status_line = "Nothing sent".to_string();
                        return Ok(false);
                    }
                    self.send_to_selected(&text)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::Command(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled command mode".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let command = buf.trim().to_string();
                    self.input_mode = InputMode::Normal;
                    self.execute_command(&command);
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::Filter(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled filter".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let filter = normalize_optional_path(buf);
                    self.input_mode = InputMode::Normal;
                    self.issues_filter = filter;
                    self.ensure_selection();
                    self.status_line = if let Some(filter) = self.issues_filter.as_deref() {
                        format!("Issue filter set: {filter}")
                    } else {
                        "Issue filter cleared".to_string()
                    };
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::MoveStatus => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled move".to_string();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    if let Some(index) = ch.to_digit(10) {
                        let idx = index as usize;
                        if (1..=STATUSES.len()).contains(&idx) {
                            let status = STATUSES[idx - 1];
                            self.move_selected(status)?;
                            self.input_mode = InputMode::Normal;
                        }
                    }
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::DeleteIssueConfirm => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.input_mode = InputMode::Normal;
                    self.delete_selected_issue()?;
                    Ok(false)
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled issue deletion".to_string();
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::SetProjectIdentifier(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled project key update".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let key = buf.trim().to_ascii_uppercase();
                    self.input_mode = InputMode::Normal;
                    if !is_valid_project_key(&key) {
                        self.status_line =
                            "Project key must be 2-8 chars, start letter, A-Z0-9 only".to_string();
                        return Ok(false);
                    }
                    let project_id = match self.selected_project_id.clone() {
                        Some(id) => id,
                        None => {
                            self.status_line = "No project selected".to_string();
                            return Ok(false);
                        }
                    };
                    match self.api.project_set_identifier(&project_id, &key) {
                        Ok(project) => {
                            self.status_line = format!(
                                "Updated project key to {}",
                                project_display_label(&project)
                            );
                            self.persist()?;
                        }
                        Err(err) => {
                            self.status_line = format!("Failed updating project key: {err}");
                        }
                    }
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::SetProjectRepoPath(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled project repo path update".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let path = normalize_optional_path(buf);
                    self.input_mode = InputMode::Normal;
                    self.set_selected_issue_project_repo_path(path)?;
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
            InputMode::SetIssueCwdOverride(buf) => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.status_line = "Cancelled issue cwd override update".to_string();
                    Ok(false)
                }
                KeyCode::Enter => {
                    let path = normalize_optional_path(buf);
                    self.input_mode = InputMode::Normal;
                    self.set_selected_issue_cwd_override(path)?;
                    Ok(false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Ok(false)
                }
                KeyCode::Char(ch) => {
                    buf.push(ch);
                    Ok(false)
                }
                _ => Ok(false),
            },
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<bool, String> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.left_subview == LeftSubview::Issues {
                    self.select_next();
                } else {
                    self.select_next_project();
                }
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.left_subview == LeftSubview::Issues {
                    self.select_prev();
                } else {
                    self.select_prev_project();
                }
                Ok(false)
            }
            KeyCode::Enter => {
                if self.left_subview != LeftSubview::Issues {
                    self.status_line =
                        "Enter launches sessions only in issues subview; press Tab to switch"
                            .to_string();
                } else if let Err(err) = self.launch_or_focus_selected_default() {
                    self.status_line = err;
                }
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.quit) => Ok(true),
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.new_issue) => {
                if self.left_subview == LeftSubview::Issues {
                    self.input_mode = InputMode::NewIssue(String::new());
                    self.form_scroll = 0;
                    self.status_line = "Enter issue title, then Enter".to_string();
                } else {
                    self.input_mode = InputMode::NewProject(String::new());
                    self.form_scroll = 0;
                    self.status_line =
                        "Enter project as: <KEY> | <NAME> (example: DEV | Development)".to_string();
                }
                Ok(false)
            }
            KeyCode::Char('K') => {
                if self.left_subview == LeftSubview::Projects {
                    self.ensure_project_selection();
                    let current = self
                        .selected_project_id
                        .as_deref()
                        .and_then(|project_id| self.api.project_get(project_id).ok())
                        .map(|project| project.identifier)
                        .unwrap_or_default();
                    self.input_mode = InputMode::SetProjectIdentifier(current);
                    self.status_line =
                        "Set project key (2-8 chars, A-Z0-9, starts with letter)".to_string();
                }
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.move_issue) => {
                self.input_mode = InputMode::MoveStatus;
                self.status_line =
                    "Move issue: press 1=backlog 2=ready 3=in_progress 4=review 5=done 6=blocked"
                        .to_string();
                Ok(false)
            }
            KeyCode::Char('e') => {
                if self.left_subview == LeftSubview::Issues
                    && let Ok(issue) = self.selected_issue()
                {
                    let description = self
                        .latest_issue_comment_markdown(&issue.id)
                        .unwrap_or_default();
                    self.input_mode = InputMode::EditIssue {
                        title: issue.title,
                        description,
                        active_field: FormField::Title,
                    };
                    self.form_scroll = 0;
                    self.status_line =
                        "Edit issue form: Tab switches fields, Enter saves, Ctrl-E edits description in $EDITOR"
                            .to_string();
                }
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.launch_opencode) => {
                if self.left_subview != LeftSubview::Issues {
                    self.status_line = "Launch actions require issues subview".to_string();
                } else if let Err(err) = self.launch_selected("opencode") {
                    self.status_line = err;
                }
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.launch_claude) => {
                if self.left_subview != LeftSubview::Issues {
                    self.status_line = "Launch actions require issues subview".to_string();
                } else if let Err(err) = self.launch_selected("claude") {
                    self.status_line = err;
                }
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.launch_shell) => {
                if self.left_subview != LeftSubview::Issues {
                    self.status_line = "Launch actions require issues subview".to_string();
                } else if let Err(err) = self.launch_selected("shell") {
                    self.status_line = err;
                }
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.send_input) => {
                self.input_mode = InputMode::SendInput(String::new());
                self.status_line = "Type input for selected session".to_string();
                Ok(false)
            }
            KeyCode::Char('a') => {
                self.input_mode = InputMode::AddComment(String::new());
                self.status_line =
                    "Add comment: Ctrl-E opens editor, Enter saves, Esc cancels".to_string();
                Ok(false)
            }
            KeyCode::Char('l') => {
                self.show_recent_comments(3);
                Ok(false)
            }
            KeyCode::Char('t') => {
                self.toggle_right_pane_mode();
                Ok(false)
            }
            KeyCode::Char(':') => {
                self.input_mode = InputMode::Command(String::new());
                self.status_line = "Command mode: issues | projects | clear-filter | filter <text> | project-new <KEY> <NAME> | project-key <PROJECT> <KEY> | project-assign <PROJECT> | theme <terminal|ocean|mono>".to_string();
                Ok(false)
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Filter(String::new());
                self.status_line = "Issue filter: Enter to apply, empty clears".to_string();
                Ok(false)
            }
            KeyCode::Tab => {
                self.switch_left_subview(1);
                Ok(false)
            }
            KeyCode::BackTab => {
                self.switch_left_subview(-1);
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.set_project_path) => {
                let current = self.initial_project_repo_path_input();
                self.input_mode = InputMode::SetProjectRepoPath(current);
                self.status_line =
                    "Set project repo path (absolute). Enter saves, empty clears".to_string();
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.set_issue_cwd) => {
                let current = self.initial_issue_cwd_override_input();
                self.input_mode = InputMode::SetIssueCwdOverride(current);
                self.status_line =
                    "Set issue cwd override (absolute). Enter saves, empty clears".to_string();
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.close_session) => {
                self.close_selected_session()?;
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.delete_issue) => {
                self.input_mode = InputMode::DeleteIssueConfirm;
                self.status_line = "Delete selected issue and linked session? (y/N)".to_string();
                Ok(false)
            }
            KeyCode::Char(ch) if key_matches(ch, self.key_bindings.refresh_output) => Ok(false),
            KeyCode::Char(ch) if ch == self.key_bindings.resize_left => {
                self.shift_divider(-3)?;
                Ok(false)
            }
            KeyCode::Char(ch) if ch == self.key_bindings.resize_right => {
                self.shift_divider(3)?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<(), String> {
        let (w, h) = size().map_err(|err| format!("terminal size failed: {err}"))?;
        let divider_col = self.compute_board_width(w);

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.handle_mouse_action_click(mouse.column, mouse.row)? {
                    return Ok(());
                }

                if self.left_subview == LeftSubview::Issues
                    && let Some(issue_id) = self.issue_id_at_row(mouse.row, h)
                {
                    self.selected_issue_id = Some(issue_id);
                    return Ok(());
                }
                if self.left_subview == LeftSubview::Projects
                    && let Some(project_id) = self.project_id_at_row(mouse.row, h)
                {
                    self.selected_project_id = Some(project_id);
                    return Ok(());
                }

                if mouse.column == divider_col
                    || mouse.column == divider_col.saturating_sub(1)
                    || mouse.column == divider_col.saturating_add(1)
                {
                    self.divider_dragging = true;
                    self.set_board_ratio_from_col(mouse.column, w)?;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.divider_dragging {
                    self.set_board_ratio_from_col(mouse.column, w)?;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.divider_dragging {
                    self.divider_dragging = false;
                    self.status_line = format!(
                        "Pane resize saved: board={}%, session={}%; use </> for keyboard resize",
                        self.board_ratio_percent,
                        100_u16.saturating_sub(self.board_ratio_percent)
                    );
                    self.persist_ui_settings()?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_mouse_action_click(&mut self, column: u16, row: u16) -> Result<bool, String> {
        if row != ACTION_BAR_Y {
            return Ok(false);
        }

        for (start, end, action) in self.action_button_regions() {
            if (start..=end).contains(&column) {
                match action {
                    MouseAction::ViewIssues => {
                        self.left_subview = LeftSubview::Issues;
                        self.status_line =
                            "Subview: issues (Tab to switch, / to filter, : for commands)"
                                .to_string();
                    }
                    MouseAction::ViewProjects => {
                        self.left_subview = LeftSubview::Projects;
                        self.ensure_project_selection();
                        self.status_line =
                            "Subview: projects (Tab to switch, : for commands)".to_string();
                    }
                    MouseAction::NewIssue => {
                        self.input_mode = InputMode::NewIssue(String::new());
                        self.form_scroll = 0;
                        self.status_line = "Enter issue title, then Enter".to_string();
                    }
                    MouseAction::NewProject => {
                        self.input_mode = InputMode::NewProject(String::new());
                        self.form_scroll = 0;
                        self.status_line =
                            "Enter project as: <KEY> | <NAME> (example: DEV | Development)"
                                .to_string();
                    }
                    MouseAction::SetProjectKey => {
                        self.ensure_project_selection();
                        if let Some(project_id) = self.selected_project_id.as_deref() {
                            let current = self
                                .api
                                .project_get(project_id)
                                .map(|project| project.identifier)
                                .unwrap_or_default();
                            self.input_mode = InputMode::SetProjectIdentifier(current);
                            self.status_line =
                                "Set project key (2-8 chars, A-Z0-9, starts with letter)"
                                    .to_string();
                        } else {
                            self.status_line = "No project available to edit key".to_string();
                        }
                    }
                    MouseAction::AddComment => {
                        self.input_mode = InputMode::AddComment(String::new());
                        self.status_line =
                            "Add comment: Ctrl-E opens editor, Enter saves, Esc cancels"
                                .to_string();
                    }
                    MouseAction::EditIssue => {
                        if self.left_subview == LeftSubview::Issues
                            && let Ok(issue) = self.selected_issue()
                        {
                            let description = self
                                .latest_issue_comment_markdown(&issue.id)
                                .unwrap_or_default();
                            self.input_mode = InputMode::EditIssue {
                                title: issue.title,
                                description,
                                active_field: FormField::Title,
                            };
                            self.form_scroll = 0;
                            self.status_line =
                                "Edit issue form: Tab switches fields, Enter saves, Ctrl-E edits description in $EDITOR"
                                    .to_string();
                        }
                    }
                    MouseAction::LaunchOpenCode => {
                        if self.left_subview != LeftSubview::Issues {
                            self.status_line = "Launch actions require issues subview".to_string();
                        } else if let Err(err) = self.launch_selected("opencode") {
                            self.status_line = err;
                        }
                    }
                    MouseAction::LaunchClaude => {
                        if self.left_subview != LeftSubview::Issues {
                            self.status_line = "Launch actions require issues subview".to_string();
                        } else if let Err(err) = self.launch_selected("claude") {
                            self.status_line = err;
                        }
                    }
                    MouseAction::LaunchShell => {
                        if self.left_subview != LeftSubview::Issues {
                            self.status_line = "Launch actions require issues subview".to_string();
                        } else if let Err(err) = self.launch_selected("shell") {
                            self.status_line = err;
                        }
                    }
                    MouseAction::SetProjectPath => {
                        let current = self.initial_project_repo_path_input();
                        self.input_mode = InputMode::SetProjectRepoPath(current);
                        self.status_line =
                            "Set project repo path (absolute). Enter saves, empty clears"
                                .to_string();
                    }
                    MouseAction::SetIssueCwd => {
                        let current = self.initial_issue_cwd_override_input();
                        self.input_mode = InputMode::SetIssueCwdOverride(current);
                        self.status_line =
                            "Set issue cwd override (absolute). Enter saves, empty clears"
                                .to_string();
                    }
                    MouseAction::CloseSession => {
                        if let Err(err) = self.close_selected_session() {
                            self.status_line = err;
                        }
                    }
                    MouseAction::DeleteIssue => {
                        self.input_mode = InputMode::DeleteIssueConfirm;
                        self.status_line =
                            "Delete selected issue and linked session? (y/N)".to_string();
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn handle_terminal_key(&mut self, key: KeyEvent) -> Result<bool, String> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
            self.input_mode = InputMode::Normal;
            self.status_line = "Detached from terminal pane (normal mode)".to_string();
            return Ok(false);
        }

        self.forward_key_to_selected_session(key)?;
        Ok(false)
    }

    fn issues_sorted(&self) -> Vec<IssueRecord> {
        let mut issues = self.api.issue_list();
        if let Some(filter) = self.issues_filter.as_deref() {
            let query = filter.to_ascii_lowercase();
            issues.retain(|issue| {
                issue.title.to_ascii_lowercase().contains(&query)
                    || issue.id.to_ascii_lowercase().contains(&query)
                    || issue.status.to_ascii_lowercase().contains(&query)
            });
        }
        let order = |status: &str| -> usize {
            STATUSES
                .iter()
                .position(|s| *s == status)
                .unwrap_or(usize::MAX)
        };
        issues.sort_by(|a, b| {
            order(&a.status)
                .cmp(&order(&b.status))
                .then_with(|| a.id.cmp(&b.id))
        });
        issues
    }

    fn projects_sorted(&self) -> Vec<rpc_core::ProjectRecord> {
        let mut projects = self.api.project_list();
        projects.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        projects
    }

    fn switch_left_subview(&mut self, delta: i8) {
        self.left_subview = match (self.left_subview, delta.is_negative()) {
            (LeftSubview::Issues, false) => LeftSubview::Projects,
            (LeftSubview::Issues, true) => LeftSubview::Projects,
            (LeftSubview::Projects, false) => LeftSubview::Issues,
            (LeftSubview::Projects, true) => LeftSubview::Issues,
        };
        self.status_line = match self.left_subview {
            LeftSubview::Issues => {
                "Subview: issues (Tab to switch, / to filter, : for commands)".to_string()
            }
            LeftSubview::Projects => {
                self.ensure_project_selection();
                "Subview: projects (Tab to switch, : for commands)".to_string()
            }
        };
    }

    fn execute_command(&mut self, command: &str) {
        if command.is_empty() {
            self.status_line = "No command entered".to_string();
            return;
        }

        let tokens: Vec<&str> = command.split_whitespace().collect();

        if command.eq_ignore_ascii_case("issues") {
            self.left_subview = LeftSubview::Issues;
            self.status_line = "Subview switched: issues".to_string();
            return;
        }
        if command.eq_ignore_ascii_case("projects") {
            self.left_subview = LeftSubview::Projects;
            self.status_line = "Subview switched: projects".to_string();
            return;
        }
        if command.eq_ignore_ascii_case("clear-filter") {
            self.issues_filter = None;
            self.ensure_selection();
            self.status_line = "Issue filter cleared".to_string();
            return;
        }
        if let Some(raw_filter) = command.strip_prefix("filter ") {
            self.issues_filter = normalize_optional_path(raw_filter);
            self.ensure_selection();
            self.status_line = if let Some(filter) = self.issues_filter.as_deref() {
                format!("Issue filter set: {filter}")
            } else {
                "Issue filter cleared".to_string()
            };
            return;
        }

        if tokens.first() == Some(&"project-new") {
            if tokens.len() < 3 {
                self.status_line = "Usage: project-new <KEY> <NAME>".to_string();
                return;
            }
            let key = tokens[1];
            let name = tokens[2..].join(" ");
            let project = self.api.project_create(&name);
            match self.api.project_set_identifier(&project.id, key) {
                Ok(updated) => {
                    self.status_line = format!(
                        "Created project {} ({})",
                        project_display_label(&updated),
                        updated.name
                    );
                    let _ = self.persist();
                }
                Err(err) => {
                    self.status_line = format!("project-new failed: {err}");
                }
            }
            return;
        }

        if tokens.first() == Some(&"project-key") {
            if tokens.len() != 3 {
                self.status_line = "Usage: project-key <PROJECT> <KEY>".to_string();
                return;
            }
            let project_ref = tokens[1];
            let key = tokens[2];
            let Ok(project_id) = self.resolve_project_id_by_ref(project_ref) else {
                self.status_line = format!("project-key failed: unknown project '{project_ref}'");
                return;
            };
            match self.api.project_set_identifier(&project_id, key) {
                Ok(updated) => {
                    self.status_line =
                        format!("Updated project key to {}", project_display_label(&updated));
                    let _ = self.persist();
                }
                Err(err) => {
                    self.status_line = format!("project-key failed: {err}");
                }
            }
            return;
        }

        if tokens.first() == Some(&"project-assign") {
            if tokens.len() != 2 {
                self.status_line = "Usage: project-assign <PROJECT>".to_string();
                return;
            }
            let Some(issue_id) = self.selected_issue_id.clone() else {
                self.status_line = "project-assign failed: no issue selected".to_string();
                return;
            };
            let project_ref = tokens[1];
            let Ok(project_id) = self.resolve_project_id_by_ref(project_ref) else {
                self.status_line =
                    format!("project-assign failed: unknown project '{project_ref}'");
                return;
            };
            match self.api.issue_assign_project(&issue_id, &project_id) {
                Ok(updated_issue) => {
                    self.status_line = format!(
                        "Assigned issue {} to {}",
                        issue_display_label(&updated_issue),
                        project_ref.to_ascii_uppercase()
                    );
                    let _ = self.persist();
                }
                Err(err) => {
                    self.status_line = format!("project-assign failed: {err}");
                }
            }
            return;
        }

        if tokens.first() == Some(&"theme") {
            if tokens.len() != 2 {
                self.status_line = "Usage: theme <terminal|ocean|mono>".to_string();
                return;
            }
            match parse_theme_mode(tokens[1]) {
                Some(mode) => {
                    self.theme_mode = mode;
                    self.status_line = format!("Theme set to {}", theme_mode_label(mode));
                }
                None => {
                    self.status_line = "Unknown theme; use: terminal|ocean|mono".to_string();
                }
            }
            return;
        }

        self.status_line =
            "Unknown command; try: issues | projects | clear-filter | filter <text> | project-new <KEY> <NAME> | project-key <PROJECT> <KEY> | project-assign <PROJECT> | theme <terminal|ocean|mono>".to_string();
    }

    fn resolve_project_id_by_ref(&self, project_ref: &str) -> Result<String, String> {
        if self.api.project_get(project_ref).is_ok() {
            return Ok(project_ref.to_string());
        }
        if let Some(project) = self.api.project_find_by_identifier(project_ref) {
            return Ok(project.id);
        }
        let lower = project_ref.to_ascii_lowercase();
        if let Some(project) = self
            .api
            .project_list()
            .into_iter()
            .find(|project| project.name.to_ascii_lowercase() == lower)
        {
            return Ok(project.id);
        }
        Err(format!("unknown project: {project_ref}"))
    }

    fn filtered_projects_for_query(&self, query: &str) -> Vec<rpc_core::ProjectRecord> {
        let query = query.trim().to_ascii_lowercase();
        let mut projects = self.projects_sorted();
        if query.is_empty() {
            return projects;
        }
        projects.retain(|project| {
            project.identifier.to_ascii_lowercase().contains(&query)
                || project.name.to_ascii_lowercase().contains(&query)
        });
        projects
    }

    fn create_issue_from_form(
        &mut self,
        title: &str,
        description: &str,
        selected_project_id: Option<String>,
    ) -> Result<(), String> {
        let issue = self.api.issue_create(title);

        let project_assignment =
            selected_project_id.or_else(|| self.ensure_default_project_id().ok());
        if let Some(project_id) = project_assignment {
            let _ = self.api.issue_assign_project(&issue.id, &project_id);
        }

        if !description.trim().is_empty() {
            let author = default_comment_author();
            let _ = self
                .api
                .comment_add(CommentEntityType::Issue, &issue.id, description, &author)
                .map_err(|err| err.to_string())?;
        }

        self.selected_issue_id = Some(issue.id.clone());
        self.persist()?;
        self.status_line = format!("Created issue {}", self.issue_display_key(&issue.id));
        Ok(())
    }

    fn edit_selected_issue(
        &mut self,
        title: &str,
        description: Option<&str>,
    ) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "No issue selected".to_string())?;

        let updated = self
            .api
            .issue_update_title(&issue_id, title)
            .map_err(|err| err.to_string())?;

        if let Some(description) = description {
            let trimmed = description.trim();
            if !trimmed.is_empty() {
                let author = default_comment_author();
                self.api
                    .comment_add(CommentEntityType::Issue, &issue_id, trimmed, &author)
                    .map_err(|err| err.to_string())?;
            }
        }

        self.persist()?;
        self.status_line = format!("Updated issue {}", issue_display_label(&updated));
        Ok(())
    }

    fn handle_select_issue_project_key(
        &mut self,
        key: KeyEvent,
        title: String,
        description: String,
        query: String,
        selected_index: usize,
    ) -> Result<bool, String> {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.status_line = "Cancelled issue creation".to_string();
                Ok(false)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let options_len = self.filtered_projects_for_query(&query).len();
                let next_index = if options_len == 0 {
                    0
                } else {
                    (selected_index + 1) % options_len
                };
                self.input_mode = InputMode::SelectIssueProject {
                    title,
                    description,
                    query,
                    selected_index: next_index,
                };
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let options_len = self.filtered_projects_for_query(&query).len();
                let prev_index = if options_len == 0 {
                    0
                } else if selected_index == 0 {
                    options_len - 1
                } else {
                    selected_index - 1
                };
                self.input_mode = InputMode::SelectIssueProject {
                    title,
                    description,
                    query,
                    selected_index: prev_index,
                };
                Ok(false)
            }
            KeyCode::Enter => {
                let options = self.filtered_projects_for_query(&query);
                let selected_project_id = if options.is_empty() {
                    None
                } else {
                    Some(options[selected_index.min(options.len() - 1)].id.clone())
                };
                self.create_issue_from_form(&title, &description, selected_project_id)?;
                self.input_mode = InputMode::Normal;
                Ok(false)
            }
            KeyCode::Backspace => {
                let mut next_query = query;
                next_query.pop();
                self.input_mode = InputMode::SelectIssueProject {
                    title,
                    description,
                    query: next_query,
                    selected_index: 0,
                };
                Ok(false)
            }
            KeyCode::Char(ch) => {
                let mut next_query = query;
                next_query.push(ch);
                self.input_mode = InputMode::SelectIssueProject {
                    title,
                    description,
                    query: next_query,
                    selected_index: 0,
                };
                Ok(false)
            }
            _ => {
                self.input_mode = InputMode::SelectIssueProject {
                    title,
                    description,
                    query,
                    selected_index,
                };
                Ok(false)
            }
        }
    }

    fn visible_actions(&self) -> Vec<MouseAction> {
        match self.left_subview {
            LeftSubview::Issues => vec![
                MouseAction::ViewIssues,
                MouseAction::ViewProjects,
                MouseAction::NewIssue,
                MouseAction::EditIssue,
                MouseAction::AddComment,
                MouseAction::LaunchOpenCode,
                MouseAction::LaunchClaude,
                MouseAction::LaunchShell,
                MouseAction::SetProjectPath,
                MouseAction::SetIssueCwd,
                MouseAction::CloseSession,
                MouseAction::DeleteIssue,
            ],
            LeftSubview::Projects => vec![
                MouseAction::ViewIssues,
                MouseAction::ViewProjects,
                MouseAction::NewProject,
                MouseAction::SetProjectKey,
                MouseAction::AddComment,
                MouseAction::SetProjectPath,
            ],
        }
    }

    fn action_button_regions(&self) -> Vec<(u16, u16, MouseAction)> {
        let mut x = 0_u16;
        let mut regions = Vec::new();
        for action in self.visible_actions() {
            let label = mouse_action_label(action);
            let width = label.chars().count() as u16;
            let start = x;
            let end = x.saturating_add(width.saturating_sub(1));
            regions.push((start, end, action));
            x = end.saturating_add(2);
        }
        regions
    }

    fn issue_id_at_row(&self, row: u16, h: u16) -> Option<String> {
        let mut y = BOARD_START_Y;
        let issues = self.issues_sorted();
        for status in STATUSES {
            if y >= h.saturating_sub(3) {
                break;
            }
            if row == y {
                return None;
            }
            y = y.saturating_add(1);

            for issue in issues.iter().filter(|i| i.status == status) {
                if y >= h.saturating_sub(3) {
                    break;
                }
                if row == y {
                    return Some(issue.id.clone());
                }
                y = y.saturating_add(1);
            }
        }
        None
    }

    fn project_id_at_row(&self, row: u16, h: u16) -> Option<String> {
        let mut y = BOARD_START_Y.saturating_add(1);
        for project in self.projects_sorted() {
            if y >= h.saturating_sub(3) {
                break;
            }
            if row == y {
                return Some(project.id);
            }
            y = y.saturating_add(1);
        }
        None
    }

    fn ensure_default_project_id(&mut self) -> Result<String, String> {
        if let Some(existing) = self
            .api
            .project_list()
            .into_iter()
            .find(|project| project.name == DEFAULT_PROJECT_NAME)
        {
            return Ok(existing.id);
        }
        Ok(self.api.project_create(DEFAULT_PROJECT_NAME).id)
    }

    fn selected_issue(&self) -> Result<IssueRecord, String> {
        let issue_id = self
            .selected_issue_id
            .as_deref()
            .ok_or_else(|| "no issue selected".to_string())?;
        self.api.issue_get(issue_id).map_err(|err| err.to_string())
    }

    fn latest_issue_comment_markdown(&self, issue_id: &str) -> Option<String> {
        self.api
            .comment_list(
                CommentEntityType::Issue,
                issue_id,
                rpc_core::CommentListOrder::Desc,
                None,
                1,
            )
            .ok()
            .and_then(|page| page.items.into_iter().next())
            .map(|comment| comment.body_markdown)
    }

    fn issue_display_key(&self, issue_id: &str) -> String {
        self.api
            .issue_get(issue_id)
            .ok()
            .and_then(|issue| issue.identifier)
            .unwrap_or_else(|| short_id(issue_id))
    }

    fn selected_issue_project_repo_path(&self) -> Option<String> {
        let issue = self.selected_issue().ok()?;
        let project_id = issue.project_id?;
        self.api.project_get(&project_id).ok()?.repo_local_path
    }

    fn selected_issue_cwd_override(&self) -> Option<String> {
        self.selected_issue().ok()?.cwd_override_path
    }

    fn initial_project_repo_path_input(&self) -> String {
        self.selected_issue_project_repo_path()
            .unwrap_or_else(|| self.app_cwd.to_string_lossy().into_owned())
    }

    fn initial_issue_cwd_override_input(&self) -> String {
        self.selected_issue_cwd_override()
            .or_else(|| self.selected_issue_project_repo_path())
            .unwrap_or_else(|| self.app_cwd.to_string_lossy().into_owned())
    }

    fn ensure_selected_issue_project_id(&mut self) -> Result<String, String> {
        let issue = self.selected_issue()?;
        if let Some(project_id) = issue.project_id {
            return Ok(project_id);
        }
        let project_id = self.ensure_default_project_id()?;
        self.api
            .issue_assign_project(&issue.id, &project_id)
            .map_err(|err| err.to_string())?;
        Ok(project_id)
    }

    fn set_selected_issue_project_repo_path(&mut self, path: Option<String>) -> Result<(), String> {
        if let Some(ref raw_path) = path {
            let path_buf = PathBuf::from(raw_path);
            validate_launch_cwd(&path_buf, "project repo path")?;
        }
        let project_id = self.ensure_selected_issue_project_id()?;
        let updated = self
            .api
            .project_set_repo_local_path(&project_id, path)
            .map_err(|err| err.to_string())?;
        self.persist()?;
        self.status_line = if let Some(ref repo_path) = updated.repo_local_path {
            format!(
                "Updated project {} repo path to {}",
                project_display_label(&updated),
                repo_path
            )
        } else {
            format!(
                "Cleared project {} repo path",
                project_display_label(&updated)
            )
        };
        Ok(())
    }

    fn set_selected_issue_cwd_override(&mut self, path: Option<String>) -> Result<(), String> {
        if let Some(ref raw_path) = path {
            let path_buf = PathBuf::from(raw_path);
            validate_launch_cwd(&path_buf, "issue cwd override")?;
        }
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;
        let updated = self
            .api
            .issue_set_cwd_override(&issue_id, path)
            .map_err(|err| err.to_string())?;
        self.persist()?;
        self.status_line = if let Some(ref cwd_override) = updated.cwd_override_path {
            format!(
                "Updated issue {} cwd override to {}",
                issue_display_label(&updated),
                cwd_override
            )
        } else {
            format!(
                "Cleared issue {} cwd override",
                issue_display_label(&updated)
            )
        };
        Ok(())
    }

    fn ensure_selection(&mut self) {
        let issues = self.issues_sorted();
        if issues.is_empty() {
            self.selected_issue_id = None;
            return;
        }

        if let Some(current) = self.selected_issue_id.as_ref()
            && issues.iter().any(|i| &i.id == current)
        {
            return;
        }
        self.selected_issue_id = Some(issues[0].id.clone());
    }

    fn ensure_project_selection(&mut self) {
        let projects = self.projects_sorted();
        if projects.is_empty() {
            self.selected_project_id = None;
            return;
        }

        if let Some(current) = self.selected_project_id.as_ref()
            && projects.iter().any(|p| &p.id == current)
        {
            return;
        }

        self.selected_project_id = Some(projects[0].id.clone());
    }

    fn select_next(&mut self) {
        let issues = self.issues_sorted();
        if issues.is_empty() {
            return;
        }

        let next = match self.selected_issue_id.as_ref() {
            Some(current) => {
                let idx = issues.iter().position(|i| &i.id == current).unwrap_or(0);
                (idx + 1) % issues.len()
            }
            None => 0,
        };
        self.selected_issue_id = Some(issues[next].id.clone());
    }

    fn select_prev(&mut self) {
        let issues = self.issues_sorted();
        if issues.is_empty() {
            return;
        }

        let prev = match self.selected_issue_id.as_ref() {
            Some(current) => {
                let idx = issues.iter().position(|i| &i.id == current).unwrap_or(0);
                (idx + issues.len() - 1) % issues.len()
            }
            None => 0,
        };
        self.selected_issue_id = Some(issues[prev].id.clone());
    }

    fn select_next_project(&mut self) {
        let projects = self.projects_sorted();
        if projects.is_empty() {
            self.selected_project_id = None;
            return;
        }

        let next = match self.selected_project_id.as_ref() {
            Some(id) => projects
                .iter()
                .position(|project| &project.id == id)
                .map(|idx| (idx + 1) % projects.len())
                .unwrap_or(0),
            None => 0,
        };
        self.selected_project_id = Some(projects[next].id.clone());
    }

    fn select_prev_project(&mut self) {
        let projects = self.projects_sorted();
        if projects.is_empty() {
            self.selected_project_id = None;
            return;
        }

        let prev = match self.selected_project_id.as_ref() {
            Some(id) => projects
                .iter()
                .position(|project| &project.id == id)
                .map(|idx| {
                    if idx == 0 {
                        projects.len() - 1
                    } else {
                        idx - 1
                    }
                })
                .unwrap_or(0),
            None => 0,
        };
        self.selected_project_id = Some(projects[prev].id.clone());
    }

    fn move_selected(&mut self, status: &str) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;
        self.api
            .board_issue_move(&issue_id, status)
            .map_err(|err| err.to_string())?;
        self.persist()?;
        self.status_line = format!("Moved issue {} to {}", issue_id, status);
        Ok(())
    }

    fn launch_selected(&mut self, provider: &str) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;
        self.api
            .issue_get(&issue_id)
            .map_err(|err| err.to_string())?;
        let (resolved_cwd, cwd_source) = self.resolve_launch_cwd_for_issue(&issue_id)?;

        self.session_manager.set_workdir(Some(resolved_cwd.clone()));

        let session = self.api.session_create();
        self.api
            .session_set_status(&session.id, SessionState::Running)
            .map_err(|err| err.to_string())?;
        self.api
            .issue_update_status(&issue_id, "in_progress")
            .map_err(|err| err.to_string())?;
        self.api
            .issue_link_primary_session(&issue_id, &session.id)
            .map_err(|err| err.to_string())?;

        let result = if provider == "claude" {
            self.session_manager
                .spawn_session(&session.id, "/bin/sh", &["-lc", self.claude_cmd.as_str()])
                .map(|_| "claude")
        } else if provider == "shell" {
            let user_shell = resolve_user_shell();
            self.session_manager
                .spawn_session(&session.id, &user_shell, &[])
                .map(|_| "shell")
        } else {
            self.session_manager
                .spawn_session(&session.id, "/bin/sh", &["-lc", self.opencode_cmd.as_str()])
                .map(|_| "opencode")
        };

        match result {
            Ok(name) => {
                self.persist()?;
                self.input_mode = InputMode::Terminal;
                self.status_line = format!(
                    "Launched {name} for issue {} and attached terminal (Ctrl-G to detach) | cwd={} ({cwd_source})",
                    issue_id,
                    resolved_cwd.display()
                );
                Ok(())
            }
            Err(err) => {
                let _ = self
                    .api
                    .session_set_status(&session.id, SessionState::Failed)
                    .map_err(|e| e.to_string());
                self.persist()?;
                Err(format!("launch failed: {err}"))
            }
        }
    }

    fn resolve_launch_cwd_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<(PathBuf, &'static str), String> {
        if let Some(runtime_override) = self.runtime_cwd_override.as_ref() {
            validate_launch_cwd(runtime_override, "runtime cwd override")?;
            return Ok((runtime_override.clone(), "runtime_override"));
        }

        let issue = self
            .api
            .issue_get(issue_id)
            .map_err(|err| err.to_string())?;
        if let Some(issue_override) = issue.cwd_override_path.as_deref() {
            let path = PathBuf::from(issue_override);
            validate_launch_cwd(&path, "issue cwd override")?;
            return Ok((path, "issue_override"));
        }

        let project_id = issue.project_id.as_deref().ok_or_else(|| {
            "launch blocked: issue has no project and no cwd override; configure project repo_local_path"
                .to_string()
        })?;

        let project = self
            .api
            .project_get(project_id)
            .map_err(|err| err.to_string())?;
        let repo_local_path = project.repo_local_path.as_deref().ok_or_else(|| {
            format!(
                "launch blocked: project {} has no repo_local_path",
                project_display_label(&project)
            )
        })?;

        let path = PathBuf::from(repo_local_path);
        validate_launch_cwd(&path, "project repo_local_path")?;
        Ok((path, "project_repo"))
    }

    fn launch_or_focus_selected_default(&mut self) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;

        if let Some(session_id) = self
            .api
            .issue_primary_session(&issue_id)
            .map(ToString::to_string)
        {
            if self.session_is_live(&session_id) {
                self.input_mode = InputMode::Terminal;
                self.status_line = format!(
                    "Focused existing primary session {} for issue {} (Ctrl-G to detach)",
                    short_id(&session_id),
                    self.issue_display_key(&issue_id)
                );
            } else {
                self.status_line = format!(
                    "Issue has non-live session {}; press o to relaunch",
                    short_id(&session_id)
                );
            }
            return Ok(());
        }

        self.launch_selected("opencode")
    }

    fn send_to_selected(&mut self, text: &str) -> Result<bool, String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;
        let session_id = self
            .api
            .issue_primary_session(&issue_id)
            .ok_or_else(|| "selected issue has no active session".to_string())?
            .to_string();

        let payload = format!("{text}\n");
        if !self.session_manager.has_session(&session_id) {
            return Err(format!(
                "session {} is not live in this TUI process; relaunch from issue",
                short_id(&session_id)
            ));
        }
        self.session_manager
            .send_input(&session_id, &payload)
            .map_err(|err| err.to_string())?;
        self.status_line = format!("sent input to {}", session_id);
        Ok(false)
    }

    fn forward_key_to_selected_session(&mut self, key: KeyEvent) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;
        let session_id = self
            .api
            .issue_primary_session(&issue_id)
            .ok_or_else(|| "selected issue has no active session".to_string())?
            .to_string();

        if !self.session_is_live(&session_id) {
            return Err(format!(
                "session {} is not live in this TUI process; relaunch from issue",
                short_id(&session_id)
            ));
        }

        let payload = encode_key_input(key);
        if payload.is_empty() {
            return Ok(());
        }

        self.session_manager
            .send_bytes(&session_id, payload.as_bytes())
            .map_err(|err| err.to_string())
    }

    fn close_selected_session(&mut self) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;
        let Some(session_id) = self.close_primary_session_for_issue(&issue_id)? else {
            self.status_line = "Selected issue has no linked primary session".to_string();
            return Ok(());
        };

        self.status_line = format!(
            "Closed session {} for issue {}",
            short_id(&session_id),
            self.issue_display_key(&issue_id)
        );
        self.persist()?;
        Ok(())
    }

    fn delete_selected_issue(&mut self) -> Result<(), String> {
        let issue_id = self
            .selected_issue_id
            .clone()
            .ok_or_else(|| "no issue selected".to_string())?;

        let maybe_session = self.close_primary_session_for_issue(&issue_id)?;
        self.api
            .issue_delete(&issue_id)
            .map_err(|err| err.to_string())?;

        self.persist()?;
        self.ensure_selection();
        self.status_line = if let Some(session_id) = maybe_session {
            format!(
                "Deleted issue {} and closed session {}",
                self.issue_display_key(&issue_id),
                short_id(&session_id)
            )
        } else {
            format!("Deleted issue {}", self.issue_display_key(&issue_id))
        };
        Ok(())
    }

    fn close_primary_session_for_issue(
        &mut self,
        issue_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(session_id) = self.api.issue_primary_session(issue_id).map(str::to_string) else {
            return Ok(None);
        };

        if self.session_manager.has_session(&session_id) {
            self.session_manager
                .terminate(&session_id)
                .map_err(|err| err.to_string())?;
        }

        let _ = self
            .api
            .session_set_status(&session_id, SessionState::Terminated)
            .map_err(|err| err.to_string());
        self.api
            .issue_unlink_primary_session(issue_id)
            .map_err(|err| err.to_string())?;

        if matches!(self.input_mode, InputMode::Terminal) {
            self.input_mode = InputMode::Normal;
        }

        Ok(Some(session_id))
    }

    fn session_is_live(&self, session_id: &str) -> bool {
        self.session_manager.has_session(session_id)
    }

    fn active_theme_palette(&self) -> ThemePalette {
        let mode = match self.theme_mode {
            ThemeMode::Session => self.session_inherited_theme_mode(),
            explicit => explicit,
        };
        palette_for_theme_mode(mode)
    }

    fn session_inherited_theme_mode(&self) -> ThemeMode {
        let Some(issue_id) = self.selected_issue_id.as_deref() else {
            return ThemeMode::Mono;
        };
        let Some(session_id) = self.api.issue_primary_session(issue_id) else {
            return ThemeMode::Mono;
        };
        if self.session_manager.has_session(session_id) {
            ThemeMode::Ocean
        } else {
            ThemeMode::Mono
        }
    }

    fn shutdown_live_sessions(&mut self) {
        self.session_manager.terminate_all();
    }

    fn render(&mut self, stdout: &mut Stdout) -> Result<(), String> {
        let (w, h) = size().map_err(|err| format!("terminal size failed: {err}"))?;
        let board_w = self.compute_board_width(w);
        let right_x = board_w.saturating_add(1);
        let theme = self.active_theme_palette();

        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))
            .map_err(|err| format!("render clear failed: {err}"))?;

        {
            let issue_count = self.api.issue_list().len();
            let project_count = self.api.project_list().len();
            queue!(
                stdout,
                MoveTo(0, 0),
                SetBackgroundColor(theme.header_bg),
                SetForegroundColor(theme.accent),
                SetAttribute(Attribute::Bold),
                Print("DDAK"),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(theme.header_bg),
                SetForegroundColor(theme.dim),
                Print(" \u{2502} "),
                SetForegroundColor(theme.header_fg),
                Print(format!("{issue_count} issues")),
                SetForegroundColor(theme.dim),
                Print(" \u{2502} "),
                SetForegroundColor(theme.header_fg),
                Print(format!("{project_count} projects")),
                Print(" ".repeat(w.saturating_sub(
                    3 + 3
                        + issue_count.to_string().len() as u16
                        + 7
                        + 3
                        + project_count.to_string().len() as u16
                        + 9
                ) as usize)),
                SetAttribute(Attribute::Reset),
                ResetColor
            )
            .map_err(|err| format!("render header failed: {err}"))?;
        }

        // Top border with pane titles
        {
            let left_title = match self.left_subview {
                LeftSubview::Issues => "Board",
                LeftSubview::Projects => "Projects",
            };
            let right_title = match self.right_pane_mode {
                RightPaneMode::Session => "Session",
                RightPaneMode::Comments => "Comments",
            };
            let left_inner = board_w.saturating_sub(1) as usize;
            let right_inner = w.saturating_sub(board_w + 2) as usize;
            let mut top_line = String::with_capacity(w as usize);
            top_line.push('\u{256d}'); // ╭
            top_line.push_str(&border_fill(left_inner, left_title));
            top_line.push('\u{252c}'); // ┬
            top_line.push_str(&border_fill(right_inner, right_title));
            top_line.push('\u{256e}'); // ╮
            queue!(
                stdout,
                MoveTo(0, 2),
                SetForegroundColor(theme.dim),
                Print(top_line),
                ResetColor
            )
            .map_err(|err| format!("render top border failed: {err}"))?;
        }

        for (start, _, action) in self.action_button_regions() {
            let label = mouse_action_label(action);
            let is_active_view = (matches!(self.left_subview, LeftSubview::Issues)
                && matches!(action, MouseAction::ViewIssues))
                || (matches!(self.left_subview, LeftSubview::Projects)
                    && matches!(action, MouseAction::ViewProjects));
            let is_separator = matches!(
                action,
                MouseAction::NewIssue | MouseAction::NewProject | MouseAction::LaunchOpenCode
            );
            if is_separator && start > 0 {
                queue!(
                    stdout,
                    MoveTo(start.saturating_sub(2), ACTION_BAR_Y),
                    SetForegroundColor(theme.dim),
                    Print("\u{2502}"),
                    ResetColor
                )
                .map_err(|err| format!("render action separator failed: {err}"))?;
            }
            if is_active_view {
                queue!(
                    stdout,
                    MoveTo(start, ACTION_BAR_Y),
                    SetForegroundColor(theme.accent),
                    SetAttribute(Attribute::Bold),
                    Print(label),
                    SetAttribute(Attribute::Reset),
                    ResetColor
                )
                .map_err(|err| format!("render action bar failed: {err}"))?;
            } else {
                queue!(
                    stdout,
                    MoveTo(start, ACTION_BAR_Y),
                    SetForegroundColor(theme.dim),
                    Print(label),
                    ResetColor
                )
                .map_err(|err| format!("render action bar failed: {err}"))?;
            }
        }

        let mut y = BOARD_START_Y;
        match self.left_subview {
            LeftSubview::Issues => {
                let issues = self.issues_sorted();
                for status in STATUSES {
                    if y >= h.saturating_sub(3) {
                        break;
                    }
                    queue!(
                        stdout,
                        MoveTo(1, y),
                        SetForegroundColor(status_color(status, &theme)),
                        SetAttribute(Attribute::Bold),
                        Print(format!("[{status}]")),
                        SetAttribute(Attribute::Reset),
                        ResetColor
                    )
                    .map_err(|err| format!("render status failed: {err}"))?;
                    y = y.saturating_add(1);

                    for issue in issues.iter().filter(|i| i.status == status) {
                        if y >= h.saturating_sub(3) {
                            break;
                        }
                        let is_selected =
                            self.selected_issue_id.as_deref() == Some(issue.id.as_str());
                        let marker = if is_selected { "\u{203a}" } else { " " };
                        let linked = self
                            .api
                            .issue_primary_session(&issue.id)
                            .map(|session_id| {
                                if self.session_is_live(session_id) {
                                    " \u{25cf}"
                                } else {
                                    ""
                                }
                            })
                            .unwrap_or("");
                        let project_label = issue
                            .project_id
                            .as_deref()
                            .and_then(|project_id| self.api.project_get(project_id).ok())
                            .map(|project| project_display_label(&project))
                            .unwrap_or_else(|| "-".to_string());
                        let comment_count = self
                            .api
                            .comment_count_for(CommentEntityType::Issue, &issue.id);
                        let comment_suffix = if comment_count > 0 {
                            format!(" c{comment_count}")
                        } else {
                            String::new()
                        };
                        let line = format!(
                            "{marker} {} {project_label} {}{comment_suffix}{linked}",
                            issue_display_label(issue),
                            issue.title,
                        );
                        let trimmed = trim_to_width(&line, board_w.saturating_sub(2) as usize);
                        if is_selected {
                            queue!(
                                stdout,
                                MoveTo(1, y),
                                SetBackgroundColor(theme.selection_bg),
                                SetForegroundColor(theme.selection_fg),
                                Print(trimmed),
                                ResetColor
                            )
                            .map_err(|err| format!("render issue failed: {err}"))?;
                        } else {
                            queue!(stdout, MoveTo(1, y), Print(trimmed))
                                .map_err(|err| format!("render issue failed: {err}"))?;
                        }
                        y = y.saturating_add(1);
                    }
                }
            }
            LeftSubview::Projects => {
                queue!(
                    stdout,
                    MoveTo(1, y),
                    SetForegroundColor(Color::Cyan),
                    SetAttribute(Attribute::Bold),
                    Print("[projects]"),
                    SetAttribute(Attribute::Reset),
                    ResetColor
                )
                .map_err(|err| format!("render projects header failed: {err}"))?;
                y = y.saturating_add(1);

                let projects = self.projects_sorted();
                for project in projects {
                    if y >= h.saturating_sub(3) {
                        break;
                    }

                    let linked_issue_count = self
                        .api
                        .issue_list()
                        .into_iter()
                        .filter(|issue| issue.project_id.as_deref() == Some(project.id.as_str()))
                        .count();
                    let comment_count = self
                        .api
                        .comment_count_for(CommentEntityType::Project, &project.id);
                    let repo = project
                        .repo_local_path
                        .clone()
                        .unwrap_or_else(|| "<unset>".to_string());
                    let line = format!(
                        "{} {} [{} issues] [c{}] {}",
                        project_display_label(&project),
                        project.name,
                        linked_issue_count,
                        comment_count,
                        repo
                    );
                    let trimmed = trim_to_width(&line, board_w.saturating_sub(2) as usize);
                    if self.selected_project_id.as_deref() == Some(project.id.as_str()) {
                        queue!(
                            stdout,
                            MoveTo(1, y),
                            SetBackgroundColor(theme.selection_bg),
                            SetForegroundColor(theme.selection_fg),
                            Print(trimmed),
                            ResetColor
                        )
                        .map_err(|err| format!("render project row failed: {err}"))?;
                    } else {
                        queue!(stdout, MoveTo(1, y), Print(trimmed))
                            .map_err(|err| format!("render project row failed: {err}"))?;
                    }
                    y = y.saturating_add(1);
                }
            }
        }

        // Action bar divider
        queue!(
            stdout,
            MoveTo(board_w, ACTION_BAR_Y),
            SetForegroundColor(theme.dim),
            Print("\u{2502}"),
            ResetColor
        )
        .map_err(|err| format!("render action bar divider failed: {err}"))?;

        // Pane borders: left edge, divider, right edge for content rows
        {
            let divider_color = if self.divider_dragging {
                Color::Yellow
            } else {
                theme.dim
            };
            for dy in BOARD_START_Y..h.saturating_sub(3) {
                queue!(
                    stdout,
                    SetForegroundColor(theme.dim),
                    MoveTo(0, dy),
                    Print("\u{2502}"),
                    MoveTo(w.saturating_sub(1), dy),
                    Print("\u{2502}"),
                    SetForegroundColor(divider_color),
                    MoveTo(board_w, dy),
                    Print("\u{2502}"),
                    ResetColor
                )
                .map_err(|err| format!("render borders failed: {err}"))?;
            }
        }

        // Right pane (title is now in top border)
        let pane_w = w.saturating_sub(right_x + 1).max(1);
        for yy in BOARD_START_Y..h.saturating_sub(3) {
            queue!(
                stdout,
                MoveTo(right_x, yy),
                SetBackgroundColor(theme.pane_bg),
                Print(" ".repeat(pane_w as usize)),
                ResetColor
            )
            .map_err(|err| format!("render pane background failed: {err}"))?;
        }
        let pane_content_y = BOARD_START_Y;

        let output_h = h.saturating_sub(pane_content_y.saturating_add(3));
        let selected_session = self
            .selected_issue_id
            .clone()
            .and_then(|id| self.api.issue_primary_session(&id).map(str::to_string));

        let (output, using_formatted): (Vec<String>, bool) = match self.right_pane_mode {
            RightPaneMode::Session => {
                if let Some(session_id) = selected_session.as_deref() {
                    self.resize_session_to_pane(session_id, pane_w, output_h);
                }
                let screen_output = selected_session.clone().and_then(|sid| {
                    let store = self.screen_store.lock().ok()?;
                    let found = store.get(&sid);

                    found.map(|parser| {
                        let screen = parser.screen();
                        let (num_rows, _) = screen.size();
                        let mut lines = Vec::with_capacity(num_rows as usize);
                        for row in 0..num_rows {
                            let mut line = String::new();
                            let mut prev_fg: Option<vt100::Color> = None;
                            let mut prev_bg: Option<vt100::Color> = None;
                            let mut prev_bold = false;
                            let mut prev_underline = false;
                            let mut prev_inverse = false;
                            for col in 0..pane_w {
                                if let Some(cell) = screen.cell(row, col) {
                                    let fg = cell.fgcolor();
                                    let bg = cell.bgcolor();
                                    let bold = cell.bold();
                                    let underline = cell.underline();
                                    let inverse = cell.inverse();
                                    // Emit SGR only when attributes change
                                    if prev_fg.as_ref() != Some(&fg)
                                        || prev_bg.as_ref() != Some(&bg)
                                        || prev_bold != bold
                                        || prev_underline != underline
                                        || prev_inverse != inverse
                                    {
                                        line.push_str("\x1b[0m"); // reset
                                        // Background
                                        match bg {
                                            vt100::Color::Default => {} // handled by pane_bg clear
                                            vt100::Color::Idx(i) => {
                                                use std::fmt::Write;
                                                let _ = write!(line, "\x1b[48;5;{i}m");
                                            }
                                            vt100::Color::Rgb(r, g, b) => {
                                                use std::fmt::Write;
                                                let _ = write!(line, "\x1b[48;2;{r};{g};{b}m");
                                            }
                                        }
                                        // Foreground
                                        match fg {
                                            vt100::Color::Default => {}
                                            vt100::Color::Idx(i) => {
                                                use std::fmt::Write;
                                                let _ = write!(line, "\x1b[38;5;{i}m");
                                            }
                                            vt100::Color::Rgb(r, g, b) => {
                                                use std::fmt::Write;
                                                let _ = write!(line, "\x1b[38;2;{r};{g};{b}m");
                                            }
                                        }
                                        if bold {
                                            line.push_str("\x1b[1m");
                                        }
                                        if underline {
                                            line.push_str("\x1b[4m");
                                        }
                                        if inverse {
                                            line.push_str("\x1b[7m");
                                        }
                                        prev_fg = Some(fg);
                                        prev_bg = Some(bg);
                                        prev_bold = bold;
                                        prev_underline = underline;
                                        prev_inverse = inverse;
                                    }
                                    let contents = cell.contents();
                                    if contents.is_empty() {
                                        line.push(' ');
                                    } else {
                                        line.push_str(contents);
                                    }
                                } else {
                                    line.push(' ');
                                }
                            }
                            line.push_str("\x1b[0m");
                            lines.push(line);
                        }
                        lines
                    })
                });

                let output = screen_output.unwrap_or_default();
                (output, true)
            }
            RightPaneMode::Comments => {
                let lines = self.current_comment_lines(pane_w as usize, output_h as usize);
                (lines, false)
            }
        };

        let display_rows = output.len().min(output_h as usize);

        for (i, line) in output.iter().take(display_rows).enumerate() {
            let yy = pane_content_y.saturating_add(i as u16);
            if yy >= h.saturating_sub(3) {
                break;
            }
            let rendered = if using_formatted {
                line.clone()
            } else {
                trim_to_width(line, pane_w as usize)
            };

            if using_formatted {
                // Clear line with pane_bg, then overlay cell-by-cell content.
                // Cells with Default bg inherit the pane_bg from the fill.
                queue!(
                    stdout,
                    MoveTo(right_x, yy),
                    SetBackgroundColor(theme.pane_bg),
                    SetForegroundColor(Color::White),
                    Print(" ".repeat(pane_w as usize)),
                    MoveTo(right_x, yy),
                    Print(rendered),
                    ResetColor
                )
            } else {
                queue!(
                    stdout,
                    MoveTo(right_x, yy),
                    SetBackgroundColor(theme.pane_bg),
                    Print(rendered),
                    Print("\x1b[0m")
                )
            }
            .map_err(|err| format!("render output line failed: {err}"))?;
        }

        // Status + input line
        let mode_hint = match &self.input_mode {
            InputMode::Normal => match self.left_subview {
                LeftSubview::Issues => format!(
                    "[j/k] select  [Enter] focus  [{n}] new  [e] edit  [m] move  [/] filter  [o/c/{h}] launch  [{x}] close  [{q}] quit",
                    n = self.key_bindings.new_issue,
                    h = self.key_bindings.launch_shell,
                    x = self.key_bindings.close_session,
                    q = self.key_bindings.quit,
                ),
                LeftSubview::Projects => format!(
                    "[j/k] select  [{n}] new  [K] key  [{p}] path  [Tab] issues  [{q}] quit",
                    n = self.key_bindings.new_issue,
                    p = self.key_bindings.set_project_path,
                    q = self.key_bindings.quit,
                ),
            },
            InputMode::Terminal => {
                "Mode: terminal | typed keys go to session | Ctrl-G detach | q does not quit"
                    .to_string()
            }
            InputMode::NewIssue(buf) => format!("New issue title: {buf}"),
            InputMode::NewIssueDescription {
                title,
                description,
                active_field,
            } => {
                let field = match active_field {
                    FormField::Title => "title",
                    FormField::Description => "description",
                };
                format!(
                    "Issue form ({field}) title='{title}' description={}",
                    description.len()
                )
            }
            InputMode::SelectIssueProject {
                query,
                selected_index,
                ..
            } => format!(
                "Select project: query='{query}' selected={} (j/k to navigate, Enter confirm)",
                selected_index.saturating_add(1)
            ),
            InputMode::EditIssue {
                title,
                description,
                active_field,
            } => {
                let field = match active_field {
                    FormField::Title => "title",
                    FormField::Description => "description",
                };
                format!(
                    "Edit issue ({field}) title='{title}' description={}",
                    description.len()
                )
            }
            InputMode::NewProject(buf) => {
                format!("New project: {buf} (format: <KEY> <NAME>)")
            }
            InputMode::AddComment(buf) => format!("Add comment: {buf}"),
            InputMode::SendInput(buf) => format!("Send input: {buf}"),
            InputMode::Command(buf) => format!("Command: {buf}"),
            InputMode::Filter(buf) => format!("Issue filter: {buf}"),
            InputMode::MoveStatus => {
                "Move status: 1 backlog 2 ready 3 in_progress 4 review 5 done 6 blocked".to_string()
            }
            InputMode::DeleteIssueConfirm => "Confirm delete: y=yes n=no".to_string(),
            InputMode::SetProjectIdentifier(buf) => format!("Project key: {buf}"),
            InputMode::SetProjectRepoPath(buf) => format!("Project repo path: {buf}"),
            InputMode::SetIssueCwdOverride(buf) => format!("Issue cwd override: {buf}"),
        };
        // Bottom border with embedded context
        {
            let left_ctx = self.context_line();
            let left_ctx_trimmed =
                trim_to_width(&left_ctx, board_w.saturating_sub(4).max(1) as usize);
            let right_ctx = self.right_pane_context_line();
            let right_ctx_trimmed =
                trim_to_width(&right_ctx, w.saturating_sub(board_w + 5).max(1) as usize);
            let left_inner = board_w.saturating_sub(1) as usize;
            let right_inner = w.saturating_sub(board_w + 2) as usize;
            let mut bot_line = String::with_capacity(w as usize);
            bot_line.push('\u{2570}'); // ╰
            bot_line.push_str(&border_fill(left_inner, &left_ctx_trimmed));
            bot_line.push('\u{2534}'); // ┴
            bot_line.push_str(&border_fill(right_inner, &right_ctx_trimmed));
            bot_line.push('\u{256f}'); // ╯
            queue!(
                stdout,
                MoveTo(0, h.saturating_sub(3)),
                SetForegroundColor(theme.dim),
                Print(bot_line),
                ResetColor
            )
            .map_err(|err| format!("render bottom border failed: {err}"))?;
        }

        let rendered_field_form = self.draw_text_field_form_modal_ratatui(stdout, w, h)?;
        if !rendered_field_form && let Some((title, lines)) = self.current_form_modal() {
            self.draw_center_modal_ratatui(stdout, w, h, title, &lines)?;
        }

        queue!(
            stdout,
            MoveTo(0, h.saturating_sub(2)),
            SetBackgroundColor(theme.status_bar_bg),
            SetForegroundColor(theme.status_bar_fg),
            Print(trim_to_width(&self.status_line, w as usize)),
            Print(
                " ".repeat(
                    w.saturating_sub(self.status_line.len().min(w as usize) as u16) as usize
                )
            ),
            ResetColor
        )
        .map_err(|err| format!("render status line failed: {err}"))?;
        queue!(
            stdout,
            MoveTo(0, h.saturating_sub(1)),
            SetForegroundColor(theme.dim),
            Print(trim_to_width(&mode_hint, w as usize)),
            ResetColor
        )
        .map_err(|err| format!("render mode line failed: {err}"))?;

        stdout
            .flush()
            .map_err(|err| format!("stdout flush failed: {err}"))?;
        Ok(())
    }

    fn context_line(&self) -> String {
        match self.left_subview {
            LeftSubview::Issues => {
                let Some(issue_id) = self.selected_issue_id.as_deref() else {
                    return "No issue selected".to_string();
                };
                let Ok(issue) = self.api.issue_get(issue_id) else {
                    return "Selected issue is missing".to_string();
                };
                let project_label = issue
                    .project_id
                    .as_deref()
                    .and_then(|project_id| self.api.project_get(project_id).ok())
                    .map(|project| project_display_label(&project))
                    .unwrap_or_else(|| "-".to_string());
                let comment_count = self
                    .api
                    .comment_count_for(CommentEntityType::Issue, issue.id.as_str());
                let session_label = self
                    .api
                    .issue_primary_session(issue.id.as_str())
                    .map(|session_id| {
                        if self.session_is_live(session_id) {
                            format!("live:{}", short_id(session_id))
                        } else {
                            format!("stale:{}", short_id(session_id))
                        }
                    })
                    .unwrap_or_else(|| "none".to_string());
                format!(
                    "Issue {} \u{2502} status={} \u{2502} project={} \u{2502} comments={} \u{2502} session={}",
                    issue_display_label(&issue),
                    issue.status,
                    project_label,
                    comment_count,
                    session_label
                )
            }
            LeftSubview::Projects => {
                let Some(project_id) = self.selected_project_id.as_deref() else {
                    return "No project selected".to_string();
                };
                let Ok(project) = self.api.project_get(project_id) else {
                    return "Selected project is missing".to_string();
                };
                let issue_count = self
                    .api
                    .issue_list()
                    .into_iter()
                    .filter(|issue| issue.project_id.as_deref() == Some(project.id.as_str()))
                    .count();
                let comment_count = self
                    .api
                    .comment_count_for(CommentEntityType::Project, project.id.as_str());
                let repo = project
                    .repo_local_path
                    .clone()
                    .unwrap_or_else(|| "<unset>".to_string());
                format!(
                    "Project {} ({}) \u{2502} issues={} \u{2502} comments={} \u{2502} repo={}",
                    project_display_label(&project),
                    project.name,
                    issue_count,
                    comment_count,
                    repo
                )
            }
        }
    }

    fn right_pane_context_line(&self) -> String {
        let Some(issue_id) = self.selected_issue_id.as_deref() else {
            return "no session".to_string();
        };
        self.api
            .issue_primary_session(issue_id)
            .map(|session_id| {
                if self.session_is_live(session_id) {
                    format!("live:{}", short_id(session_id))
                } else {
                    format!("stale:{}", short_id(session_id))
                }
            })
            .unwrap_or_else(|| "no session".to_string())
    }

    fn add_comment_to_selection(&mut self, body_markdown: &str) -> Result<(), String> {
        let author = default_comment_author();
        match self.left_subview {
            LeftSubview::Issues => {
                let issue_id = self
                    .selected_issue_id
                    .clone()
                    .ok_or_else(|| "No issue selected".to_string())?;
                let issue_label = self.issue_display_key(&issue_id);
                self.api
                    .comment_add(CommentEntityType::Issue, &issue_id, body_markdown, &author)
                    .map_err(|err| err.to_string())?;
                self.persist()?;
                self.status_line = format!("Added comment to issue {issue_label}");
            }
            LeftSubview::Projects => {
                self.ensure_project_selection();
                let project_id = self
                    .selected_project_id
                    .clone()
                    .ok_or_else(|| "No project selected".to_string())?;
                let project = self
                    .api
                    .project_get(&project_id)
                    .map_err(|err| err.to_string())?;
                let project_label = project_display_label(&project);
                self.api
                    .comment_add(
                        CommentEntityType::Project,
                        &project_id,
                        body_markdown,
                        &author,
                    )
                    .map_err(|err| err.to_string())?;
                self.persist()?;
                self.status_line = format!("Added comment to project {project_label}");
            }
        }
        Ok(())
    }

    fn show_recent_comments(&mut self, limit: usize) {
        let entity = match self.left_subview {
            LeftSubview::Issues => {
                let Some(issue_id) = self.selected_issue_id.as_deref() else {
                    self.status_line = "No issue selected".to_string();
                    return;
                };
                (CommentEntityType::Issue, issue_id.to_string(), "issue")
            }
            LeftSubview::Projects => {
                self.ensure_project_selection();
                let Some(project_id) = self.selected_project_id.as_deref() else {
                    self.status_line = "No project selected".to_string();
                    return;
                };
                (
                    CommentEntityType::Project,
                    project_id.to_string(),
                    "project",
                )
            }
        };

        match self.api.comment_list(
            entity.0,
            &entity.1,
            rpc_core::CommentListOrder::Desc,
            None,
            limit,
        ) {
            Ok(page) => {
                if page.items.is_empty() {
                    self.status_line = format!("No comments on selected {}", entity.2);
                    return;
                }
                let preview = page
                    .items
                    .iter()
                    .map(|comment| {
                        format!(
                            "{}:{}",
                            comment.author,
                            trim_to_width(
                                comment.body_markdown.lines().next().unwrap_or_default(),
                                32,
                            )
                        )
                    })
                    .collect::<Vec<String>>()
                    .join(" | ");
                self.status_line = format!("Recent comments: {preview}");
            }
            Err(err) => {
                self.status_line = format!("Failed loading comments: {err}");
            }
        }
    }

    fn toggle_right_pane_mode(&mut self) {
        self.right_pane_mode = match self.right_pane_mode {
            RightPaneMode::Session => RightPaneMode::Comments,
            RightPaneMode::Comments => RightPaneMode::Session,
        };
        self.status_line = match self.right_pane_mode {
            RightPaneMode::Session => "Right pane: session output".to_string(),
            RightPaneMode::Comments => "Right pane: comments feed".to_string(),
        };
    }

    fn scroll_form(&mut self, delta: i32) {
        if delta < 0 {
            self.form_scroll = self
                .form_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.form_scroll = self.form_scroll.saturating_add(delta as usize);
        }
    }

    fn current_comment_lines(&self, pane_w: usize, limit: usize) -> Vec<String> {
        let target = match self.left_subview {
            LeftSubview::Issues => self
                .selected_issue_id
                .as_ref()
                .map(|id| (CommentEntityType::Issue, id.clone())),
            LeftSubview::Projects => self
                .selected_project_id
                .as_ref()
                .map(|id| (CommentEntityType::Project, id.clone())),
        };
        let Some((entity_type, entity_id)) = target else {
            return vec!["No selection".to_string()];
        };

        match self.api.comment_list(
            entity_type,
            &entity_id,
            rpc_core::CommentListOrder::Desc,
            None,
            limit.max(1),
        ) {
            Ok(page) => {
                if page.items.is_empty() {
                    return vec!["No comments".to_string()];
                }
                page.items
                    .iter()
                    .map(|comment| {
                        trim_to_width(
                            &format!(
                                "{} | {}",
                                comment.author,
                                comment
                                    .body_markdown
                                    .lines()
                                    .next()
                                    .unwrap_or_default()
                                    .trim()
                            ),
                            pane_w,
                        )
                    })
                    .collect()
            }
            Err(err) => vec![format!("Comments unavailable: {err}")],
        }
    }

    fn current_form_modal(&self) -> Option<(&'static str, Vec<String>)> {
        let lines = match &self.input_mode {
            InputMode::NewIssue(raw) => {
                let title = raw.trim();
                vec![
                    format!(
                        "title: {}",
                        if title.is_empty() {
                            "<required>"
                        } else {
                            &title
                        }
                    ),
                    "Enter to continue to description".to_string(),
                    "Esc cancels".to_string(),
                ]
            }
            InputMode::NewIssueDescription {
                title,
                description,
                active_field,
            } => {
                let mut lines = vec![
                    format!(
                        "active: {}",
                        match active_field {
                            FormField::Title => "title",
                            FormField::Description => "description",
                        }
                    ),
                    format!("title: {}", title),
                    format!("description lines: {}", description.lines().count().max(1)),
                    "description:".to_string(),
                ];
                if description.is_empty() {
                    lines.push("<optional>".to_string());
                } else {
                    lines.extend(description.lines().map(ToString::to_string));
                }
                lines.push(
                    "Tab switch field | Enter continue | Ctrl-E edit description".to_string(),
                );
                lines.push("Up/Down/PageUp/PageDown scroll | Esc cancels".to_string());
                lines
            }
            InputMode::SelectIssueProject {
                query,
                selected_index,
                ..
            } => {
                let projects = self.filtered_projects_for_query(query);
                let mut lines = vec![
                    format!("query: {}", if query.is_empty() { "<none>" } else { query }),
                    "j/k move selection | Enter create".to_string(),
                ];
                if projects.is_empty() {
                    lines.push("no matching projects".to_string());
                } else {
                    for (idx, project) in projects.iter().take(5).enumerate() {
                        let marker = if idx == *selected_index { ">" } else { " " };
                        lines.push(format!(
                            "{marker} {} {}",
                            project_display_label(project),
                            project.name
                        ));
                    }
                }
                lines
            }
            InputMode::EditIssue {
                title,
                description,
                active_field,
            } => {
                let mut lines = vec![
                    format!(
                        "active: {}",
                        match active_field {
                            FormField::Title => "title",
                            FormField::Description => "description",
                        }
                    ),
                    format!(
                        "title: {}",
                        if title.trim().is_empty() {
                            "<required>"
                        } else {
                            title
                        }
                    ),
                    format!("description lines: {}", description.lines().count().max(1)),
                    "description:".to_string(),
                ];
                if description.trim().is_empty() {
                    lines.push("<optional>".to_string());
                } else {
                    lines.extend(description.lines().map(ToString::to_string));
                }
                lines.extend([
                    "Tab switch field | Ctrl-E edit description in $EDITOR".to_string(),
                    "Up/Down/PageUp/PageDown scroll | Enter save | Esc cancel".to_string(),
                ]);
                lines
            }
            InputMode::NewProject(raw) => {
                let (key, name) = parse_new_project_input(raw);
                vec![
                    format!("key: {}", if key.is_empty() { "<required>" } else { &key }),
                    format!(
                        "name: {}",
                        if name.is_empty() { "<required>" } else { &name }
                    ),
                    "format: <KEY> | <NAME>".to_string(),
                    "Enter save | Esc cancel".to_string(),
                ]
            }
            InputMode::AddComment(raw) => vec![
                format!(
                    "preview: {}",
                    trim_to_width(raw.lines().next().unwrap_or_default(), 52)
                ),
                format!("chars: {}", raw.chars().count()),
                "markdown stored as raw text".to_string(),
                "Ctrl-E editor | Enter save | Esc cancel".to_string(),
            ],
            _ => return None,
        };

        let title = match self.input_mode {
            InputMode::NewIssue(_) => "New Issue",
            InputMode::NewProject(_) => "New Project",
            InputMode::AddComment(_) => "Add Comment",
            InputMode::EditIssue { .. } => "Edit Issue",
            _ => "Input",
        };
        Some((title, lines))
    }

    fn draw_center_modal_ratatui(
        &mut self,
        stdout: &mut Stdout,
        terminal_w: u16,
        terminal_h: u16,
        title: &str,
        lines: &[String],
    ) -> Result<(), String> {
        let theme = self.active_theme_palette();
        let desired_w = terminal_w.saturating_sub(8).clamp(24, 72);
        let desired_h = (lines.len() as u16)
            .saturating_add(4)
            .min(terminal_h.saturating_sub(4));
        let modal_area = Self::centered_rect(
            desired_w,
            desired_h,
            Rect::new(0, 0, terminal_w, terminal_h),
        );
        let modal_w = modal_area.width;
        let modal_h = modal_area.height;
        let inner_lines = lines
            .iter()
            .map(|line| TuiLine::from(trim_to_width(line, modal_w.saturating_sub(4) as usize)))
            .collect::<Vec<TuiLine>>();
        let visible_lines = modal_h.saturating_sub(2) as usize;
        let max_scroll = lines.len().saturating_sub(visible_lines);
        self.form_scroll = self.form_scroll.min(max_scroll);
        let scroll = self.form_scroll as u16;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = ratatui::Terminal::new(backend)
            .map_err(|err| format!("modal terminal init failed: {err}"))?;
        terminal
            .draw(|frame| {
                frame.render_widget(TuiClear, modal_area);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(title)
                    .style(
                        TuiStyle::default()
                            .fg(theme.foreground)
                            .bg(theme.background),
                    )
                    .border_style(
                        TuiStyle::default()
                            .fg(theme.border)
                            .add_modifier(TuiModifier::BOLD),
                    );
                let paragraph = Paragraph::new(inner_lines)
                    .block(block)
                    .style(
                        TuiStyle::default()
                            .fg(theme.foreground)
                            .bg(theme.background),
                    )
                    .scroll((scroll, 0));
                frame.render_widget(paragraph, modal_area);
            })
            .map_err(|err| format!("render ratatui modal failed: {err}"))?;

        Ok(())
    }

    fn draw_text_field_form_modal_ratatui(
        &mut self,
        stdout: &mut Stdout,
        terminal_w: u16,
        terminal_h: u16,
    ) -> Result<bool, String> {
        let theme = self.active_theme_palette();
        let (form_title, title_value, description_value, active_field, footer_line) =
            match self.input_mode.clone() {
                InputMode::NewIssueDescription {
                    title,
                    description,
                    active_field,
                } => (
                    "New Issue",
                    title,
                    description,
                    active_field,
                    "Tab switch fields | Ctrl-E editor | Enter continue | Esc cancel".to_string(),
                ),
                InputMode::EditIssue {
                    title,
                    description,
                    active_field,
                } => (
                    "Edit Issue",
                    title,
                    description,
                    active_field,
                    "Tab switch fields | Ctrl-E editor | Enter save | Esc cancel".to_string(),
                ),
                _ => return Ok(false),
            };

        let modal_w = terminal_w.saturating_sub(8).clamp(42, 96);
        let modal_h = terminal_h.saturating_sub(4).clamp(14, 30);
        let modal_area =
            Self::centered_rect(modal_w, modal_h, Rect::new(0, 0, terminal_w, terminal_h));

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = ratatui::Terminal::new(backend)
            .map_err(|err| format!("modal terminal init failed: {err}"))?;
        terminal
            .draw(|frame| {
                frame.render_widget(TuiClear, modal_area);

                let outer = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(form_title)
                    .style(
                        TuiStyle::default()
                            .fg(theme.foreground)
                            .bg(theme.background),
                    )
                    .border_style(
                        TuiStyle::default()
                            .fg(theme.border)
                            .add_modifier(TuiModifier::BOLD),
                    );
                frame.render_widget(outer, modal_area);

                let inner = modal_area.inner(Margin {
                    vertical: 1,
                    horizontal: 1,
                });
                let chunks = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Min(4),
                    Constraint::Length(2),
                ])
                .split(inner);

                let title_style = if active_field == FormField::Title {
                    TuiStyle::default().fg(theme.focus)
                } else {
                    TuiStyle::default().fg(theme.muted)
                };
                let description_style = if active_field == FormField::Description {
                    TuiStyle::default().fg(theme.focus)
                } else {
                    TuiStyle::default().fg(theme.muted)
                };

                let title_text = if title_value.is_empty() {
                    "<required>".to_string()
                } else {
                    title_value
                };
                let title_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title("Title")
                    .border_style(title_style);
                let title_paragraph =
                    Paragraph::new(vec![TuiLine::from(title_text)]).block(title_block);
                frame.render_widget(title_paragraph, chunks[0]);

                let description_lines = if description_value.is_empty() {
                    vec![TuiLine::from("<optional markdown description>")]
                } else {
                    description_value
                        .lines()
                        .map(|line| TuiLine::from(line.to_string()))
                        .collect::<Vec<TuiLine>>()
                };
                let description_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(format!(
                        "Description ({} lines)",
                        description_value.lines().count().max(1)
                    ))
                    .border_style(description_style);
                let desc_inner_height = chunks[1].height.saturating_sub(2) as usize;
                let max_scroll = description_lines
                    .len()
                    .saturating_sub(desc_inner_height.max(1));
                self.form_scroll = self.form_scroll.min(max_scroll);
                let description_paragraph = Paragraph::new(description_lines)
                    .block(description_block)
                    .scroll((self.form_scroll as u16, 0));
                frame.render_widget(description_paragraph, chunks[1]);

                let footer = Paragraph::new(vec![TuiLine::from(format!(
                    "{} | Up/Down/PageUp/PageDown scroll",
                    footer_line
                ))])
                .style(TuiStyle::default().fg(theme.muted));
                frame.render_widget(footer, chunks[2]);
            })
            .map_err(|err| format!("render ratatui text form modal failed: {err}"))?;

        Ok(true)
    }

    fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
        let vertical = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .split(area);
        let horizontal = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .split(vertical[0]);
        horizontal[0]
    }

    fn persist(&self) -> Result<(), String> {
        save_api_to_path(&self.api, &self.state_path)?;
        self.persist_ui_settings()
    }

    fn shift_divider(&mut self, delta_percent: i16) -> Result<(), String> {
        let (w, _) = size().map_err(|err| format!("terminal size failed: {err}"))?;
        let next = if delta_percent.is_negative() {
            self.board_ratio_percent
                .saturating_sub(delta_percent.unsigned_abs())
        } else {
            self.board_ratio_percent
                .saturating_add(delta_percent.unsigned_abs())
        };
        let (min_percent, max_percent) = ratio_bounds(w);
        self.board_ratio_percent = next.clamp(min_percent, max_percent);
        self.status_line = format!(
            "Pane resize: board={}%, session={}%; drag divider or use </>",
            self.board_ratio_percent,
            100_u16.saturating_sub(self.board_ratio_percent)
        );
        self.persist_ui_settings()
    }

    fn set_board_ratio_from_col(&mut self, col: u16, total_width: u16) -> Result<(), String> {
        if total_width <= 1 {
            return Ok(());
        }
        let raw_percent = ((u32::from(col) * 100) / u32::from(total_width)) as u16;
        let (min_percent, max_percent) = ratio_bounds(total_width);
        self.board_ratio_percent = raw_percent.clamp(min_percent, max_percent);
        self.status_line = format!(
            "Resizing panes: board={}%, session={}%; release mouse to save",
            self.board_ratio_percent,
            100_u16.saturating_sub(self.board_ratio_percent)
        );
        self.persist_ui_settings()
    }

    fn compute_board_width(&self, total_width: u16) -> u16 {
        if total_width <= MIN_PANE_COLS.saturating_mul(2) {
            return total_width / 2;
        }
        let (min_percent, max_percent) = ratio_bounds(total_width);
        let percent = self.board_ratio_percent.clamp(min_percent, max_percent);
        ((u32::from(total_width) * u32::from(percent)) / 100) as u16
    }

    fn persist_ui_settings(&self) -> Result<(), String> {
        let path = ui_state_path(&self.state_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed creating ui state dir: {err}"))?;
        }
        fs::write(path, self.board_ratio_percent.to_string())
            .map_err(|err| format!("failed writing ui state: {err}"))
    }

    fn resize_session_to_pane(&mut self, session_id: &str, cols: u16, rows: u16) {
        let cols = cols.max(1);
        let rows = rows.max(1);

        // Only resize when dimensions actually change to avoid SIGWINCH storms.
        // Without this guard, every render frame sends a resize, causing TUI child
        // processes (like Claude Code) to re-render 60x/sec in a feedback loop.
        let current_size = {
            let store = self.screen_store.lock().ok();
            store.and_then(|s| s.get(session_id).map(|p| p.screen().size()))
        };
        if current_size == Some((rows, cols)) {
            return;
        }

        if self.session_manager.has_session(session_id) {
            let _ = self.session_manager.resize(session_id, cols, rows);
        }
    }
}

fn border_fill(width: usize, title: &str) -> String {
    if width == 0 {
        return String::new();
    }
    let title_display = format!(" {title} ");
    let title_chars = title_display.chars().count();
    if width < title_chars + 2 {
        return "\u{2500}".repeat(width);
    }
    let prefix = "\u{2500}\u{2500}";
    let suffix_len = width.saturating_sub(2 + title_chars);
    format!("{prefix}{title_display}{}", "\u{2500}".repeat(suffix_len))
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn issue_display_label(issue: &IssueRecord) -> String {
    issue
        .identifier
        .clone()
        .unwrap_or_else(|| short_id(&issue.id))
}

fn project_display_label(project: &rpc_core::ProjectRecord) -> String {
    if project.identifier.is_empty() {
        short_id(&project.id)
    } else {
        project.identifier.clone()
    }
}

fn ratio_bounds(total_width: u16) -> (u16, u16) {
    if total_width <= 1 {
        return (0, 100);
    }
    let min_percent = ((u32::from(MIN_PANE_COLS) * 100) / u32::from(total_width)) as u16;
    let max_percent = 100_u16.saturating_sub(min_percent);
    (min_percent.min(50), max_percent.max(50))
}

fn ui_state_path(state_path: &std::path::Path) -> PathBuf {
    state_path.with_extension("ui")
}

fn load_board_ratio_percent(state_path: &std::path::Path) -> u16 {
    let path = ui_state_path(state_path);
    let Ok(raw) = fs::read_to_string(path) else {
        return DEFAULT_BOARD_RATIO_PERCENT;
    };
    let Ok(parsed) = raw.trim().parse::<u16>() else {
        return DEFAULT_BOARD_RATIO_PERCENT;
    };
    parsed.clamp(10, 90)
}

fn encode_key_input(key: KeyEvent) -> String {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(ch) = key.code
    {
        let lower = ch.to_ascii_lowercase();
        if !lower.is_ascii() {
            return String::new();
        }
        let byte = (lower as u8) & 0x1f;
        return (byte as char).to_string();
    }

    let base = match key.code {
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Enter => "\r".to_string(),
        KeyCode::Backspace => "\u{7f}".to_string(),
        KeyCode::Tab => "\t".to_string(),
        KeyCode::BackTab => "\u{1b}[Z".to_string(),
        KeyCode::Esc => "\u{1b}".to_string(),
        KeyCode::Up => "\u{1b}[A".to_string(),
        KeyCode::Down => "\u{1b}[B".to_string(),
        KeyCode::Right => "\u{1b}[C".to_string(),
        KeyCode::Left => "\u{1b}[D".to_string(),
        KeyCode::Home => "\u{1b}[H".to_string(),
        KeyCode::End => "\u{1b}[F".to_string(),
        KeyCode::Delete => "\u{1b}[3~".to_string(),
        KeyCode::Insert => "\u{1b}[2~".to_string(),
        KeyCode::PageUp => "\u{1b}[5~".to_string(),
        KeyCode::PageDown => "\u{1b}[6~".to_string(),
        _ => String::new(),
    };

    if base.is_empty() {
        return base;
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        format!("\u{1b}{base}")
    } else {
        base
    }
}

fn trim_to_width(input: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= width {
        input.to_string()
    } else {
        let mut out: String = chars[..width.saturating_sub(1)].iter().collect();
        out.push('~');
        out
    }
}

#[allow(dead_code)]
fn sanitize_output_lines(input: &str) -> Vec<String> {
    let cleaned = strip_ansi_control_sequences(input);
    cleaned
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn normalize_optional_path(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_user_shell() -> String {
    resolve_shell_from_env(std::env::var("SHELL").ok())
}

fn resolve_shell_from_env(shell_env: Option<String>) -> String {
    shell_env
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
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
        fs::create_dir_all(parent)
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

fn load_api_from_path(path: &Path) -> Result<ApiService, String> {
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

fn save_api_to_path(api: &ApiService, path: &Path) -> Result<(), String> {
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

fn theme_mode_from_env() -> ThemeMode {
    std::env::var("DDAK_TUI_THEME")
        .ok()
        .as_deref()
        .and_then(parse_theme_mode)
        .unwrap_or(ThemeMode::Terminal)
}

fn parse_theme_mode(raw: &str) -> Option<ThemeMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "terminal" | "tty" | "auto" => Some(ThemeMode::Terminal),
        "session" | "inherit" => Some(ThemeMode::Session),
        "ocean" => Some(ThemeMode::Ocean),
        "amber" => Some(ThemeMode::Amber),
        "mono" | "default" => Some(ThemeMode::Mono),
        _ => None,
    }
}

fn theme_mode_label(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Terminal => "terminal",
        ThemeMode::Session => "session",
        ThemeMode::Ocean => "ocean",
        ThemeMode::Amber => "amber",
        ThemeMode::Mono => "mono",
    }
}

fn palette_for_theme_mode(mode: ThemeMode) -> ThemePalette {
    match mode {
        ThemeMode::Terminal | ThemeMode::Amber => ThemePalette {
            background: ratatui::style::Color::Black,
            foreground: ratatui::style::Color::Reset,
            border: ratatui::style::Color::Reset,
            focus: ratatui::style::Color::Yellow,
            muted: ratatui::style::Color::DarkGray,

            pane_bg: Color::Black,
            header_bg: Color::Rgb {
                r: 30,
                g: 30,
                b: 46,
            },
            header_fg: Color::Rgb {
                r: 205,
                g: 214,
                b: 244,
            },
            selection_bg: Color::Rgb {
                r: 49,
                g: 50,
                b: 68,
            },
            selection_fg: Color::Rgb {
                r: 205,
                g: 214,
                b: 244,
            },
            status_bar_bg: Color::Rgb {
                r: 30,
                g: 30,
                b: 46,
            },
            status_bar_fg: Color::Rgb {
                r: 205,
                g: 214,
                b: 244,
            },
            accent: Color::Rgb {
                r: 137,
                g: 180,
                b: 250,
            },
            dim: Color::Rgb {
                r: 108,
                g: 112,
                b: 134,
            },
        },
        ThemeMode::Session => palette_for_theme_mode(ThemeMode::Terminal),
        ThemeMode::Ocean => ThemePalette {
            background: ratatui::style::Color::Black,
            foreground: ratatui::style::Color::Cyan,
            border: ratatui::style::Color::LightCyan,
            focus: ratatui::style::Color::Yellow,
            muted: ratatui::style::Color::DarkGray,
            pane_bg: Color::Black,
            header_bg: Color::Rgb {
                r: 24,
                g: 36,
                b: 52,
            },
            header_fg: Color::Rgb {
                r: 166,
                g: 218,
                b: 255,
            },
            selection_bg: Color::Rgb {
                r: 36,
                g: 52,
                b: 70,
            },
            selection_fg: Color::Rgb {
                r: 166,
                g: 218,
                b: 255,
            },
            status_bar_bg: Color::Rgb {
                r: 24,
                g: 36,
                b: 52,
            },
            status_bar_fg: Color::Rgb {
                r: 166,
                g: 218,
                b: 255,
            },
            accent: Color::Rgb {
                r: 100,
                g: 200,
                b: 255,
            },
            dim: Color::Rgb {
                r: 90,
                g: 110,
                b: 130,
            },
        },
        ThemeMode::Mono => ThemePalette {
            background: ratatui::style::Color::Black,
            foreground: ratatui::style::Color::White,
            border: ratatui::style::Color::Gray,
            focus: ratatui::style::Color::Yellow,
            muted: ratatui::style::Color::DarkGray,
            pane_bg: Color::Black,
            header_bg: Color::Rgb {
                r: 38,
                g: 38,
                b: 38,
            },
            header_fg: Color::Rgb {
                r: 200,
                g: 200,
                b: 200,
            },
            selection_bg: Color::Rgb {
                r: 55,
                g: 55,
                b: 55,
            },
            selection_fg: Color::Rgb {
                r: 220,
                g: 220,
                b: 220,
            },
            status_bar_bg: Color::Rgb {
                r: 38,
                g: 38,
                b: 38,
            },
            status_bar_fg: Color::Rgb {
                r: 200,
                g: 200,
                b: 200,
            },
            accent: Color::Rgb {
                r: 180,
                g: 180,
                b: 180,
            },
            dim: Color::Rgb {
                r: 110,
                g: 110,
                b: 110,
            },
        },
    }
}

fn open_external_editor(initial: &str, context: &str) -> Result<String, String> {
    let temp_path = std::env::temp_dir().join(format!(
        "ddak-editor-{}-{}.md",
        std::process::id(),
        current_epoch_ms()
    ));
    fs::write(&temp_path, initial)
        .map_err(|err| format!("{context}: failed to prepare temp file: {err}"))?;

    let mut stdout = io::stdout();
    execute!(stdout, DisableMouseCapture, Show, LeaveAlternateScreen)
        .map_err(|err| format!("{context}: failed to leave TUI: {err}"))?;
    disable_raw_mode().map_err(|err| format!("{context}: failed to disable raw mode: {err}"))?;

    let status = Command::new("/bin/sh")
        .arg("-lc")
        .arg("${EDITOR:-vi} \"$DDAK_EDITOR_FILE\"")
        .env("DDAK_EDITOR_FILE", &temp_path)
        .status();

    let restore_result = enable_raw_mode()
        .and_then(|_| execute!(stdout, EnterAlternateScreen, Hide, EnableMouseCapture).map(|_| ()));

    let _ = stdout.flush();

    if let Err(err) = restore_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "{context}: failed to restore TUI after editor: {err}"
        ));
    }

    let status = status.map_err(|err| format!("{context}: failed launching $EDITOR: {err}"))?;
    if !status.success() {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("{context}: editor exited with status {status}"));
    }

    let edited = fs::read_to_string(&temp_path)
        .map_err(|err| format!("{context}: failed reading editor output: {err}"))?;
    let _ = fs::remove_file(&temp_path);
    Ok(edited)
}

fn default_comment_author() -> String {
    std::env::var("USER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_new_project_input(raw: &str) -> (String, String) {
    if raw.contains('|') {
        let mut parts = raw.splitn(2, '|').map(str::trim);
        let key = parts.next().unwrap_or_default().to_ascii_uppercase();
        let name = parts.next().unwrap_or_default().to_string();
        return (key, name);
    }

    let mut split = raw.splitn(2, char::is_whitespace);
    let key = split.next().unwrap_or_default().trim().to_ascii_uppercase();
    let name = split.next().unwrap_or_default().trim().to_string();
    (key, name)
}

fn is_valid_project_key(key: &str) -> bool {
    let trimmed = key.trim();
    if !(2..=8).contains(&trimmed.len()) {
        return false;
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn apply_key_override(target: &mut char, value: Option<String>) {
    if let Some(value) = value
        && let Some(ch) = value.chars().find(|ch| !ch.is_whitespace())
    {
        *target = ch;
    }
}

fn key_matches(input: char, expected: char) -> bool {
    if expected.is_ascii_alphabetic() {
        input.eq_ignore_ascii_case(&expected)
    } else {
        input == expected
    }
}

fn current_epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn mouse_action_label(action: MouseAction) -> &'static str {
    match action {
        MouseAction::ViewIssues => "Issues",
        MouseAction::ViewProjects => "Projects",
        MouseAction::NewIssue => "+New",
        MouseAction::EditIssue => "Edit",
        MouseAction::NewProject => "+New",
        MouseAction::SetProjectKey => "Key",
        MouseAction::AddComment => "Comment",
        MouseAction::LaunchOpenCode => "OpenCode",
        MouseAction::LaunchClaude => "Claude",
        MouseAction::LaunchShell => "Shell",
        MouseAction::SetProjectPath => "Path",
        MouseAction::SetIssueCwd => "CWD",
        MouseAction::CloseSession => "Close",
        MouseAction::DeleteIssue => "Delete",
    }
}

fn status_color(status: &str, palette: &ThemePalette) -> Color {
    match status {
        "backlog" => palette.dim,
        "ready" => palette.accent,
        "in_progress" => Color::Yellow,
        "review" => Color::Cyan,
        "done" => Color::Green,
        "blocked" => Color::Red,
        _ => palette.header_fg,
    }
}

fn validate_launch_cwd(path: &Path, source_label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!(
            "launch blocked: {source_label} must be an absolute path ({})",
            path.display()
        ));
    }
    if !path.exists() {
        return Err(format!(
            "launch blocked: {source_label} does not exist ({})",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "launch blocked: {source_label} is not a directory ({})",
            path.display()
        ));
    }
    Ok(())
}

#[allow(dead_code)]
fn strip_ansi_control_sequences(input: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Text,
        Escape,
        Csi,
        Osc,
        OscEsc,
    }

    let mut mode = Mode::Text;
    let mut out = String::with_capacity(input.len());
    let mut csi_final: Option<char> = None;

    for ch in input.chars() {
        match mode {
            Mode::Text => {
                if ch == '\u{1b}' {
                    mode = Mode::Escape;
                } else if ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control() {
                    out.push(ch);
                }
            }
            Mode::Escape => match ch {
                '[' => mode = Mode::Csi,
                ']' => mode = Mode::Osc,
                _ => mode = Mode::Text,
            },
            Mode::Csi => {
                if ('@'..='~').contains(&ch) {
                    csi_final = Some(ch);
                    if matches!(ch, 'H' | 'f') {
                        out.push('\n');
                    }
                    mode = Mode::Text;
                }
            }
            Mode::Osc => {
                if ch == '\u{7}' {
                    mode = Mode::Text;
                } else if ch == '\u{1b}' {
                    mode = Mode::OscEsc;
                }
            }
            Mode::OscEsc => {
                if ch == '\\' {
                    mode = Mode::Text;
                } else {
                    mode = Mode::Osc;
                }
            }
        }
    }

    if matches!(csi_final, Some('H' | 'f')) && !out.ends_with('\n') {
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{
        ACTION_BAR_Y, BOARD_START_Y, BoardPocApp, DEFAULT_PROJECT_NAME, LeftSubview,
        load_board_ratio_percent, ratio_bounds, sanitize_output_lines,
        strip_ansi_control_sequences, trim_to_width, ui_state_path,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use vt100::Parser;

    #[test]
    fn creating_and_moving_issue_updates_status() {
        let temp = std::env::temp_dir().join("ddak-poc-state-a.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("first task");
        app.selected_issue_id = Some(issue.id.clone());
        app.move_selected("in_progress")
            .expect("move should succeed");
        let moved = app.api.issue_get(&issue.id).expect("issue should exist");
        assert_eq!(moved.status, "in_progress");
    }

    #[test]
    fn close_selected_session_unlinks_primary_session() {
        let temp = std::env::temp_dir().join("ddak-poc-state-close.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("close task");
        let session = app.api.session_create();
        app.api
            .issue_link_primary_session(&issue.id, &session.id)
            .expect("link should succeed");
        app.selected_issue_id = Some(issue.id.clone());

        app.close_selected_session()
            .expect("close action should succeed");
        assert_eq!(app.api.issue_primary_session(&issue.id), None);
    }

    #[test]
    fn delete_selected_issue_removes_issue_and_linked_session() {
        let temp = std::env::temp_dir().join("ddak-poc-state-delete.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("delete task");
        let session = app.api.session_create();
        app.api
            .issue_link_primary_session(&issue.id, &session.id)
            .expect("link should succeed");
        app.selected_issue_id = Some(issue.id.clone());

        app.delete_selected_issue()
            .expect("delete action should succeed");
        assert!(app.api.issue_get(&issue.id).is_err());
    }

    #[test]
    fn trim_marks_truncation() {
        let s = trim_to_width("abcdefghijk", 6);
        assert_eq!(s, "abcde~");
    }

    #[test]
    fn strip_ansi_removes_osc_and_csi_sequences() {
        let input = "before\u{1b}]11;?\u{7}mid\u{1b}[31mred\u{1b}[0mafter";
        let stripped = strip_ansi_control_sequences(input);
        assert_eq!(stripped, "beforemidredafter");
    }

    #[test]
    fn sanitize_output_discards_empty_and_normalizes_carriage_returns() {
        let input = "line-1\rline-2\n\u{1b}]11;rgb:ffff/ffff/ffff\u{7}\n";
        let lines = sanitize_output_lines(input);
        assert_eq!(lines, vec!["line-1", "line-2"]);
    }

    #[test]
    fn sanitize_output_preserves_cursor_addressed_text() {
        let input = "\u{1b}[6;42HHELLO\u{1b}[7;1HBYE";
        let lines = sanitize_output_lines(input);
        assert_eq!(lines, vec!["HELLO", "BYE"]);
    }

    #[test]
    fn vt_formatted_rows_preserve_sgr_sequences() {
        let mut parser = Parser::new(4, 20, 100);
        parser.process(b"\x1b[31mred\x1b[0m");
        let formatted: Vec<String> = parser
            .screen()
            .rows_formatted(0, 20)
            .map(|row| String::from_utf8_lossy(&row).into_owned())
            .collect();
        let joined = formatted.join("\n");
        assert!(joined.contains("\x1b[31m"));
    }

    #[test]
    fn ratio_bounds_preserve_minimum_pane_width() {
        let (min_percent, max_percent) = ratio_bounds(100);
        assert!(min_percent >= 24);
        assert!(max_percent <= 76);
    }

    #[test]
    fn ui_state_path_reuses_base_filename() {
        let path = std::path::Path::new(".ddak/poc-state.json");
        let ui_path = ui_state_path(path);
        assert_eq!(ui_path, std::path::PathBuf::from(".ddak/poc-state.ui"));
    }

    #[test]
    fn load_board_ratio_uses_default_on_missing_file() {
        let path = std::env::temp_dir().join("ddak-ui-state-missing.json");
        let _ = std::fs::remove_file(path.with_extension("ui"));
        let ratio = load_board_ratio_percent(&path);
        assert_eq!(ratio, 45);
    }

    #[test]
    fn launch_cwd_is_blocked_without_project_or_overrides() {
        let temp = std::env::temp_dir().join("ddak-poc-state-cwd-blocked.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("missing cwd policy");

        let err = app
            .resolve_launch_cwd_for_issue(&issue.id)
            .expect_err("cwd resolution should fail");
        assert!(err.contains("launch blocked"));
    }

    #[test]
    fn enter_launch_error_is_reported_without_exiting_app() {
        let temp = std::env::temp_dir().join("ddak-poc-state-launch-error-nonfatal.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("missing cwd policy");
        app.selected_issue_id = Some(issue.id);

        let should_quit = app
            .handle_normal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("enter key handler should not fail");

        assert!(!should_quit);
        assert!(app.status_line.contains("launch blocked"));
    }

    #[test]
    fn launch_cwd_resolution_precedence_is_runtime_then_issue_then_project() {
        let temp = std::env::temp_dir().join("ddak-poc-state-cwd-precedence.json");
        let _ = std::fs::remove_file(&temp);

        let runtime_dir = std::env::temp_dir().join("ddak-runtime-cwd");
        let project_dir = std::env::temp_dir().join("ddak-project-cwd");
        let issue_dir = std::env::temp_dir().join("ddak-issue-cwd");
        std::fs::create_dir_all(&runtime_dir).expect("runtime dir should exist");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");
        std::fs::create_dir_all(&issue_dir).expect("issue dir should exist");

        let mut app = BoardPocApp::new(
            Some(temp),
            Some("printf opencode".to_string()),
            None,
            Some(runtime_dir.clone()),
        );
        let project = app.api.project_create("repo project");
        app.api
            .project_set_repo_local_path(
                &project.id,
                Some(project_dir.to_string_lossy().into_owned()),
            )
            .expect("project path should be set");
        let issue = app.api.issue_create("issue with override");
        app.api
            .issue_assign_project(&issue.id, &project.id)
            .expect("issue should link to project");
        app.api
            .issue_set_cwd_override(&issue.id, Some(issue_dir.to_string_lossy().into_owned()))
            .expect("issue cwd override should be set");

        let (resolved, source) = app
            .resolve_launch_cwd_for_issue(&issue.id)
            .expect("cwd should resolve");
        assert_eq!(source, "runtime_override");
        assert_eq!(resolved, runtime_dir);
    }

    #[test]
    fn launch_cwd_rejects_relative_issue_override_path() {
        let temp = std::env::temp_dir().join("ddak-poc-state-cwd-relative.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("relative path");
        app.api
            .issue_set_cwd_override(&issue.id, Some("relative/path".to_string()))
            .expect("issue cwd override should be set");

        let err = app
            .resolve_launch_cwd_for_issue(&issue.id)
            .expect_err("relative path must be rejected");
        assert!(err.contains("must be an absolute path"));
    }

    #[test]
    fn set_project_repo_path_assigns_default_project_to_issue() {
        let temp = std::env::temp_dir().join("ddak-poc-state-default-project.json");
        let _ = std::fs::remove_file(&temp);

        let project_dir = std::env::temp_dir().join("ddak-default-project-repo");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("needs project");
        app.selected_issue_id = Some(issue.id.clone());

        app.set_selected_issue_project_repo_path(Some(project_dir.to_string_lossy().into_owned()))
            .expect("project path set should succeed");

        let updated_issue = app.api.issue_get(&issue.id).expect("issue should exist");
        let project_id = updated_issue
            .project_id
            .expect("issue should be attached to default project");
        let project = app
            .api
            .project_get(&project_id)
            .expect("project should exist");
        assert_eq!(project.name, DEFAULT_PROJECT_NAME);
        assert_eq!(
            project.repo_local_path.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn set_issue_cwd_override_rejects_relative_path() {
        let temp = std::env::temp_dir().join("ddak-poc-state-issue-override-validate.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("override validate");
        app.selected_issue_id = Some(issue.id.clone());

        let err = app
            .set_selected_issue_cwd_override(Some("relative/path".to_string()))
            .expect_err("relative issue override should fail");
        assert!(err.contains("must be an absolute path"));
    }

    #[test]
    fn initial_path_inputs_fall_back_to_app_cwd() {
        let temp = std::env::temp_dir().join("ddak-poc-state-default-input-cwd.json");
        let _ = std::fs::remove_file(&temp);

        let app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let expected = app.app_cwd.to_string_lossy().into_owned();
        assert_eq!(app.initial_project_repo_path_input(), expected);
        assert_eq!(app.initial_issue_cwd_override_input(), expected);
    }

    #[test]
    fn initial_issue_override_input_prefers_project_repo_path() {
        let temp = std::env::temp_dir().join("ddak-poc-state-issue-default-from-project.json");
        let _ = std::fs::remove_file(&temp);

        let project_dir = std::env::temp_dir().join("ddak-issue-default-project-path");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("path default");
        app.selected_issue_id = Some(issue.id.clone());
        app.set_selected_issue_project_repo_path(Some(project_dir.to_string_lossy().into_owned()))
            .expect("project path set should succeed");

        assert_eq!(
            app.initial_issue_cwd_override_input(),
            project_dir.to_string_lossy()
        );
    }

    #[test]
    fn mouse_click_on_action_bar_opens_new_issue_mode() {
        let temp = std::env::temp_dir().join("ddak-poc-state-mouse-action-new-issue.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let (click_col, _, _) = app
            .action_button_regions()
            .into_iter()
            .find(|(_, _, action)| matches!(action, super::MouseAction::NewIssue))
            .expect("new issue action should exist");
        let handled = app
            .handle_mouse_action_click(click_col, ACTION_BAR_Y)
            .expect("mouse action click should be handled");
        assert!(handled);
        assert!(matches!(app.input_mode, super::InputMode::NewIssue(_)));
    }

    #[test]
    fn projects_subview_hides_launch_actions_in_action_bar() {
        let temp = std::env::temp_dir().join("ddak-poc-state-action-visibility-projects.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.left_subview = LeftSubview::Projects;

        let actions: Vec<super::MouseAction> = app
            .action_button_regions()
            .into_iter()
            .map(|(_, _, action)| action)
            .collect();
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, super::MouseAction::LaunchOpenCode))
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, super::MouseAction::DeleteIssue))
        );
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, super::MouseAction::NewProject))
        );
    }

    #[test]
    fn issues_subview_hides_project_only_actions_in_action_bar() {
        let temp = std::env::temp_dir().join("ddak-poc-state-action-visibility-issues.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.left_subview = LeftSubview::Issues;

        let actions: Vec<super::MouseAction> = app
            .action_button_regions()
            .into_iter()
            .map(|(_, _, action)| action)
            .collect();
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, super::MouseAction::NewProject))
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, super::MouseAction::SetProjectKey))
        );
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, super::MouseAction::LaunchOpenCode))
        );
    }

    #[test]
    fn issue_row_mapping_returns_selected_issue_id() {
        let temp = std::env::temp_dir().join("ddak-poc-state-issue-row-mapping.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("row mapping");

        let id = app
            .issue_id_at_row(BOARD_START_Y + 1, 40)
            .expect("first backlog issue should map to first issue row");
        assert_eq!(id, issue.id);
    }

    #[test]
    fn execute_command_switches_subview() {
        let temp = std::env::temp_dir().join("ddak-poc-state-command-subview.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.execute_command("projects");
        assert_eq!(app.left_subview, LeftSubview::Projects);
        app.execute_command("issues");
        assert_eq!(app.left_subview, LeftSubview::Issues);
    }

    #[test]
    fn execute_command_sets_and_clears_issue_filter() {
        let temp = std::env::temp_dir().join("ddak-poc-state-command-filter.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue_a = app.api.issue_create("alpha task");
        let issue_b = app.api.issue_create("beta task");

        app.execute_command("filter beta");
        let filtered = app.issues_sorted();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, issue_b.id);

        app.execute_command("clear-filter");
        let all = app.issues_sorted();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|issue| issue.id == issue_a.id));
        assert!(all.iter().any(|issue| issue.id == issue_b.id));
    }

    #[test]
    fn execute_command_project_new_and_assign() {
        let temp = std::env::temp_dir().join("ddak-poc-state-command-project-new-assign.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("ticket");
        app.selected_issue_id = Some(issue.id.clone());

        app.execute_command("project-new DEV Development");
        assert!(app.status_line.contains("Created project DEV"));

        app.execute_command("project-assign DEV");
        let updated = app
            .api
            .issue_get(&issue.id)
            .expect("issue should be assignable to created project");
        assert_eq!(updated.identifier.as_deref(), Some("DEV-0001"));
    }

    #[test]
    fn execute_command_project_key_updates_pre_issue_project() {
        let temp = std::env::temp_dir().join("ddak-poc-state-command-project-key.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.execute_command("project-new TMP Temporary Project");
        app.execute_command("project-key TMP ATTR");
        assert!(app.status_line.contains("Updated project key to ATTR"));
        assert!(app.api.project_find_by_identifier("ATTR").is_some());
    }

    #[test]
    fn execute_command_theme_updates_theme_mode() {
        let temp = std::env::temp_dir().join("ddak-poc-state-command-theme.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.execute_command("theme ocean");
        assert!(matches!(app.theme_mode, super::ThemeMode::Ocean));
        assert!(app.status_line.contains("Theme set to ocean"));

        app.execute_command("theme unknown");
        assert!(app.status_line.contains("Unknown theme"));
        assert!(matches!(app.theme_mode, super::ThemeMode::Ocean));
    }

    #[test]
    fn new_shortcut_in_projects_view_opens_new_project_input() {
        let temp = std::env::temp_dir().join("ddak-poc-state-new-project-input-mode.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.left_subview = LeftSubview::Projects;

        let should_quit = app
            .handle_normal_key(KeyEvent::new(
                KeyCode::Char(app.key_bindings.new_issue),
                KeyModifiers::NONE,
            ))
            .expect("new key should enter project creation mode in projects subview");

        assert!(!should_quit);
        assert!(matches!(app.input_mode, super::InputMode::NewProject(_)));
    }

    #[test]
    fn project_key_shortcut_opens_edit_mode_for_selected_project() {
        let temp = std::env::temp_dir().join("ddak-poc-state-project-key-edit-mode.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let project = app.api.project_create("Development");
        app.api
            .project_set_identifier(&project.id, "DEV")
            .expect("project key set should succeed");
        app.left_subview = LeftSubview::Projects;
        app.selected_project_id = Some(project.id);

        let should_quit = app
            .handle_normal_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT))
            .expect("K should open project key editor");

        assert!(!should_quit);
        match &app.input_mode {
            super::InputMode::SetProjectIdentifier(value) => assert_eq!(value, "DEV"),
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn context_line_for_issue_shows_comment_count() {
        let temp = std::env::temp_dir().join("ddak-poc-state-context-issue.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("issue context");
        app.api
            .comment_add(
                rpc_core::CommentEntityType::Issue,
                &issue.id,
                "markdown note",
                "tester",
            )
            .expect("comment add should succeed");
        app.selected_issue_id = Some(issue.id);

        let context = app.context_line();
        assert!(context.contains("comments=1"));
    }

    #[test]
    fn context_line_for_project_shows_issue_and_comment_counts() {
        let temp = std::env::temp_dir().join("ddak-poc-state-context-project.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let project = app.api.project_create("Development");
        let issue = app.api.issue_create("linked");
        app.api
            .issue_assign_project(&issue.id, &project.id)
            .expect("assign issue to project should succeed");
        app.api
            .comment_add(
                rpc_core::CommentEntityType::Project,
                &project.id,
                "project note",
                "tester",
            )
            .expect("project comment add should succeed");
        app.left_subview = LeftSubview::Projects;
        app.selected_project_id = Some(project.id);

        let context = app.context_line();
        assert!(context.contains("issues=1"));
        assert!(context.contains("comments=1"));
    }

    #[test]
    fn add_comment_shortcut_opens_input_mode() {
        let temp = std::env::temp_dir().join("ddak-poc-state-comment-mode.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("comment mode issue");
        app.selected_issue_id = Some(issue.id);

        let should_quit = app
            .handle_normal_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("a should open comment input mode");
        assert!(!should_quit);
        assert!(matches!(app.input_mode, super::InputMode::AddComment(_)));
    }

    #[test]
    fn add_comment_mode_saves_issue_comment() {
        let temp = std::env::temp_dir().join("ddak-poc-state-comment-save.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("comment save issue");
        app.selected_issue_id = Some(issue.id.clone());
        app.input_mode = super::InputMode::AddComment("note from tui".to_string());

        let should_quit = app
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("Enter should save comment");
        assert!(!should_quit);

        let comments = app
            .api
            .comment_list(
                rpc_core::CommentEntityType::Issue,
                &issue.id,
                rpc_core::CommentListOrder::Desc,
                None,
                10,
            )
            .expect("comment list should succeed");
        assert_eq!(comments.items.len(), 1);
        assert_eq!(comments.items[0].body_markdown, "note from tui");
    }

    #[test]
    fn new_issue_flow_transitions_to_description_and_project_picker() {
        let temp = std::env::temp_dir().join("ddak-poc-state-issue-wizard.json");
        let _ = std::fs::remove_file(&temp);
        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);

        app.input_mode = super::InputMode::NewIssue("wizard issue".to_string());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("title enter should advance to description mode");
        assert!(matches!(
            app.input_mode,
            super::InputMode::NewIssueDescription { .. }
        ));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("description enter should advance to project picker");
        assert!(matches!(
            app.input_mode,
            super::InputMode::SelectIssueProject { .. }
        ));
    }

    #[test]
    fn edit_issue_mode_updates_title_and_adds_description_comment() {
        let temp = std::env::temp_dir().join("ddak-poc-state-edit-issue.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("old title");
        app.selected_issue_id = Some(issue.id.clone());
        app.input_mode = super::InputMode::EditIssue {
            title: "new title".to_string(),
            description: "details markdown".to_string(),
            active_field: super::FormField::Title,
        };

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("Enter should save issue updates");

        let updated = app
            .api
            .issue_get(&issue.id)
            .expect("updated issue should still exist");
        assert_eq!(updated.title, "new title");
        let comments = app
            .api
            .comment_list(
                rpc_core::CommentEntityType::Issue,
                &issue.id,
                rpc_core::CommentListOrder::Desc,
                None,
                10,
            )
            .expect("comment list should succeed");
        assert_eq!(comments.items.len(), 1);
        assert_eq!(comments.items[0].body_markdown, "details markdown");
    }

    #[test]
    fn edit_issue_mode_supports_multiline_description_markdown() {
        let temp = std::env::temp_dir().join("ddak-poc-state-edit-issue-multiline.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("old title");
        app.selected_issue_id = Some(issue.id.clone());
        app.input_mode = super::InputMode::EditIssue {
            title: "new title".to_string(),
            description: "heading\n* item".to_string(),
            active_field: super::FormField::Title,
        };

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("Enter should save issue updates");

        let comments = app
            .api
            .comment_list(
                rpc_core::CommentEntityType::Issue,
                &issue.id,
                rpc_core::CommentListOrder::Desc,
                None,
                10,
            )
            .expect("comment list should succeed");
        assert_eq!(comments.items[0].body_markdown, "heading\n* item");
    }

    #[test]
    fn issue_form_tab_switches_between_title_and_description() {
        let temp = std::env::temp_dir().join("ddak-poc-state-issue-form-tab-switch.json");
        let _ = std::fs::remove_file(&temp);
        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);

        app.input_mode = super::InputMode::NewIssueDescription {
            title: "old title".to_string(),
            description: String::new(),
            active_field: super::FormField::Title,
        };
        app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE))
            .expect("typing should update title field");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("tab should switch field");
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("typing on description field should be handled");

        match app.input_mode {
            super::InputMode::NewIssueDescription {
                title,
                description,
                active_field,
            } => {
                assert_eq!(title, "old titleX");
                assert_eq!(description, "");
                assert!(matches!(active_field, super::FormField::Description));
            }
            other => panic!("unexpected mode: {other:?}"),
        }
        assert!(app.status_line.contains("Ctrl-E"));
    }

    #[test]
    fn form_modal_includes_full_multiline_description_content() {
        let temp = std::env::temp_dir().join("ddak-poc-state-form-multiline-view.json");
        let _ = std::fs::remove_file(&temp);
        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);

        app.input_mode = super::InputMode::EditIssue {
            title: "desc test".to_string(),
            description: "line one\nline two\nline three".to_string(),
            active_field: super::FormField::Description,
        };

        let (_, lines) = app
            .current_form_modal()
            .expect("edit issue modal should be visible");
        let joined = lines.join("\n");
        assert!(joined.contains("line one"));
        assert!(joined.contains("line two"));
        assert!(joined.contains("line three"));
    }

    #[test]
    fn edit_issue_mode_prefills_description_from_latest_comment() {
        let temp = std::env::temp_dir().join("ddak-poc-state-edit-prefill-comment.json");
        let _ = std::fs::remove_file(&temp);
        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("prefill title");
        app.api
            .comment_add(
                rpc_core::CommentEntityType::Issue,
                &issue.id,
                "existing details",
                "tester",
            )
            .expect("comment add should succeed");
        app.selected_issue_id = Some(issue.id);

        app.handle_normal_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("edit shortcut should open issue edit mode");

        match app.input_mode {
            super::InputMode::EditIssue {
                title,
                description,
                active_field,
            } => {
                assert_eq!(title, "prefill title");
                assert_eq!(description, "existing details");
                assert!(matches!(active_field, super::FormField::Title));
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn toggle_right_pane_mode_switches_between_session_and_comments() {
        let temp = std::env::temp_dir().join("ddak-poc-state-toggle-pane.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        assert!(matches!(app.right_pane_mode, super::RightPaneMode::Session));

        app.toggle_right_pane_mode();
        assert!(matches!(
            app.right_pane_mode,
            super::RightPaneMode::Comments
        ));

        app.toggle_right_pane_mode();
        assert!(matches!(app.right_pane_mode, super::RightPaneMode::Session));
    }

    #[test]
    fn form_modal_present_for_creation_modes() {
        let temp = std::env::temp_dir().join("ddak-poc-state-form-preview.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        app.input_mode = super::InputMode::NewIssue("feature work | DEV".to_string());
        let issue_preview = app
            .current_form_modal()
            .expect("issue preview should render in input mode");
        assert_eq!(issue_preview.0, "New Issue");

        app.input_mode = super::InputMode::NewProject("DEV | Development".to_string());
        let project_preview = app
            .current_form_modal()
            .expect("project preview should render in input mode");
        assert_eq!(project_preview.0, "New Project");

        app.input_mode = super::InputMode::Normal;
        assert!(app.current_form_modal().is_none());
    }

    #[test]
    fn enter_in_projects_view_does_not_attempt_launch() {
        let temp = std::env::temp_dir().join("ddak-poc-state-enter-projects-view.json");
        let _ = std::fs::remove_file(&temp);

        let mut app = BoardPocApp::new(Some(temp), Some("printf opencode".to_string()), None, None);
        let issue = app.api.issue_create("do not launch");
        app.selected_issue_id = Some(issue.id);
        app.left_subview = LeftSubview::Projects;

        let should_quit = app
            .handle_normal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("enter key in projects view should not fail");

        assert!(!should_quit);
        assert!(app.status_line.contains("issues subview"));
    }

    #[test]
    fn resolve_shell_from_env_prefers_non_empty_value() {
        let shell = super::resolve_shell_from_env(Some("/bin/zsh".to_string()));
        assert_eq!(shell, "/bin/zsh");
    }

    #[test]
    fn resolve_shell_from_env_falls_back_to_sh() {
        let shell = super::resolve_shell_from_env(Some("   ".to_string()));
        assert_eq!(shell, "/bin/sh");

        let shell = super::resolve_shell_from_env(None);
        assert_eq!(shell, "/bin/sh");
    }
}
