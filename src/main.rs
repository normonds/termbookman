use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEvent, MouseEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect, Position},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarState, ScrollbarOrientation},
    Frame, Terminal,
};
use std::{
    error::Error,
    io::{self, Write, Read},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use vt100::{Parser, MouseProtocolEncoding, MouseProtocolMode};
use std::sync::mpsc;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Default)]
struct Config {
    #[serde(default)]
    statusbar: StatusBarConfig,
    #[serde(default)]
    auth: AuthConfig,
    #[serde(default = "default_editor")]
    external_editor: String,
}

fn default_editor() -> String { "nano".to_string() }

#[derive(Deserialize, Serialize, Clone, Default)]
struct AuthConfig {
    github_client_id: Option<String>,
    personal_access_token: Option<String>,
    #[serde(default)]
    scope: String,
}

#[derive(Deserialize, Serialize, Clone, Default)]
struct StatusBarConfig {
    #[serde(default)]
    upper: Vec<StatusBarItem>,
    #[serde(default)]
    lower: Vec<StatusBarItem>,
}

#[derive(Deserialize, Serialize, Clone)]
struct StatusBarItem {
    #[serde(default)]
    type_: ItemType,
    label: Option<String>,
    action: Option<ActionType>,
    command: Option<String>,
    color: Option<String>,
    hover_color: Option<String>,
    condition: Option<ConditionType>,
    width: Option<u16>,
}

#[derive(Deserialize, Serialize, Clone, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ItemType { #[default] Button, Spacer, SystemStats, GitInfo, TimeAndScroll, SelectedCommandInfo }

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ActionType { ToggleMenu, CopySelection, SendCommand, Quit, ShowSettingsModal, FetchGists }

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ConditionType { HasGit, HasSelection }

fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
        let config_dir = proj_dirs.config_dir();
        if !config_dir.exists() {
            std::fs::create_dir_all(config_dir)?;
        }
        let config_file = config_dir.join("config.toml");
        let content = toml::to_string_pretty(config)?;
        std::fs::write(config_file, content)?;
    }
    Ok(())
}

fn load_config() -> Config {
    let default_config = r#"
external_editor = "nano"

[auth]
scope = "gist"
personal_access_token = "YOUR_TOKEN_HERE"

[statusbar]
[[statusbar.upper]]
label = " ≡ MENU "
action = "toggle_menu"
color = "green"
hover_color = "light_green"

[[statusbar.upper]]
label = " COPY "
action = "copy_selection"
color = "cyan"
hover_color = "light_cyan"
condition = "has_selection"

[[statusbar.upper]]
type_ = "selected_command_info"

[[statusbar.upper]]
type_ = "time_and_scroll"
width = 30

[[statusbar.lower]]
label = " STATUS "
action = "send_command"
command = "git status\r"
color = "cyan"
hover_color = "light_cyan"
condition = "has_git"

[[statusbar.lower]]
label = " DIFF "
action = "send_command"
command = "git diff\r"
color = "blue"
hover_color = "light_blue"
condition = "has_git"

[[statusbar.lower]]
label = " SHOW "
action = "send_command"
command = "git show\r"
color = "magenta"
hover_color = "light_magenta"
condition = "has_git"

[[statusbar.lower]]
label = " HISTORY "
action = "send_command"
command = "git log --oneline -n 20\r"
color = "orange"
hover_color = "light_orange"
condition = "has_git"

[[statusbar.lower]]
type_ = "git_info"

[[statusbar.lower]]
type_ = "system_stats"
width = 25

[[statusbar.lower]]
type_ = "spacer"
width = 1

[[statusbar.lower]]
label = " SETTINGS "
action = "show_settings_modal"
color = "magenta"
hover_color = "light_magenta"

[[statusbar.lower]]
label = " GISTS "
action = "fetch_gists"
color = "yellow"
hover_color = "light_yellow"

[[statusbar.lower]]
label = " EXIT "
action = "quit"
color = "red"
hover_color = "light_red"
    "#;
    
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
        let config_dir = proj_dirs.config_dir();
        let config_file = config_dir.join("config.toml");
        log_debug(&format!("Checking config at: {:?}", config_file));
        
        if config_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_file) {
                match toml::from_str(&content) {
                    Ok(config) => {
                        log_debug("Config loaded successfully from file");
                        return config;
                    },
                    Err(e) => {
                        log_debug(&format!("Config parse error: {}", e));
                    }
                }
            }
        } else {
            if !config_dir.exists() {
                let _ = std::fs::create_dir_all(config_dir);
            }
            log_debug("Config file not found, writing default");
            let _ = std::fs::write(&config_file, default_config);
        }
    } else {
        log_debug("Could not determine project directories");
    }
    
    log_debug("Falling back to default config");
    toml::from_str(default_config).unwrap_or_default()
}

enum Message {
    Event(Event),
    PtyData,
    Tick,
    DeviceCodeSuccess(String, String, String), // device_code, user_code, verification_uri
    AuthSuccess(String), // access_token
    AuthError(String),
    FetchGists,
    GistsFetched(Vec<(String, String, String, Option<std::time::SystemTime>, Option<std::path::PathBuf>, String)>), // (label, info, command, mtime, path, remote_name)
    GistUploadStatus(String, bool), // (message, is_success)
}

fn log_debug(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("debug.log") {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg);
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum SidebarMode { Commands, History, Gists }

struct App {
    cpu_usage: f32,
    mem_usage: (u64, u64),
    git_info: Option<String>,
    sidebar_state: ListState,
    is_dragging_scrollbar: bool,
    is_dragging_sidebar_scrollbar: bool,
    is_selecting: bool,
    selection_start: Option<(u16, u16)>,
    selection_end: Option<(u16, u16)>,
    sidebar_items: Vec<String>,
    sidebar_commands: Vec<String>,
    sidebar_infos: Vec<String>,
    sidebar_mtimes: Vec<Option<std::time::SystemTime>>,
    sidebar_paths: Vec<Option<std::path::PathBuf>>,
    sidebar_width: u16,
    is_dragging_sidebar: bool,
    is_dragging_term_scrollbar: bool,
    show_menu: bool,
    mouse_pos: Option<(u16, u16)>,
    parser: Arc<Mutex<Parser>>,
    pty_write: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    should_quit: bool,
    last_activity: Instant,
    start_time: Instant,
    search_query: String,
    is_search_focused: bool,
    sidebar_mode: SidebarMode,
    history_items: Vec<String>,
    history_commands: Vec<String>,
    gist_items: Vec<String>,
    gist_commands: Vec<String>,
    gist_infos: Vec<String>,
    gist_mtimes: Vec<Option<std::time::SystemTime>>,
    gist_paths: Vec<Option<std::path::PathBuf>>,
    gist_remote_names: Vec<String>,
    shell_pid: u32,
    is_pty_busy: bool,
    config: Config,
    
    // Settings / Auth State
    show_settings_modal: bool,
    github_user_code: Option<String>,
    github_verification_uri: Option<String>,
    github_device_code: Option<String>,
    auth_token: Option<String>,
    login_error: Option<String>,
    pat_input: String,
    is_pat_focused: bool,
    editor_input: String,
    is_editor_focused: bool,
    loading_gist: bool,
    last_click_time: Option<Instant>,
    last_clicked_index: Option<usize>,
    
    // Editing Tracking
    editing_file: Option<(std::path::PathBuf, std::time::SystemTime)>,
    last_pty_busy: bool,
    show_upload_confirm: bool,
    pending_gist_file: Option<std::path::PathBuf>,
}


fn format_time_passed(t: std::time::SystemTime) -> String {
    let now = std::time::SystemTime::now();
    let duration = now.duration_since(t).unwrap_or_default();
    let secs = duration.as_secs();
    
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    
    let mut parts = Vec::new();
    if days > 0 { parts.push(format!("{}d", days)); }
    if hours > 0 { parts.push(format!("{}h", hours)); }
    if mins > 0 || parts.is_empty() { parts.push(format!("{}m", mins)); }
    
    parts.join("")
}

fn parse_script_content(content: &str) -> (String, String) {
    let mut description = String::new();
    let mut code_preview = String::new();
    let mut in_description = false;
    let mut found_shebang = false;
    
    for line in content.lines() {
        let trimmed = line.trim();
        
        if !found_shebang {
            if trimmed.starts_with("#!") {
                found_shebang = true;
                in_description = true;
            }
            continue;
        }
        
        if in_description {
            if trimmed.starts_with('#') {
                let desc_text = trimmed.trim_start_matches('#').trim();
                if !desc_text.is_empty() {
                    if !description.is_empty() {
                        description.push(' ');
                    }
                    description.push_str(desc_text);
                }
            } else if trimmed.is_empty() {
                in_description = false;
            } else {
                in_description = false;
                // First non-comment line after comments
                if !trimmed.is_empty() && !code_preview.is_empty() {
                    code_preview.push(' ');
                }
                code_preview.push_str(trimmed);
            }
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
            if !code_preview.is_empty() {
                code_preview.push(' ');
            }
            code_preview.push_str(trimmed);
            // Limit preview length
            if code_preview.len() > 100 {
                code_preview.truncate(97);
                code_preview.push_str("...");
                break;
            }
        }
    }
    
    (description, code_preview)
}

fn parse_lines(content: &str, default_label: &str, label_counts: &mut std::collections::HashMap<String, usize>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut labels = Vec::new();
    let mut commands = Vec::new();
    let mut infos = Vec::new();
    let mut last_label_base: Option<String> = None;
    let mut last_info = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.contains("# termbookman") { continue; }
        
        if trimmed.starts_with('#') {
            let parts: Vec<&str> = trimmed[1..].trim().split_whitespace().collect();
            if let Some(first) = parts.first() {
                last_label_base = Some(first.to_string());
                last_info = parts[1..].join(" ");
            }
        } else {
            let base_name = last_label_base.take().unwrap_or_else(|| {
                trimmed.split_whitespace().next().unwrap_or(default_label).to_string()
            });
            
            let count = label_counts.entry(base_name.clone()).or_insert(0);
            *count += 1;
            
            let final_label = if *count > 1 {
                format!("{}{}", base_name, *count)
            } else {
                base_name
            };

            labels.push(final_label);
            commands.push(trimmed.to_string());
            infos.push(last_info.clone());
            last_info = String::new();
        }
    }
    (labels, commands, infos)
}

fn load_commands(exe_dir: &std::path::Path) -> (Vec<String>, Vec<String>, Vec<String>, Vec<Option<std::time::SystemTime>>, Vec<Option<std::path::PathBuf>>) {
    let mut labels = Vec::new();
    let mut commands = Vec::new();
    let mut infos = Vec::new();
    let mut mtimes = Vec::new();
    let mut paths = Vec::new();
    let mut label_counts = std::collections::HashMap::new();

    // Load local commands
    let cmd_path = exe_dir.join("commands.txt");
    if let Ok(content) = std::fs::read_to_string(&cmd_path) {
        let (l, c, i) = parse_lines(&content, "cmd", &mut label_counts);
        mtimes.extend(vec![None; l.len()]);
        paths.extend(vec![Some(cmd_path.clone()); l.len()]);
        labels.extend(l);
        commands.extend(c);
        infos.extend(i);
    }

    // Load cached Gists
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
        let gist_dir = proj_dirs.config_dir().join("gists");
        if gist_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(gist_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                    
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let filename = entry.file_name().to_string_lossy().to_string();
                        if content.contains("# termbookman") {
                            let (l, c, i) = parse_lines(&content, "cmd", &mut label_counts);
                            mtimes.extend(vec![mtime; l.len()]);
                            paths.extend(vec![Some(path.clone()); l.len()]);
                            labels.extend(l);
                            commands.extend(c);
                            infos.extend(i);
                        } else if content.trim_start().starts_with("#!") {
                            let name = filename.trim_start_matches("script").trim_start_matches('-').trim().to_string();
                            let dedup_key = format!("__script__{}", name);
                            let count = label_counts.entry(dedup_key).or_insert(0);
                            *count += 1;
                            let final_label = if *count > 1 { format!("{}{}", name, *count) } else { name.clone() };
                            let (desc, code_preview) = parse_script_content(&content);
                            let mut info = "__SCRIPT__".to_string();
                            if !desc.is_empty() {
                                info.push(' ');
                                info.push_str(&desc);
                            }
                            if !code_preview.is_empty() {
                                info.push(' ');
                                info.push_str(&code_preview);
                            }
                            labels.push(final_label.clone());
                            commands.push(path.to_string_lossy().to_string());
                            infos.push(info);
                            mtimes.push(mtime);
                            paths.push(Some(path.clone()));
                        } else {
                            let count = label_counts.entry(filename.clone()).or_insert(0);
                            *count += 1;
                            let final_label = if *count > 1 { format!("{}{}", filename, *count) } else { filename.clone() };
                            labels.push(final_label.clone());
                            commands.push(final_label);
                            infos.push(format!("GIST: {}", filename));
                            mtimes.push(mtime);
                            paths.push(Some(path.clone()));
                        }
                    }
                }
            }
        }
    }
    (labels, commands, infos, mtimes, paths)
}

fn upload_gist(token: &str, file_path: &std::path::Path, remote_filename: &str) -> Result<(), Box<dyn Error>> {
    let content = std::fs::read_to_string(file_path)?;
    
    // We need to find the Gist ID. We can scan cached gists to find which one this file belongs to.
    let client = reqwest::blocking::Client::builder()
        .user_agent("termbookman/0.1.0")
        .build()?;
    
    let url = "https://api.github.com/gists";
    let res = client.get(url)
        .header("Authorization", format!("token {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .send()?;
    
    if !res.status().is_success() {
        return Err(format!("Failed to list gists: {}", res.status()).into());
    }
    
    let gists: serde_json::Value = res.json()?;
    let mut gist_id = None;
    
    if let Some(arr) = gists.as_array() {
        for gist in arr {
            if let Some(files) = gist["files"].as_object() {
                if files.contains_key(remote_filename) {
                    gist_id = gist["id"].as_str().map(|s| s.to_string());
                    break;
                }
            }
        }
    }
    
    let gist_id = gist_id.ok_or_else(|| format!("Gist containing {} not found on GitHub", remote_filename))?;
    let update_url = format!("https://api.github.com/gists/{}", gist_id);
    
    let mut files = serde_json::Map::new();
    let mut file_data = serde_json::Map::new();
    file_data.insert("content".to_string(), serde_json::Value::String(content));
    files.insert(remote_filename.to_string(), serde_json::Value::Object(file_data));
    
    let mut body = serde_json::Map::new();
    body.insert("files".to_string(), serde_json::Value::Object(files));
    
    let res = client.patch(update_url)
        .header("Authorization", format!("token {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .json(&body)
        .send()?;
        
    if res.status().is_success() {
        Ok(())
    } else {
        Err(format!("Failed to update gist: {}", res.status()).into())
    }
}


fn parse_color(color: &str) -> Color {
    match color.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark_gray" | "dark_grey" => Color::DarkGray,
        "light_red" => Color::LightRed,
        "light_green" => Color::LightGreen,
        "light_yellow" => Color::LightYellow,
        "light_blue" => Color::LightBlue,
        "light_magenta" => Color::LightMagenta,
        "light_cyan" => Color::LightCyan,
        "white" => Color::White,
        "orange" => Color::Rgb(255, 165, 0),
        "light_orange" => Color::Rgb(255, 200, 150),
        _ => Color::White,
    }
}

impl App {
    fn is_item_visible(&self, item: &StatusBarItem) -> bool {
        match &item.condition {
            Some(ConditionType::HasGit) => self.git_info.is_some(),
            Some(ConditionType::HasSelection) => self.selection_start.is_some() && self.selection_end.is_some() && !self.is_selecting,
            None => true,
        }
    }

    fn process_prompt(&self, cmd: &mut String) {
        while let Some(start) = cmd.find("<prompt") {
            if let Some(end) = cmd[start..].find(">") {
                let prompt_str = &cmd[start..start + end + 1];
                let prompt_name = prompt_str.trim_matches(|c| c == '<' || c == '>' || c == 'p' || c == 'r' || c == 'o' || c == 'm' || c == 'p' || c == 't' || c == ':');
                
                let input = if let Ok(output) = std::process::Command::new("bash").arg("-c").arg(format!("read -p '{}: ' val && echo $val", prompt_name)).output() {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                } else {
                    String::new()
                };
                cmd.replace_range(start..start + end + 1, &input);
            } else {
                break;
            }
        }
    }
}

impl App {
    fn new(
        pty_write: Box<dyn Write + Send>, 
        master: Box<dyn portable_pty::MasterPty + Send>,
        parser: Arc<Mutex<Parser>>,
        sidebar_items: Vec<String>,
        sidebar_commands: Vec<String>,
        sidebar_infos: Vec<String>,
        sidebar_mtimes: Vec<Option<std::time::SystemTime>>,
        sidebar_paths: Vec<Option<std::path::PathBuf>>,
        shell_pid: u32,
    ) -> App {
        let mut state = ListState::default();
        state.select(Some(0));
        let config = load_config();
        let auth_token = config.auth.personal_access_token.clone().filter(|t| !t.is_empty() && t != "YOUR_TOKEN_HERE");
        App {
            cpu_usage: 0.0,
            mem_usage: (0, 0),
            git_info: None,
            sidebar_state: state,
            sidebar_items,
            sidebar_commands,
            sidebar_infos,
            sidebar_mtimes,
            sidebar_paths,
            sidebar_width: 40,
            is_dragging_sidebar: false,
            is_dragging_term_scrollbar: false,
            show_menu: false,
            mouse_pos: None,
            parser,
            pty_write,
            master,
            should_quit: false,
            last_activity: Instant::now(),
            start_time: Instant::now(),
            is_dragging_scrollbar: false,
            is_dragging_sidebar_scrollbar: false,
            is_selecting: false,
            selection_start: None,
            selection_end: None,
            search_query: String::new(),
            is_search_focused: false,
            sidebar_mode: SidebarMode::Commands,
            history_items: Vec::new(),
            history_commands: Vec::new(),
            gist_items: Vec::new(),
            gist_commands: Vec::new(),
            gist_infos: Vec::new(),
            gist_mtimes: Vec::new(),
            gist_paths: Vec::new(),
            gist_remote_names: Vec::new(),
            shell_pid,
            is_pty_busy: false,
            config: config.clone(),
            show_settings_modal: false,
            github_user_code: None,
            github_verification_uri: None,
            github_device_code: None,
            auth_token: auth_token.clone(),
            login_error: None,
            pat_input: auth_token.unwrap_or_default(),
            is_pat_focused: false,
            editor_input: config.external_editor.clone(),
            is_editor_focused: false,
            loading_gist: false,
            last_click_time: None,
            last_clicked_index: None,
            editing_file: None,
            last_pty_busy: false,
            show_upload_confirm: false,
            pending_gist_file: None,
        }
    }

    fn refresh_history(&mut self) {
        let home = std::env::var("HOME").unwrap_or_default();
        let history_path = std::path::Path::new(&home).join(".bash_history");
        
        self.history_items.clear();
        self.history_commands.clear();

        if let Ok(content) = std::fs::read_to_string(history_path) {
            let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            lines.reverse(); // Newest first
            
            let mut seen = std::collections::HashSet::new();
            for line in lines {
                let trimmed = line.trim();
                if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                    self.history_items.push(trimmed.to_string());
                    self.history_commands.push(trimmed.to_string());
                }
                if self.history_items.len() >= 500 { break; }
            }
        } else {
            // Fallback to 'history' command if file read fails
            if let Ok(output) = Command::new("bash").arg("-c").arg("history").output() {
                let s = String::from_utf8_lossy(&output.stdout);
                let mut lines: Vec<String> = s.lines().map(|s| {
                    let parts: Vec<&str> = s.trim().split_whitespace().collect();
                    if parts.len() > 1 { parts[1..].join(" ") } else { s.to_string() }
                }).collect();
                lines.reverse();
                
                let mut seen = std::collections::HashSet::new();
                for line in lines {
                    if !line.is_empty() && seen.insert(line.to_string()) {
                        self.history_items.push(line.clone());
                        self.history_commands.push(line);
                    }
                    if self.history_items.len() >= 500 { break; }
                }
            }
        }
    }

    fn update_stats(&mut self, sys: &mut System) {
        sys.refresh_cpu();
        sys.refresh_memory();
        sys.refresh_processes();

        self.cpu_usage = sys.global_cpu_info().cpu_usage();
        self.mem_usage = (sys.used_memory(), sys.total_memory());

        // Check if shell has children
        self.is_pty_busy = false;
        let shell_pid = sysinfo::Pid::from_u32(self.shell_pid);
        for process in sys.processes().values() {
            if process.parent() == Some(shell_pid) {
                self.is_pty_busy = true;
                break;
            }
        }

        // Detect transition from busy to not busy (editor closed)
        if self.last_pty_busy && !self.is_pty_busy {
            if let Some((path, old_mtime)) = self.editing_file.take() {
                if let Ok(metadata) = std::fs::metadata(&path) {
                    if let Ok(new_mtime) = metadata.modified() {
                        if new_mtime > old_mtime {
                            log_debug(&format!("File modified: {:?}", path));
                            // Check if it's a Gist file
                            if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
                                let gist_dir = proj_dirs.config_dir().join("gists");
                                if path.starts_with(&gist_dir) {
                                    self.show_upload_confirm = true;
                                    self.pending_gist_file = Some(path);
                                    
                                    // Reload sidebar items to reflect changes in labels/keywords immediately
                                    log_debug("Gist modified, reloading sidebar...");
                                    let exe_path = std::env::current_exe().unwrap_or_default();
                                    let exe_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));
                                    let (l, c, i, m, p) = load_commands(exe_dir);
                                    self.sidebar_items = l;
                                    self.sidebar_commands = c;
                                    self.sidebar_infos = i;
                                    self.sidebar_mtimes = m;
                                    self.sidebar_paths = p;
                                    
                                    // Also update Gist tab items if we have them and they were loaded from this path
                                    for idx in 0..self.gist_paths.len() {
                                        if let Some(gp) = &self.gist_paths[idx] {
                                            if gp == &self.pending_gist_file.clone().unwrap() {
                                                if let Ok(content) = std::fs::read_to_string(gp) {
                                                    if content.trim_start().starts_with("#!") {
                                                        let (desc, preview) = parse_script_content(&content);
                                                        let mut info = "__SCRIPT__".to_string();
                                                        if !desc.is_empty() { info.push(' '); info.push_str(&desc); }
                                                        if !preview.is_empty() { info.push(' '); info.push_str(&preview); }
                                                        self.gist_infos[idx] = info;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Local command file (commands.txt) modified, reload
                                    log_debug("Local commands modified, reloading...");
                                    let exe_path = std::env::current_exe().unwrap_or_default();
                                    let exe_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));
                                    let (l, c, i, m, p) = load_commands(exe_dir);
                                    self.sidebar_items = l;
                                    self.sidebar_commands = c;
                                    self.sidebar_infos = i;
                                    self.sidebar_mtimes = m;
                                    self.sidebar_paths = p;
                                }
                            }
                        }
                    }
                }
            }
        }
        self.last_pty_busy = self.is_pty_busy;

        // Only refresh git info every ~2 seconds (40 * 50ms) to save CPU
        static mut GIT_COUNTER: u32 = 0;
        unsafe {
            GIT_COUNTER += 1;
            if GIT_COUNTER < 4 && self.git_info.is_some() {
                return;
            }
            GIT_COUNTER = 0;
        }

        let branch = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .and_then(|out| if out.status.success() { 
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string()) 
            } else { None });

        if let Some(b) = branch {
            let status = Command::new("git")
                .args(["status", "--porcelain"])
                .output()
                .ok()
                .and_then(|out| if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout);
                    Some(if s.trim().is_empty() { "clean" } else { "modified" })
                } else { None })
                .unwrap_or("unknown");
            self.git_info = Some(format!("GIT: {} ({})", b, status));
        } else {
            self.git_info = None;
        }
    }

    fn copy_selection(&mut self) {
        log_debug("Copy selection triggered");
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            log_debug(&format!("Selection range: {:?} to {:?}", start, end));
            let parser = self.parser.lock().unwrap();
            let screen = parser.screen();
            let (_, cols) = screen.size();
            
            let (s_row, s_col) = start;
            let (e_row, e_col) = end;
            let (min_row, min_col, max_row, max_col) = if s_row < e_row || (s_row == e_row && s_col <= e_col) {
                (s_row, s_col, e_row, e_col)
            } else {
                (e_row, e_col, s_row, s_col)
            };

            let mut text = String::new();
            for r in min_row..=max_row {
                let start_c = if r == min_row { min_col } else { 0 };
                let end_c = if r == max_row { max_col } else { (cols as u16).saturating_sub(1) };
                
                let mut line = String::new();
                for c in start_c..=end_c {
                    if let Some(cell) = screen.cell(r, c) {
                        line.push_str(cell.contents());
                    } else {
                        line.push(' ');
                    }
                }
                text.push_str(line.trim_end());
                if r < max_row {
                    text.push('\n');
                }
            }
            
            log_debug(&format!("Extracted text ({} chars): {:?}", text.len(), text));
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => {
                    if let Err(e) = clipboard.set_text(text) {
                        log_debug(&format!("Clipboard set_text error: {:?}", e));
                    } else {
                        log_debug("Clipboard set_text success");
                    }
                }
                Err(e) => {
                    log_debug(&format!("Clipboard init error: {:?}", e));
                }
            }
        } else {
            log_debug("No selection to copy");
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path.parent().ok_or("Could not find executable directory")?;
    let args: Vec<String> = std::env::args().collect();
    let (labels, sidebar_commands, sidebar_infos, sidebar_mtimes, sidebar_paths) = load_commands(exe_dir);

    if args.len() > 1 {
        let action = &args[1];
        
        if action == "print" && args.len() > 2 {
            if args[2] == "script" && args.len() > 3 {
                // termbookman print script <label>
                let search_label = &args[3];
                for (i, label) in labels.iter().enumerate() {
                    if label == search_label && sidebar_infos[i].starts_with("__SCRIPT__") {
                        let path = &sidebar_commands[i];
                        match std::fs::read_to_string(path) {
                            Ok(content) => print!("{}", content),
                            Err(e) => eprintln!("Error reading script: {}", e),
                        }
                        return Ok(());
                    }
                }
            } else {
                let search_label = &args[2];
                if let Some(i) = labels.iter().position(|l| l == search_label) {
                    if let Some(cmd) = sidebar_commands.get(i) {
                        println!("{}", cmd);
                        return Ok(());
                    }
                }
            }
        } else if action == "script" && args.len() > 2 {
            // termbookman script <label>
            let search_label = &args[2];
            for (i, label) in labels.iter().enumerate() {
                if label == search_label && sidebar_infos[i].starts_with("__SCRIPT__") {
                    let cmd = &sidebar_commands[i];
                    let gist_info = if let Some(mtime) = sidebar_mtimes[i] {
                        format!(" [gist from local cache, saved {} ago]", format_time_passed(mtime))
                    } else {
                        String::new()
                    };
                    println!("\x1b[90mExecuting script{}: {}\x1b[0m", gist_info, cmd);
                    std::process::Command::new("bash").arg(cmd).status()?;
                    return Ok(());
                }
            }
        } else {
            let search_label = action;
            if let Some(i) = labels.iter().position(|l| l == search_label) {
                if let Some(cmd) = sidebar_commands.get(i) {
                    let gist_info = if let Some(mtime) = sidebar_mtimes[i] {
                        format!(" [gist from local cache, saved {} ago]", format_time_passed(mtime))
                    } else {
                        String::new()
                    };
                    
                    if cmd.contains("<prompt:") {
                        let mut final_cmd = cmd.clone();
                        while let Some(start) = final_cmd.find("<prompt:") {
                            if let Some(end) = final_cmd[start..].find(">") {
                                let tag = &final_cmd[start + 8..start + end];
                                let parts: Vec<&str> = tag.split(':').collect();
                                let label = parts.get(0).unwrap_or(&"Value");
                                let default = parts.get(1).unwrap_or(&"");
                                
                                print!("{}: (Default: {}) ", label, default);
                                std::io::stdout().flush()?;
                                
                                let mut input = String::new();
                                std::io::stdin().read_line(&mut input)?;
                                let input = input.trim();
                                
                                let val = if input.is_empty() { default.to_string() } else { input.to_string() };
                                final_cmd.replace_range(start..start + end + 1, &val);
                            } else {
                                break;
                            }
                        }
                        println!("\x1b[90mExecuting{}: {}\x1b[0m", gist_info, final_cmd);
                        std::process::Command::new("bash").arg("-c").arg(&final_cmd).status()?;
                    } else {
                        println!("\x1b[90mExecuting{}: {}\x1b[0m", gist_info, cmd);
                        std::process::Command::new("bash").arg("-c").arg(cmd).status()?;
                    }
                    return Ok(());
                }
            }
        }
        return Ok(());
    }

    log_debug("--- Starting Rust Dashboard ---");

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    
    let chunks = if size.height < 26 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(size)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(2)])
            .split(size)
    };
    let top_chunks = if size.width < 106 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(chunks[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(25)])
            .split(chunks[0])
    };
        
    let term_area = top_chunks[0];
    let rows = term_area.height;
    let cols = term_area.width;

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new("bash");
    cmd.cwd(exe_dir);
    cmd.env("TERM", "xterm-256color");
    cmd.env("LANG", "C.UTF-8");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLUMNS", cols.to_string());
    cmd.env("LINES", rows.to_string());
    let mut child = pair.slave.spawn_command(cmd)?;
    
    let mut reader = pair.master.try_clone_reader()?;
    let pty_write = pair.master.take_writer()?;
    let master = pair.master;
    
    let parser = Arc::new(Mutex::new(Parser::new(rows, cols, 1000)));
    let parser_clone = Arc::clone(&parser);

    let (tx, rx) = mpsc::channel();
    let pty_tx = tx.clone();
    let event_tx = tx.clone();
    let tick_tx = tx.clone();

    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    {
                        let mut p = parser_clone.lock().unwrap();
                        p.process(&buffer[..n]);
                    }
                    let _ = pty_tx.send(Message::PtyData);
                }
                Ok(_) => break,
                Err(_) => break,
            }
        }
    });

    std::thread::spawn(move || {
        loop {
            if let Ok(event) = event::read() {
                if let Err(_) = event_tx.send(Message::Event(event)) {
                    break;
                }
            }
        }
    });

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            if let Err(_) = tick_tx.send(Message::Tick) {
                break;
            }
        }
    });

    let _ = master.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    });

    let mut app = App::new(pty_write, master, parser, labels, sidebar_commands, sidebar_infos, sidebar_mtimes, sidebar_paths, child.process_id().unwrap_or(0));
    
    if app.auth_token.is_some() {
        let _ = tx.send(Message::FetchGists);
    }
    
    let mut sys = System::new_all();

    loop {
        if app.should_quit { break; }
        
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }

        terminal.draw(|f| {
            ui(f, &mut app);
        })?;

        match rx.recv()? {
            Message::PtyData => {
                // Just wake up to redraw
            }
            Message::Tick => {
                app.update_stats(&mut sys);
            }
            Message::DeviceCodeSuccess(device_code, user_code, verification_uri) => {
                app.github_device_code = Some(device_code.clone());
                app.github_user_code = Some(user_code);
                app.github_verification_uri = Some(verification_uri);
                app.login_error = None;

                let tx = tx.clone();
                let client_id = app.config.auth.github_client_id.clone().unwrap_or_default();
                
                std::thread::spawn(move || {
                    let url = "https://github.com/login/oauth/access_token";
                    let client = reqwest::blocking::Client::new();
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        let payload = [
                            ("client_id", client_id.as_str()),
                            ("device_code", device_code.as_str()),
                            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ];
                        
                        match client.post(url).header("Accept", "application/json").form(&payload).send() {
                            Ok(res) => {
                                if let Ok(json) = res.json::<serde_json::Value>() {
                                    if let Some(token) = json["access_token"].as_str() {
                                        let _ = tx.send(Message::AuthSuccess(token.to_string()));
                                        break;
                                    } else if let Some(error) = json["error"].as_str() {
                                        if error == "authorization_pending" {
                                            continue;
                                        } else if error == "slow_down" {
                                            std::thread::sleep(std::time::Duration::from_secs(5));
                                            continue;
                                        } else {
                                            let _ = tx.send(Message::AuthError(error.to_string()));
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                let _ = tx.send(Message::AuthError("Network error polling token.".to_string()));
                                break;
                            }
                        }
                    }
                });
            }
            Message::AuthSuccess(token) => {
                app.auth_token = Some(token.clone());
                app.config.auth.personal_access_token = Some(token);
                let _ = save_config(&app.config);
                app.show_settings_modal = false;
                app.login_error = None;
            }
            Message::AuthError(err) => {
                app.login_error = Some(err);
            }
            Message::FetchGists => {
                log_debug("Message::FetchGists received");
                app.loading_gist = true;
                if let Some(token) = &app.auth_token {
                    let tx = tx.clone();
                    let token = token.clone();
                    std::thread::spawn(move || {
                        let url = "https://api.github.com/gists";
                        let client = reqwest::blocking::Client::builder()
                            .user_agent("termbookman/0.1.0")
                            .build().unwrap();
                        
                        match client.get(url)
                            .header("Authorization", format!("token {}", token))
                            .header("Accept", "application/vnd.github.v3+json")
                            .send() {
                            Ok(res) => {
                                let status = res.status();
                                match res.text() {
                                    Ok(body) => {
                                        log_debug(&format!("Gist fetch response ({}): {}", status, body));
                                        if status.is_success() {
                                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                                                if let Some(gists) = json.as_array() {
                                                    let mut fetched = Vec::new();
                                                    let mut label_counts = std::collections::HashMap::new();
                                                    for gist in gists {
                                                        let description = gist["description"].as_str().unwrap_or("No description").to_string();
                                                        if let Some(files) = gist["files"].as_object() {
                                                            for (filename, file_info) in files {
                                                                let raw_url = file_info["raw_url"].as_str().unwrap_or("");
                                                                if !raw_url.is_empty() {
                                                                    if let Ok(mut resp) = client.get(raw_url).send() {
                                                                        if let Ok(content) = resp.text() {
                                                                            let gist_dir = directories::ProjectDirs::from("", "", "termbookman").map(|pd| pd.config_dir().join("gists")).unwrap_or_else(|| std::path::PathBuf::from("gists"));
                                                                            let _ = std::fs::create_dir_all(&gist_dir);
                                                                            let safe_filename = filename.replace(' ', "_");
                                                                            let gist_file = gist_dir.join(&safe_filename);
                                                                            let _ = std::fs::write(&gist_file, &content);

                                                                            log_debug(&format!("Processing Gist file: {} (saved as {})", filename, safe_filename));
                                                                            if content.contains("# termbookman") {
                                                                                let (labels, cmds, infos) = parse_lines(&content, "cmd", &mut label_counts);
                                                                                for ((label, cmd), info) in labels.into_iter().zip(cmds.into_iter()).zip(infos.into_iter()) {
                                                                                    fetched.push((label, info, cmd, Some(std::time::SystemTime::now()), Some(gist_file.clone()), filename.clone()));
                                                                                }
                                                                            } else if content.trim_start().starts_with("#!") {
                                                                                log_debug("Detected script gist");
                                                                                let name = filename.trim_start_matches("script").trim_start_matches('-').trim_start_matches('_').trim().to_string();
                                                                                let count = label_counts.entry(name.clone()).or_insert(0);
                                                                                *count += 1;
                                                                                let final_label = if *count > 1 { format!("{}{}", name, *count) } else { name };
                                                                                let (desc, code_preview) = parse_script_content(&content);
                                                                                let mut info = "__SCRIPT__".to_string();
                                                                                if !desc.is_empty() {
                                                                                    info.push(' ');
                                                                                    info.push_str(&desc);
                                                                                }
                                                                                if !code_preview.is_empty() {
                                                                                    info.push(' ');
                                                                                    info.push_str(&code_preview);
                                                                                }
                                                                                fetched.push((final_label.clone(), info, gist_file.to_string_lossy().to_string(), Some(std::time::SystemTime::now()), Some(gist_file.clone()), filename.clone()));
                                                                            } else {
                                                                                log_debug("Detected default gist");
                                                                                fetched.push((filename.clone(), filename.clone(), format!("curl -sL {}", raw_url), Some(std::time::SystemTime::now()), Some(gist_file.clone()), filename.clone()));
                                                                            }
                                                                            log_debug(&format!("Current fetched size: {}", fetched.len()));
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    let _ = tx.send(Message::GistsFetched(fetched));
                                                }
                                            }
                                        } else {
                                            let _ = tx.send(Message::AuthError(format!("Gist fetch failed ({})", status)));
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Message::AuthError(format!("Gist response read error: {}", e)));
                                    }
                                }
                            }
                            Err(e) => {
                                log_debug(&format!("Gist fetch network error: {}", e));
                                let _ = tx.send(Message::AuthError(format!("Gist network error: {}", e)));
                            }
                        }
                    });
                } else {
                    app.login_error = Some("Not logged in to GitHub.".to_string());
                    app.show_settings_modal = true;
                }
            }
            Message::GistsFetched(gists) => {
                app.loading_gist = false;
                app.gist_items.clear();
                app.gist_infos.clear();
                app.gist_commands.clear();
                app.gist_mtimes.clear();
                app.gist_paths.clear();
                app.gist_remote_names.clear();
                for (label, info, cmd, mtime, path, remote_name) in gists {
                    app.gist_items.push(label);
                    app.gist_infos.push(info);
                    app.gist_commands.push(cmd);
                    app.gist_mtimes.push(mtime);
                    app.gist_paths.push(path);
                    app.gist_remote_names.push(remote_name);
                }
                app.sidebar_mode = SidebarMode::Gists;
                app.sidebar_state.select(Some(0));
            }
            Message::GistUploadStatus(msg, _is_success) => {
                // Use echo to safely output the message through the shell
                let escaped = msg.replace("'", "'\"'\"'");
                let echo_cmd = format!("echo '{}'\r", escaped);
                let _ = app.pty_write.write_all(echo_cmd.as_bytes());
                let _ = app.pty_write.flush();
            }
            Message::Event(event) => {
                match event {
                    Event::Key(key) => {
                        app.last_activity = Instant::now();
                        
                        if (key.code == KeyCode::Char('C') && key.modifiers.contains(KeyModifiers::CONTROL)) ||
                           (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::SHIFT)) {
                            app.copy_selection();
                            continue;
                        }

                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            match key.code {
                                KeyCode::PageUp => {
                                    let mut p = app.parser.lock().unwrap();
                                    let current = p.screen().scrollback();
                                    p.screen_mut().set_scrollback(current + 20);
                                    continue;
                                }
                                KeyCode::PageDown => {
                                    let mut p = app.parser.lock().unwrap();
                                    let current = p.screen().scrollback();
                                    p.screen_mut().set_scrollback(current.saturating_sub(20));
                                    continue;
                                }
                                _ => {}
                            }
                        }

                        {
                            let mut p = app.parser.lock().unwrap();
                            p.screen_mut().set_scrollback(0);
                        }

                        if app.show_upload_confirm {
                            match key.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') => {
                                    if let (Some(path), Some(token)) = (app.pending_gist_file.take(), app.auth_token.clone()) {
                                        // Find original remote name
                                        let mut remote_name = None;
                                        for (i, p) in app.gist_paths.iter().enumerate() {
                                            if let Some(p) = p {
                                                if p == &path {
                                                    remote_name = Some(app.gist_remote_names[i].clone());
                                                    break;
                                                }
                                            }
                                        }
                                        
                                        if let Some(remote_name) = remote_name {
                                            let tx = tx.clone();
                                            let display_name = remote_name.clone();
                                            std::thread::spawn(move || {
                                                let start_msg = format!("[Uploading Gist: {}]", display_name);
                                                let _ = tx.send(Message::GistUploadStatus(start_msg, true));
                                                
                                                if let Err(e) = upload_gist(&token, &path, &remote_name) {
                                                    log_debug(&format!("Gist upload error: {}", e));
                                                    let msg = format!("✗ Gist upload failed: {}", e);
                                                    let _ = tx.send(Message::GistUploadStatus(msg, false));
                                                } else {
                                                    log_debug("Gist uploaded successfully");
                                                    let msg = format!("✓ Gist updated: {}", display_name);
                                                    let _ = tx.send(Message::GistUploadStatus(msg, true));
                                                }
                                            });
                                        }
                                    }
                                    app.show_upload_confirm = false;
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    app.show_upload_confirm = false;
                                    app.pending_gist_file = None;
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if app.show_settings_modal {
                            if app.is_pat_focused {
                                match key.code {
                                    KeyCode::Esc => { app.is_pat_focused = false; }
                                    KeyCode::Backspace => { app.pat_input.pop(); }
                                    KeyCode::Char(c) => { app.pat_input.push(c); }
                                    KeyCode::Enter => {
                                        if !app.pat_input.trim().is_empty() {
                                            let token = app.pat_input.trim().to_string();
                                            app.auth_token = Some(token.clone());
                                            app.config.auth.personal_access_token = Some(token);
                                            let _ = save_config(&app.config);
                                            app.is_pat_focused = false;
                                            app.login_error = None;
                                        }
                                    }
                                    _ => {}
                                }
                            } else if app.is_editor_focused {
                                match key.code {
                                    KeyCode::Esc => { app.is_editor_focused = false; }
                                    KeyCode::Backspace => { app.editor_input.pop(); }
                                    KeyCode::Char(c) => { app.editor_input.push(c); }
                                    KeyCode::Enter => {
                                        if !app.editor_input.trim().is_empty() {
                                            app.config.external_editor = app.editor_input.trim().to_string();
                                            let _ = save_config(&app.config);
                                            app.is_editor_focused = false;
                                        }
                                    }
                                    _ => {}
                                }
                            } else if let KeyCode::Esc = key.code {
                                app.show_settings_modal = false;
                            }
                            continue;
                        }

                        if app.is_search_focused {
                            match key.code {
                                KeyCode::Esc => { app.is_search_focused = false; }
                                KeyCode::Backspace => { 
                                    app.search_query.pop(); 
                                    app.sidebar_state.select(Some(0));
                                }
                                KeyCode::Char(c) => { 
                                    app.search_query.push(c); 
                                    app.sidebar_state.select(Some(0));
                                }
                                KeyCode::Enter => { app.is_search_focused = false; }
                                _ => {}
                            }
                            continue;
                        }

                        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            let _ = app.pty_write.write_all(b"\x03");
                            let _ = app.pty_write.flush();
                            continue;
                        }

                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match key.code {
                                KeyCode::Char(c) => {
                                    let n = c as u8;
                                    if (97..=122).contains(&n) {
                                        let seq = format!("\x1b{}", (n - 96) as char);
                                        let _ = app.pty_write.write_all(seq.as_bytes());
                                    } else if c == '[' {
                                        let _ = app.pty_write.write_all(b"\x1b");
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Char(c) => { let _ = write!(app.pty_write, "{}", c); }
                                KeyCode::Enter => { let _ = app.pty_write.write_all(b"\r"); }
                                KeyCode::Backspace => { let _ = app.pty_write.write_all(b"\x08"); }
                                KeyCode::Tab => { let _ = app.pty_write.write_all(b"\x09"); }
                                KeyCode::Esc => { let _ = app.pty_write.write_all(b"\x1b"); }
                                KeyCode::Up => { let _ = app.pty_write.write_all(b"\x1b[A"); }
                                KeyCode::Down => { let _ = app.pty_write.write_all(b"\x1b[B"); }
                                KeyCode::Right => { let _ = app.pty_write.write_all(b"\x1b[C"); }
                                KeyCode::Left => { let _ = app.pty_write.write_all(b"\x1b[D"); }
                                KeyCode::Home => { let _ = app.pty_write.write_all(b"\x1b[H"); }
                                KeyCode::End => { let _ = app.pty_write.write_all(b"\x1b[F"); }
                                KeyCode::Delete => { let _ = app.pty_write.write_all(b"\x1b[3~"); }
                                KeyCode::F(n) => {
                                    let seq = match n {
                                        1..=4 => format!("\x1bO{}", (n as u8 + 79) as char),
                                        5 => "\x1b[15~".to_string(),
                                        6 => "\x1b[17~".to_string(),
                                        7 => "\x1b[18~".to_string(),
                                        8 => "\x1b[19~".to_string(),
                                        9 => "\x1b[20~".to_string(),
                                        10 => "\x1b[21~".to_string(),
                                        11 => "\x1b[23~".to_string(),
                                        12 => "\x1b[24~".to_string(),
                                        _ => "".to_string(),
                                    };
                                    let _ = write!(app.pty_write, "{}", seq);
                                }
                                _ => {}
                            }
                        }
                        let _ = app.pty_write.flush();
                    }
                    Event::Mouse(mouse) => {
                        log_debug("Mouse event received in event loop");
                        app.last_activity = Instant::now();
                        app.mouse_pos = Some((mouse.column, mouse.row));
                        handle_click(&mut app, mouse, terminal.size()?, &tx);
                    }
                    Event::Resize(_w, _h) => {
                        // Redraw handled by recv
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = child.kill();
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    
    while event::poll(Duration::from_millis(10))? {
        let _ = event::read()?;
    }

    disable_raw_mode()?;
    terminal.show_cursor()?;
    log_debug("App exited cleanly.");
    Ok(())
}

fn handle_click(app: &mut App, mouse: MouseEvent, size: Rect, tx: &mpsc::Sender<Message>) {
    log_debug(&format!("Click at ({}, {}). Screen size: {:?}, sidebar_area: {:?}", mouse.column, mouse.row, size, Rect::new(size.width.saturating_sub(app.sidebar_width), 0, app.sidebar_width, size.height)));
    
    if app.show_upload_confirm {
        let area = centered_rect_fixed(50, 10, size);
        if area.contains(Position::new(mouse.column, mouse.row)) {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let block = Block::default().borders(Borders::ALL);
                let inner = block.inner(area);
                
                // The text is rendered centered in 'inner'
                // "[Y] Yes, Upload    [N] No, Keep Local" is at the bottom (line 4 of 6 inside inner)
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1), // filename
                        Constraint::Length(2), // question
                        Constraint::Length(1), // spacer
                        Constraint::Length(1), // choices
                        Constraint::Min(0),
                    ])
                    .split(inner);
                
                if chunks[3].contains(Position::new(mouse.column, mouse.row)) {
                    let choices_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(50),
                            Constraint::Percentage(50),
                        ])
                        .split(chunks[3]);
                    
                    if choices_chunks[0].contains(Position::new(mouse.column, mouse.row)) {
                        // Clicked [Y]
                        if let (Some(path), Some(token)) = (app.pending_gist_file.take(), app.auth_token.clone()) {
                            let mut remote_name = None;
                            for (i, p) in app.gist_paths.iter().enumerate() {
                                if let Some(p) = p {
                                    if p == &path {
                                        remote_name = Some(app.gist_remote_names[i].clone());
                                        break;
                                    }
                                }
                            }
                            if let Some(remote_name) = remote_name {
                                let tx = tx.clone();
                                let display_name = remote_name.clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(Message::GistUploadStatus(format!("[Uploading Gist: {}]", display_name), true));
                                    if let Err(e) = upload_gist(&token, &path, &remote_name) {
                                        let _ = tx.send(Message::GistUploadStatus(format!("✗ Gist upload failed: {}", e), false));
                                    } else {
                                        let _ = tx.send(Message::GistUploadStatus(format!("✓ Gist updated: {}", display_name), true));
                                    }
                                });
                            }
                        }
                        app.show_upload_confirm = false;
                    } else if choices_chunks[1].contains(Position::new(mouse.column, mouse.row)) {
                        // Clicked [N]
                        app.show_upload_confirm = false;
                        app.pending_gist_file = None;
                    }
                }
            }
        }
        return;
    }

    if app.show_settings_modal {
        let area = centered_rect_fixed(50, 20, size);
        if area.contains(Position::new(mouse.column, mouse.row)) {
            let block = Block::default().borders(Borders::ALL);
            let inner_area = block.inner(area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(1), // Header
                    Constraint::Length(2), // Device Flow
                    Constraint::Length(1), // PAT Header
                    Constraint::Length(3), // PAT Input
                    Constraint::Length(1), // Editor Header
                    Constraint::Length(3), // Editor Input
                    Constraint::Min(0),    // Status
                ])
                .split(inner_area);
            
            if chunks[3].contains(Position::new(mouse.column, mouse.row)) {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    app.is_pat_focused = true;
                    app.is_editor_focused = false;
                }
                return;
            }
            if chunks[5].contains(Position::new(mouse.column, mouse.row)) {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    app.is_editor_focused = true;
                    app.is_pat_focused = false;
                }
                return;
            }
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            app.is_pat_focused = false;
            app.is_editor_focused = false;
            if !area.contains(Position::new(mouse.column, mouse.row)) {
                app.show_settings_modal = false;
            }
        }
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(2)])
        .split(size);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(app.sidebar_width)])
        .split(chunks[0]);

    let term_area = top_chunks[0];
    let right_pane = top_chunks[1];

    let right_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // Terminal Scrollbar / Resize Handle
            Constraint::Length(0), 
            Constraint::Fill(1),   // Sidebar Items
            Constraint::Length(1), // Sidebar Scrollbar
        ])
        .split(right_pane);

    let scrollbar_area = right_layout[0];
    let sidebar_area = right_layout[2];
    let sidebar_scrollbar_area = right_layout[3];

    let (mouse_mode, mouse_enc) = {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        (screen.mouse_protocol_mode(), screen.mouse_protocol_encoding())
    };

    if term_area.contains(Position::new(mouse.column, mouse.row)) && mouse_mode != MouseProtocolMode::None {
        let tx = (mouse.column.saturating_sub(term_area.x) + 1) as i32;
        let ty = (mouse.row.saturating_sub(term_area.y) + 1) as i32;
        
        let btn = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => 0,
            MouseEventKind::Down(MouseButton::Middle) => 1,
            MouseEventKind::Down(MouseButton::Right) => 2,
            MouseEventKind::Up(MouseButton::Left) => 0,
            MouseEventKind::Up(MouseButton::Middle) => 1,
            MouseEventKind::Up(MouseButton::Right) => 2,
            MouseEventKind::Drag(MouseButton::Left) => 32,
            MouseEventKind::Drag(MouseButton::Middle) => 33,
            MouseEventKind::Drag(MouseButton::Right) => 34,
            MouseEventKind::Moved => 35,
            MouseEventKind::ScrollUp => 64,
            MouseEventKind::ScrollDown => 65,
            _ => 0,
        };

        let is_up = matches!(mouse.kind, MouseEventKind::Up(_));
        let suffix = if is_up { 'm' } else { 'M' };
        
        if mouse_enc == MouseProtocolEncoding::Sgr {
            let seq = format!("\x1b[<{};{};{}{}", btn, tx, ty, suffix);
            let _ = app.pty_write.write_all(seq.as_bytes());
            let _ = app.pty_write.flush();
            return;
        }
    }

    if let MouseEventKind::Up(MouseButton::Left) = mouse.kind {
        app.is_dragging_sidebar = false;
        app.is_dragging_term_scrollbar = false;
        app.is_dragging_sidebar_scrollbar = false;
    }

    if app.is_dragging_term_scrollbar {
        let mut p = app.parser.lock().unwrap();
        let screen = p.screen();
        let history_len = screen.scrollback_len();
        if history_len > 0 {
            let relative_y = mouse.row.saturating_sub(scrollbar_area.y) as f32;
            let percent = relative_y / scrollbar_area.height as f32;
            let new_pos = (percent * history_len as f32) as usize;
            let target_offset = history_len.saturating_sub(new_pos);
            p.screen_mut().set_scrollback(target_offset);
        }
        return;
    }

    if app.is_dragging_sidebar {
        let new_width = size.width.saturating_sub(mouse.column);
        app.sidebar_width = new_width.clamp(10, size.width.saturating_sub(20));
        return;
    }

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        if scrollbar_area.contains(Position::new(mouse.column, mouse.row)) {
            if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                app.is_dragging_sidebar = true;
            } else {
                app.is_dragging_term_scrollbar = true;
            }
            return;
        }
    }

    let status_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(chunks[1]);
    
    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        let bars = vec![
            (app.config.statusbar.upper.clone(), status_chunks[0]),
            (app.config.statusbar.lower.clone(), status_chunks[1]),
        ];

        for (bar_items, bar_chunk) in bars {
            let mut constraints = Vec::new();
            let mut visible_items = Vec::new();

            for item in bar_items {
                if app.is_item_visible(&item) {
                    let width = if let Some(w) = item.width {
                        Constraint::Length(w)
                    } else if item.type_ == ItemType::Spacer || item.type_ == ItemType::GitInfo || item.type_ == ItemType::SelectedCommandInfo {
                        Constraint::Fill(1)
                    } else if let Some(label) = &item.label {
                        Constraint::Length(label.len() as u16)
                    } else {
                        Constraint::Min(0)
                    };
                    constraints.push(width);
                    visible_items.push(item);
                }
            }

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(bar_chunk);

            for (i, item) in visible_items.into_iter().enumerate() {
                let chunk = chunks[i];
                if chunk.contains(Position::new(mouse.column, mouse.row)) {
                    if let Some(action) = item.action {
                        match action {
                            ActionType::ToggleMenu => {
                                app.show_menu = !app.show_menu;
                            }
                            ActionType::CopySelection => {
                                app.copy_selection();
                            }
                            ActionType::SendCommand => {
                                if let Some(cmd) = item.command {
                                    let _ = app.pty_write.write_all(cmd.as_bytes());
                                    let _ = app.pty_write.flush();
                                }
                            }
                            ActionType::Quit => {
                                app.should_quit = true;
                            }
                            ActionType::ShowSettingsModal => {
                                app.show_settings_modal = !app.show_settings_modal;
                                if app.show_settings_modal {
                                    app.editor_input = app.config.external_editor.clone();
                                    if let Some(token) = &app.auth_token {
                                        app.pat_input = token.clone();
                                    } else if let Some(client_id) = &app.config.auth.github_client_id {
                                        app.login_error = Some("Fetching GitHub code...".to_string());
                                        let client_id = client_id.clone();
                                        let scope = app.config.auth.scope.clone();
                                        let tx = tx.clone();
                                        std::thread::spawn(move || {
                                            let url = "https://github.com/login/device/code";
                                            let client = reqwest::blocking::Client::new();
                                            let mut params = vec![("client_id", client_id.as_str())];
                                            if !scope.is_empty() {
                                                params.push(("scope", scope.as_str()));
                                            }
                                            match client.post(url).header("Accept", "application/json").query(&params).send() {
                                                Ok(res) => {
                                                    let status = res.status();
                                                    if let Ok(json) = res.json::<serde_json::Value>() {
                                                        if let (Some(device), Some(user), Some(uri)) = (json["device_code"].as_str(), json["user_code"].as_str(), json["verification_uri"].as_str()) {
                                                            let _ = tx.send(Message::DeviceCodeSuccess(device.to_string(), user.to_string(), uri.to_string()));
                                                            return;
                                                        }
                                                        if let Some(error) = json["error"].as_str() {
                                                            let desc = json["error_description"].as_str().unwrap_or(error);
                                                            let _ = tx.send(Message::AuthError(format!("GitHub: {}", desc)));
                                                            log_debug(&format!("GitHub auth error: {} - {}", error, desc));
                                                            return;
                                                        }
                                                        log_debug(&format!("Failed to parse GitHub JSON (status {}): {}", status, json));
                                                    } else {
                                                        log_debug(&format!("Failed to parse GitHub response as JSON (status {})", status));
                                                    }
                                                    let _ = tx.send(Message::AuthError(format!("Failed to parse device code (Status: {})", status)));
                                                }
                                                Err(_) => {
                                                    let _ = tx.send(Message::AuthError("Network error fetching code.".to_string()));
                                                }
                                            }
                                        });
                                    } else {
                                        app.login_error = Some("No github_client_id found in config.".to_string());
                                    }
                                }
                            }
                            ActionType::FetchGists => {
                                let _ = tx.send(Message::FetchGists);
                            }
                        }
                    }
                }
            }
        }
    }
    
    let is_app_active = {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        screen.alternate_screen() || screen.hide_cursor() || screen.application_cursor()
    };
    let is_sidebar_locked = is_app_active || app.is_pty_busy;

    let tabs_area = Rect::new(sidebar_area.x, sidebar_area.y, sidebar_area.width, 1);
    let search_area = Rect::new(sidebar_area.x, sidebar_area.y + 1, sidebar_area.width, 1);
    let list_area = Rect::new(sidebar_area.x, sidebar_area.y + 2, sidebar_area.width, sidebar_area.height.saturating_sub(2));

    if is_sidebar_locked {
        return;
    }

    if tabs_area.contains(Position::new(mouse.column, mouse.row)) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let tabs_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
                .split(tabs_area);
            
            if tabs_chunks[0].contains(Position::new(mouse.column, mouse.row)) {
                app.sidebar_mode = SidebarMode::Commands;
            } else if tabs_chunks[1].contains(Position::new(mouse.column, mouse.row)) {
                app.sidebar_mode = SidebarMode::History;
                app.refresh_history();
            } else if tabs_chunks[2].contains(Position::new(mouse.column, mouse.row)) {
                app.sidebar_mode = SidebarMode::Gists;
            }
            app.sidebar_state.select(Some(0));
        }
        return;
    }

    if search_area.contains(Position::new(mouse.column, mouse.row)) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            app.is_search_focused = true;
        }
        return;
    } else if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        app.is_search_focused = false;
    }
    if list_area.contains(Position::new(mouse.column, mouse.row)) && !is_app_active {
        let (items, infos, commands) = match app.sidebar_mode {
            SidebarMode::Commands => (&app.sidebar_items, Some(&app.sidebar_infos), &app.sidebar_commands),
            SidebarMode::History => (&app.history_items, None, &app.history_commands),
            SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
        };

        let filtered: Vec<(usize, &String)> = items.iter().enumerate()
            .filter(|(idx, label)| {
                let matches_label = label.to_lowercase().contains(&app.search_query.to_lowercase());
                let matches_info = if let Some(infos) = infos {
                    infos[*idx].to_lowercase().contains(&app.search_query.to_lowercase())
                } else {
                    false
                };
                matches_label || matches_info
            })
            .collect();

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let current = app.sidebar_state.selected().unwrap_or(0);
                if current > 0 {
                    app.sidebar_state.select(Some(current - 1));
                }
            }
            MouseEventKind::ScrollDown => {
                let current = app.sidebar_state.selected().unwrap_or(0);
                if current + 1 < filtered.len() {
                    app.sidebar_state.select(Some(current + 1));
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let sidebar_y = mouse.row.saturating_sub(list_area.y);
                let offset = app.sidebar_state.offset();
                let index = (sidebar_y as usize).saturating_add(offset);

                if index < filtered.len() {
                    let (original_index, label) = filtered[index];
                    let now = Instant::now();
                    let is_double_click = if let (Some(last_time), Some(last_idx)) = (app.last_click_time, app.last_clicked_index) {
                        now.duration_since(last_time) < Duration::from_millis(300) && last_idx == index
                    } else { false };

                    if is_double_click {
                        app.last_click_time = None;
                        app.last_clicked_index = None;
                        
                        let (path, cmd_str) = match app.sidebar_mode {
                            SidebarMode::Commands => {
                                let p = &app.sidebar_paths[original_index];
                                (p.clone(), p.as_ref().map(|path| {
                                    let p_str = path.to_string_lossy().replace("'", "'\\''");
                                    format!("{} '{}'\r", app.config.external_editor, p_str)
                                }).unwrap_or_default())
                            },
                            SidebarMode::Gists => {
                                let p = &app.gist_paths[original_index];
                                (p.clone(), p.as_ref().map(|path| {
                                    let p_str = path.to_string_lossy().replace("'", "'\\''");
                                    format!("{} '{}'\r", app.config.external_editor, p_str)
                                }).unwrap_or_default())
                            },
                            SidebarMode::History => (None, String::new()),
                        };
                        
                        if let (Some(p), _) = (&path, &cmd_str) {
                            if let Ok(metadata) = std::fs::metadata(p) {
                                if let Ok(mtime) = metadata.modified() {
                                    app.editing_file = Some((p.clone(), mtime));
                                }
                            }
                        }

                        if !cmd_str.is_empty() {
                            let _ = app.pty_write.write_all(cmd_str.as_bytes());
                            let _ = app.pty_write.flush();
                        }
                    } else {
                        app.sidebar_state.select(Some(index));
                        app.last_click_time = Some(now);
                        app.last_clicked_index = Some(index);
                        
                        let sidebar_x = mouse.column.saturating_sub(list_area.x);
                        if sidebar_x <= 1 {
                            if label.starts_with("GIST: ") && app.sidebar_mode == SidebarMode::Gists {
                                 let _ = tx.send(Message::FetchGists);
                            } else {
                        let (items, infos, _) = match app.sidebar_mode {
                            SidebarMode::Commands => (&app.sidebar_items, Some(&app.sidebar_infos), &app.sidebar_commands),
                            SidebarMode::History => (&app.history_items, None, &app.history_commands),
                            SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
                        };
                        let is_script = infos.map(|inf| inf[original_index].starts_with("__SCRIPT__")).unwrap_or(false);
                        let cmd_str = match app.sidebar_mode {
                            SidebarMode::History => format!("{}\r", app.history_commands[original_index]),
                            SidebarMode::Gists | SidebarMode::Commands => {
                                let label = &items[original_index];
                                let exe = std::env::current_exe().unwrap_or_default();
                                if is_script {
                                    format!("{} script {}\r", exe.display(), label)
                                } else {
                                    format!("{} {}\r", exe.display(), label)
                                }
                            }
                        };
                        let _ = app.pty_write.write_all(cmd_str.as_bytes());
                        let _ = app.pty_write.flush();
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    } else if sidebar_scrollbar_area.contains(Position::new(mouse.column, mouse.row)) || app.is_dragging_sidebar_scrollbar {
        let (items, infos, _) = match app.sidebar_mode {
            SidebarMode::Commands => (&app.sidebar_items, Some(&app.sidebar_infos), &app.sidebar_commands),
            SidebarMode::History => (&app.history_items, None, &app.history_commands),
            SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
        };

        let filtered: Vec<(usize, &String)> = items.iter().enumerate()
            .filter(|(idx, label)| {
                let matches_label = label.to_lowercase().contains(&app.search_query.to_lowercase());
                let matches_info = if let Some(infos) = infos {
                    infos[*idx].to_lowercase().contains(&app.search_query.to_lowercase())
                } else {
                    false
                };
                matches_label || matches_info
            })
            .collect();

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = true;
                let track_height = sidebar_scrollbar_area.height as f32;
                let click_pos = (mouse.row.saturating_sub(sidebar_scrollbar_area.y)) as f32;
                if track_height > 0.0 && !filtered.is_empty() {
                    let visual_idx = ((filtered.len() as f32 * (click_pos / track_height)) as usize).min(filtered.len() - 1);
                    let (original_index, _) = filtered[visual_idx];
                    app.sidebar_state.select(Some(original_index));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_dragging_sidebar_scrollbar => {
                let track_height = sidebar_scrollbar_area.height as f32;
                let mouse_y = mouse.row.clamp(sidebar_scrollbar_area.y, sidebar_scrollbar_area.bottom().saturating_sub(1));
                let click_pos = (mouse_y.saturating_sub(sidebar_scrollbar_area.y)) as f32;
                if track_height > 0.0 && !filtered.is_empty() {
                    let visual_idx = ((filtered.len() as f32 * (click_pos / track_height)) as usize).min(filtered.len() - 1);
                    let (original_index, _) = filtered[visual_idx];
                    app.sidebar_state.select(Some(original_index));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = false;
            }
            _ => {}
        }
    } else if scrollbar_area.contains(Position::new(mouse.column, mouse.row)) || app.is_dragging_scrollbar {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_scrollbar = true;
                let mut parser = app.parser.lock().unwrap();
                let screen = parser.screen_mut();
                let current = screen.scrollback();
                
                if mouse.row == scrollbar_area.y {
                    screen.set_scrollback(current + 3);
                } else if mouse.row == scrollbar_area.bottom() - 1 {
                    screen.set_scrollback(current.saturating_sub(3));
                } else {
                    let track_height = scrollbar_area.height.saturating_sub(2) as f32;
                    let click_pos = (mouse.row.saturating_sub(scrollbar_area.y + 1)) as f32;
                    let history_len = screen.scrollback_len() as f32;
                    if track_height > 0.0 {
                        let target_scroll = (history_len * (1.0 - (click_pos / track_height))) as usize;
                        screen.set_scrollback(target_scroll);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_dragging_scrollbar => {
                let mut parser = app.parser.lock().unwrap();
                let screen = parser.screen_mut();
                let track_height = scrollbar_area.height.saturating_sub(2) as f32;
                let mouse_y = mouse.row.clamp(scrollbar_area.y + 1, scrollbar_area.bottom().saturating_sub(2));
                let click_pos = (mouse_y.saturating_sub(scrollbar_area.y + 1)) as f32;
                let history_len = screen.scrollback_len() as f32;
                
                if track_height > 0.0 {
                    let target_scroll = (history_len * (1.0 - (click_pos / track_height))) as usize;
                    screen.set_scrollback(target_scroll);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.is_dragging_scrollbar = false;
            }
            _ => {}
        }
    } else if term_area.contains(Position::new(mouse.column, mouse.row)) || app.is_selecting {
        let (parser, writer) = (&app.parser, &mut app.pty_write);
        let mut parser = parser.lock().unwrap();
        let screen = parser.screen_mut();

        let tx = mouse.column.saturating_sub(term_area.x);
        let ty = mouse.row.saturating_sub(term_area.y);

        if screen.mouse_protocol_mode() == MouseProtocolMode::None {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    app.is_selecting = true;
                    app.selection_start = Some((ty, tx));
                    app.selection_end = Some((ty, tx));
                    return;
                }
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_selecting => {
                    app.selection_end = Some((ty, tx));
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) if app.is_selecting => {
                    app.is_selecting = false;
                    if app.selection_start == app.selection_end {
                        app.selection_start = None;
                        app.selection_end = None;
                    }
                    return;
                }
                _ => {}
            }
        }

        if screen.mouse_protocol_encoding() == MouseProtocolEncoding::Sgr {
            let x = mouse.column.saturating_sub(term_area.x) + 1;
            let y = mouse.row.saturating_sub(term_area.y) + 1;
            
            let (button, event_char) = match mouse.kind {
                MouseEventKind::Down(btn) => (btn, 'M'),
                MouseEventKind::Up(btn) => (btn, 'm'),
                MouseEventKind::ScrollUp => (MouseButton::Left, 'M'),
                MouseEventKind::ScrollDown => (MouseButton::Left, 'M'),
                MouseEventKind::Drag(btn) => (btn, 'M'),
                _ => return, 
            };
            
            let button_code = match mouse.kind {
                MouseEventKind::ScrollUp => 64,
                MouseEventKind::ScrollDown => 65,
                MouseEventKind::Drag(MouseButton::Left) => 32,
                MouseEventKind::Drag(MouseButton::Middle) => 33,
                MouseEventKind::Drag(MouseButton::Right) => 34,
                _ => match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                }
            };

            let seq = format!("\x1b[<{};{};{}{}", button_code, x, y, event_char);
            let _ = writer.write_all(seq.as_bytes());
            let _ = writer.flush();
        } else {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    let current = screen.scrollback();
                    screen.set_scrollback(current + 3);
                }
                MouseEventKind::ScrollDown => {
                    let current = screen.scrollback();
                    screen.set_scrollback(current.saturating_sub(3));
                }
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let size = f.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(2)])
        .split(size);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(app.sidebar_width)])
        .split(chunks[0]);

    let term_area = top_chunks[0];
    let right_pane = top_chunks[1];
    
    let right_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // Terminal Scrollbar / Resize Handle
            Constraint::Length(0), 
            Constraint::Fill(1),   // Sidebar Items
            Constraint::Length(1), // Sidebar Scrollbar
        ])
        .split(right_pane);

    let scrollbar_area = right_layout[0];
    let sidebar_area = right_layout[2];
    let sidebar_scrollbar_area = right_layout[3];
    
    f.render_widget(Block::default().style(Style::default().bg(Color::Black)), right_pane);

    {
        let mut parser = app.parser.lock().unwrap();
        let (p_rows, p_cols) = parser.screen().size();
        if term_area.height != p_rows || term_area.width != p_cols {
            let _ = app.master.resize(PtySize {
                rows: term_area.height,
                cols: term_area.width,
                pixel_width: 0,
                pixel_height: 0,
            });
            parser.screen_mut().set_size(term_area.height, term_area.width);
        }
    }

    let term_bg = if app.is_search_focused { Color::Rgb(30, 30, 30) } else { Color::Black };
    for y in term_area.top()..term_area.bottom() {
        for x in term_area.left()..term_area.right() {
            let cell = f.buffer_mut().get_mut(x, y);
            cell.reset();
            cell.set_bg(term_bg);
        }
    }

    {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        
        for row in 0..term_area.height {
            for col in 0..term_area.width {
                let x = term_area.x + col;
                let y = term_area.y + row;
                
                if let Some(cell) = screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }

                    let mut style = Style::default().bg(term_bg);
                    match cell.fgcolor() {
                        vt100::Color::Rgb(r, g, b) => { style = style.fg(Color::Rgb(r, g, b)); }
                        vt100::Color::Idx(i) => { style = style.fg(Color::Indexed(i)); }
                        _ => {} 
                    }
                    match cell.bgcolor() {
                        vt100::Color::Rgb(r, g, b) => { style = style.bg(Color::Rgb(r, g, b)); }
                        vt100::Color::Idx(i) => { style = style.bg(Color::Indexed(i)); }
                        _ => {} 
                    }
                    
                    if cell.bold() { style = style.add_modifier(Modifier::BOLD); }
                    if cell.italic() { style = style.add_modifier(Modifier::ITALIC); }
                    if cell.inverse() { style = style.add_modifier(Modifier::REVERSED); }
                    if cell.underline() { style = style.add_modifier(Modifier::UNDERLINED); }
                    
                    if let (Some(start), Some(end)) = (app.selection_start, app.selection_end) {
                        let (s_row, s_col) = start;
                        let (e_row, e_col) = end;
                        let (min_row, min_col, max_row, max_col) = if s_row < e_row || (s_row == e_row && s_col <= e_col) {
                            (s_row, s_col, e_row, e_col)
                        } else {
                            (e_row, e_col, s_row, s_col)
                        };
                        let is_selected = if row > min_row && row < max_row {
                            true
                        } else if row == min_row && row == max_row {
                            col >= min_col && col <= max_col
                        } else if row == min_row {
                            col >= min_col
                        } else if row == max_row {
                            col <= max_col
                        } else {
                            false
                        };
                        if is_selected {
                            style = style.bg(Color::Rgb(80, 80, 80)).fg(Color::White);
                        }
                    }

                    let symbol = cell.contents();
                    let draw_sym = if symbol.is_empty() { " " } else { symbol };
                    f.buffer_mut().set_string(x, y, draw_sym, style);
                } else {
                    f.buffer_mut().get_mut(x, y).set_symbol(" ").set_style(Style::default());
                }
            }
        }

        let (cursor_row, cursor_col) = screen.cursor_position();
        let cx = (term_area.x + cursor_col).min(term_area.right().saturating_sub(1));
        let cy = (term_area.y + cursor_row).min(term_area.bottom().saturating_sub(1));
        f.set_cursor(cx, cy);

        let is_app_active = screen.alternate_screen() || screen.hide_cursor() || screen.application_cursor();
        let is_sidebar_locked = is_app_active || app.is_pty_busy;
        
        let sidebar_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Tabs
                Constraint::Length(1), // Search Bar
                Constraint::Min(0)     // List
            ])
            .split(sidebar_area);
        let tabs_area = sidebar_layout[0];
        let search_area = sidebar_layout[1];
        let list_area = sidebar_layout[2];
        
        static mut LOGGED_LAYOUT: bool = false;
        unsafe {
            if !LOGGED_LAYOUT {
                log_debug(&format!("Sidebar: {:?}, Tabs: {:?}, List: {:?}", sidebar_area, tabs_area, list_area));
                LOGGED_LAYOUT = true;
            }
        }
        
        if is_sidebar_locked {
             f.render_widget(
                Paragraph::new(Span::styled("  SIDEBAR DISABLED  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)))
                    .style(Style::default().bg(Color::Black))
                    .alignment(ratatui::layout::Alignment::Center),
                tabs_area
            );
        } else {
            let tabs_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
                .split(tabs_area);
            
            let modes = [
                (SidebarMode::Commands, " COMMANDS ", Color::Green),
                (SidebarMode::History, " HISTORY ", Color::Rgb(255, 180, 100)),
                (SidebarMode::Gists, " GISTS ", Color::Yellow),
            ];

            for (i, (mode, label, color)) in modes.iter().enumerate() {
                let is_active = app.sidebar_mode == *mode;
                let is_hovered = if let Some((mx, my)) = app.mouse_pos {
                    tabs_chunks[i].contains(Position::new(mx, my))
                } else { false };

                let style = if is_active {
                    Style::default().fg(Color::Black).bg(*color).add_modifier(Modifier::BOLD)
                } else if is_hovered {
                    Style::default().fg(*color).bg(Color::Rgb(50, 50, 50))
                } else {
                    Style::default().fg(*color).bg(Color::Black)
                };

                f.render_widget(
                    Paragraph::new(Span::styled(*label, style))
                        .alignment(ratatui::layout::Alignment::Center),
                    tabs_chunks[i]
                );
            }
        }
        
        let search_style = if is_sidebar_locked {
            Style::default().fg(Color::DarkGray).bg(Color::Black)
        } else if app.is_search_focused {
            Style::default().fg(Color::Yellow).bg(Color::Rgb(30, 30, 30))
        } else {
            Style::default().fg(Color::DarkGray).bg(Color::Black)
        };
 
        let now = Instant::now().duration_since(app.start_time).as_millis();
        let cursor_char = if app.is_search_focused && (now / 500) % 2 == 0 { "_" } else { " " };
        
        let (items, infos, commands) = match app.sidebar_mode {
            SidebarMode::Commands => (&app.sidebar_items, Some(&app.sidebar_infos), &app.sidebar_commands),
            SidebarMode::History => (&app.history_items, None, &app.history_commands),
            SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
        };

        let search_text = if app.search_query.is_empty() && !app.is_search_focused {
            format!(" Search {} items .. ", items.len())
        } else {
            format!(" {}{} ", app.search_query, cursor_char)
        };
        f.render_widget(Paragraph::new(search_text).style(search_style), search_area);

        let filtered_items: Vec<(usize, &String)> = items.iter().enumerate()
            .filter(|(idx, label)| {
                let matches_label = label.to_lowercase().contains(&app.search_query.to_lowercase());
                let matches_info = if let Some(infos) = infos {
                    infos[*idx].to_lowercase().contains(&app.search_query.to_lowercase())
                } else {
                    false
                };
                matches_label || matches_info
            })
            .collect();

        let sidebar_list_items: Vec<ListItem> = filtered_items.iter().enumerate().map(|(idx, (i, item))| {
            let color = match app.sidebar_mode {
                SidebarMode::History => Color::Rgb(200, 200, 200),
                SidebarMode::Gists => Color::Yellow,
                SidebarMode::Commands => {
                    if commands[*i].contains("<prompt") {
                        Color::Green
                    } else if commands[*i].contains("sudo") || (infos.is_some() && infos.unwrap()[*i].to_lowercase().contains("sudo")) {
                        Color::Red
                    } else {
                        Color::White
                    }
                }
            };

            let item_bg = if is_sidebar_locked {
                Color::Black
            } else if app.sidebar_state.selected() == Some(idx) {
                Color::Rgb(60, 60, 60)
            } else {
                Color::Black
            };

            let offset = app.sidebar_state.offset();
            let is_row_hovered = if let Some((mx, my)) = app.mouse_pos {
                list_area.contains(Position::new(mx, my)) && 
                my == list_area.y + (idx as u16).saturating_sub(offset as u16) && 
                idx >= offset && idx < offset + list_area.height as usize
            } else {
                false
            };

            let is_button_hovered = is_row_hovered && if let Some((mx, _)) = app.mouse_pos {
                mx == list_area.x + 1
            } else { false };

            let style = if is_sidebar_locked {
                Style::default().fg(Color::DarkGray).bg(item_bg)
            } else if app.sidebar_state.selected() == Some(idx) || is_row_hovered {
                Style::default().fg(color).bg(item_bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color).bg(item_bg)
            };
            
            let indicator_style = if is_sidebar_locked {
                Style::default().fg(Color::Rgb(25, 25, 25)).bg(item_bg)
            } else if is_button_hovered {
                Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg)
            };

            let symbol = "▸";
            
            let (info, is_fallback) = if let Some(infos) = infos {
                let info_text = &infos[*i];
                if info_text.trim().is_empty() || info_text.len() < 3 {
                    (format!(" {}", commands[*i]), true)
                } else {
                    (format!(" {}", info_text), false)
                }
            } else {
                (String::new(), false)
            };

            let label_text = if app.sidebar_mode == SidebarMode::Gists && app.loading_gist && idx == 0 {
                let now = Instant::now().duration_since(app.start_time).as_millis();
                let spinner = ["|", "/", "-", "\\"][(now / 150 % 4) as usize];
                format!("{} LOADING ...", spinner)
            } else {
                item.to_string()
            };

            // Detect script items via __SCRIPT__ tag in info field
            let is_script = if let Some(infos) = infos {
                infos[*i].starts_with("__SCRIPT__")
            } else {
                false
            };

            // Strip __SCRIPT__ prefix from label if present (gists tab)
            let label_text = if label_text.starts_with("__SCRIPT__") {
                label_text[10..].to_string()
            } else {
                label_text
            };

            // Strip __SCRIPT__ from info display text
            let info = if info.trim_start().starts_with("__SCRIPT__") {
                let s = info.replacen("__SCRIPT__", "", 1);
                if s.starts_with("  ") {
                    s[1..].to_string()
                } else {
                    s
                }
            } else {
                info
            };

            let mut command_text = String::new();
            let mut hide_info = false;
            if label_text.len() < 20 && app.sidebar_mode != SidebarMode::History {
                command_text = format!(" {}", commands[*i]);
                if is_fallback {
                    hide_info = true;
                }
            }

            let info_style = if is_fallback {
                Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg)
            } else if is_row_hovered || app.sidebar_state.selected() == Some(idx) {
                Style::default().fg(Color::Rgb(90, 90, 90)).bg(item_bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(90, 90, 90)).bg(item_bg)
            };

            let command_style = Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg);
            let script_badge_style = Style::default().fg(Color::Rgb(80, 80, 80)).bg(item_bg);

            let line = Line::from(vec![
                Span::styled(" ", Style::default().bg(item_bg)),
                Span::styled(symbol, indicator_style),
                Span::styled(label_text, style),
                if is_script { Span::styled(" SCRIPT", script_badge_style) } else { Span::raw("") },
                if hide_info { Span::raw("") } else { Span::styled(info, info_style) },
                Span::styled(command_text, command_style),
            ]);
            ListItem::new(line).style(Style::default().bg(item_bg))
        }).collect();

        let sidebar_list = List::new(sidebar_list_items)
            .style(Style::default().bg(Color::Black))
            .highlight_style(if is_app_active { 
                Style::default()
            } else { 
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol("");
        f.render_stateful_widget(sidebar_list, list_area, &mut app.sidebar_state);

        let sidebar_scroll_pos = app.sidebar_state.selected().unwrap_or(0);
        let mut sidebar_scrollbar_state = ScrollbarState::new(filtered_items.len())
            .position(sidebar_scroll_pos);

        let is_sidebar_scrollbar_hovered = if let Some((mx, my)) = app.mouse_pos {
            sidebar_scrollbar_area.contains(Position::new(mx, my))
        } else { false };

        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(" "))
                .thumb_symbol("┃")
                .track_style(Style::default().bg(Color::Black).fg(Color::Rgb(20, 20, 20)))
                .thumb_style(if is_sidebar_scrollbar_hovered || app.is_dragging_sidebar_scrollbar {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::Rgb(50, 50, 50))
                }),
            sidebar_scrollbar_area,
            &mut sidebar_scrollbar_state,
        );
        f.render_widget(Block::default().style(Style::default().bg(Color::Black)), chunks[1]);
        
        let status_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(chunks[1]);
            
        let (rows, _) = screen.size();
        let scroll_offset = screen.scrollback();
        let history_len = screen.scrollback_len();

        let mut build_and_render_bar = |bar_items: &Vec<StatusBarItem>, bar_chunk: Rect| {
            let mut constraints = Vec::new();
            let mut visible_items = Vec::new();

            for item in bar_items {
                if app.is_item_visible(item) {
                    visible_items.push(item);
                    let width = if let Some(w) = item.width {
                        Constraint::Length(w)
                    } else if item.type_ == ItemType::Spacer || item.type_ == ItemType::GitInfo || item.type_ == ItemType::SelectedCommandInfo {
                        Constraint::Fill(1)
                    } else if let Some(label) = &item.label {
                        Constraint::Length(label.len() as u16)
                    } else {
                        Constraint::Min(0)
                    };
                    constraints.push(width);
                }
            }

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(bar_chunk);

            for (i, item) in visible_items.iter().enumerate() {
                let chunk = chunks[i];
                if chunk.width == 0 { continue; }

                match item.type_ {
                    ItemType::Button => {
                        if let Some(label) = &item.label {
                            let is_hovered = if let Some((mx, my)) = app.mouse_pos {
                                chunk.contains(Position::new(mx, my))
                            } else { false };

                            let bg_color_str = if is_hovered {
                                item.hover_color.as_deref().unwrap_or(item.color.as_deref().unwrap_or("white"))
                            } else {
                                item.color.as_deref().unwrap_or("white")
                            };
                            
                            let bg_color = parse_color(bg_color_str);
                            let fg_color = if bg_color_str == "green" || bg_color_str == "cyan" || bg_color_str.starts_with("light_") { Color::Black } else { Color::White };
                            
                            let span = Span::styled(label, Style::default().bg(bg_color).fg(fg_color).add_modifier(Modifier::BOLD));
                            f.render_widget(Paragraph::new(span).style(Style::default().bg(Color::Black)), chunk);
                        }
                    }
                    ItemType::SystemStats => {
                        let cpu_color = if app.cpu_usage > 80.0 { Color::Red } else { Color::Yellow };
                        let cpu_mem_str = format!("{:.0}% {}/{}GB", 
                            app.cpu_usage.round(), 
                            (app.mem_usage.0 as f64 / 1_073_741_824.0).round() as u64, 
                            (app.mem_usage.1 as f64 / 1_073_741_824.0).round() as u64
                        );
                        f.render_widget(
                            Paragraph::new(Span::styled(cpu_mem_str, Style::default().fg(cpu_color)))
                                .alignment(ratatui::layout::Alignment::Right)
                                .style(Style::default().bg(Color::Black)), 
                            chunk
                        );
                    }
                    ItemType::GitInfo => {
                        let git_str = app.git_info.as_deref().unwrap_or("");
                        f.render_widget(
                            Paragraph::new(Span::styled(format!("  {}", git_str), Style::default().fg(Color::Magenta)))
                                .style(Style::default().bg(Color::Black)), 
                            chunk
                        );
                    }
                    ItemType::TimeAndScroll => {
                        let (rows, _) = screen.size();
                        let scroll_offset = screen.scrollback();
                        let history_len = screen.scrollback_len();
                        let total_lines = rows as usize + history_len; 
                        let line_info = format!("{}/{}", scroll_offset, total_lines);
                        let stats_line = Line::from(vec![
                            Span::styled(format!("{} | {}", line_info, chrono::Local::now().format("%H:%M:%S")), Style::default().fg(Color::White)),
                        ]);
                        f.render_widget(
                            Paragraph::new(stats_line)
                                .alignment(ratatui::layout::Alignment::Right)
                                .style(Style::default().bg(Color::Black)), 
                            chunk
                        );
                    }
                    ItemType::SelectedCommandInfo => {
                        if let Some(idx) = app.sidebar_state.selected() {
                            if idx < app.sidebar_items.len() {
                                let label = &app.sidebar_items[idx];
                                let info = &app.sidebar_infos[idx];
                                let cmd = &app.sidebar_commands[idx];
                                let status_line = Line::from(vec![
                                    Span::styled(" # ", Style::default().fg(Color::DarkGray)),
                                    Span::styled(format!("{} {} ", label, info), Style::default().fg(Color::White)),
                                    Span::styled(cmd.to_string(), Style::default().fg(Color::DarkGray)),
                                ]);
                                f.render_widget(
                                    Paragraph::new(status_line).style(Style::default().bg(Color::Black)),
                                    chunk
                                );
                            }
                        }
                    }
                    ItemType::Spacer => {}
                }
            }
        };

        build_and_render_bar(&app.config.statusbar.upper, status_chunks[0]);
        build_and_render_bar(&app.config.statusbar.lower, status_chunks[1]);

        f.render_widget(Block::default().style(Style::default().bg(Color::Black)), scrollbar_area);
        let history_len = screen.scrollback_len();
        let scroll_pos = history_len.saturating_sub(scroll_offset);

        let mut scrollbar_state = ScrollbarState::new(history_len)
            .position(scroll_pos);

        let is_term_scrollbar_hovered = if let Some((mx, my)) = app.mouse_pos {
            scrollbar_area.contains(Position::new(mx, my))
        } else { false };

        let term_scrollbar_color = if app.is_dragging_sidebar {
            Color::Yellow
        } else if is_term_scrollbar_hovered || app.is_dragging_term_scrollbar { 
            Color::White 
        } else { 
            Color::Rgb(60, 60, 60) 
        };

        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(Style::default().fg(Color::Rgb(30, 30, 30)))
                .thumb_style(Style::default().fg(term_scrollbar_color))
                .begin_style(Style::default().fg(term_scrollbar_color))
                .end_style(Style::default().fg(term_scrollbar_color)),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }

    if app.show_menu {
        let area = centered_rect(40, 30, size);
        f.render_widget(Clear, area);
        let menu_block = Block::default()
            .title(" Commands ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .style(Style::default().bg(Color::Black));
        let menu_items = vec![
            ListItem::new("b: Run Btop").style(Style::default().bg(Color::Black)), 
            ListItem::new("s: Check Space").style(Style::default().bg(Color::Black)), 
            ListItem::new("q: Close").style(Style::default().bg(Color::Black))
        ];
        f.render_widget(List::new(menu_items).block(menu_block).style(Style::default().bg(Color::Black)), area);
    }

    if app.show_settings_modal {
        let area = centered_rect_fixed(50, 20, size);
        f.render_widget(Clear, area);
        
        let block = Block::default()
            .title(Span::styled(" Settings & GitHub Auth ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .style(Style::default().bg(Color::Black));
            
        f.render_widget(block.clone(), area);
        let inner_area = block.inner(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(2), // Device Flow
                Constraint::Length(1), // PAT Header
                Constraint::Length(3), // PAT Input
                Constraint::Length(1), // Editor Header
                Constraint::Length(3), // Editor Input
                Constraint::Min(0),    // Status
            ])
            .split(inner_area);

        // --- Device Flow Section ---
        if let (Some(uri), Some(code)) = (&app.github_verification_uri, &app.github_user_code) {
            let device_text = format!("1. Open {}   2. Enter: {}", uri, code);
            f.render_widget(Paragraph::new("GITHUB DEVICE FLOW:").style(Style::default().fg(Color::DarkGray)), chunks[0]);
            f.render_widget(Paragraph::new(Span::styled(device_text, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))), chunks[1]);
        } else {
            f.render_widget(Paragraph::new("GITHUB DEVICE FLOW:").style(Style::default().fg(Color::DarkGray)), chunks[0]);
            f.render_widget(Paragraph::new("Enter the code on GitHub to authenticate...").style(Style::default().fg(Color::DarkGray)), chunks[1]);
        }

        // --- PAT Section ---
        f.render_widget(Paragraph::new("GITHUB PERSONAL ACCESS TOKEN (PAT):").style(Style::default().fg(Color::DarkGray)), chunks[2]);
        
        let pat_style = if app.is_pat_focused {
            Style::default().fg(Color::Yellow).bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let pat_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if app.is_pat_focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) });

        let now = Instant::now().duration_since(app.start_time).as_millis();
        let pat_cursor = if app.is_pat_focused && (now / 500) % 2 == 0 { "_" } else { " " };
        let pat_display = format!(" {}{} ", app.pat_input, pat_cursor);
        f.render_widget(Paragraph::new(pat_display).style(pat_style).block(pat_block), chunks[3]);

        // --- Editor Section ---
        f.render_widget(Paragraph::new("EXTERNAL EDITOR:").style(Style::default().fg(Color::DarkGray)), chunks[4]);
        
        let editor_style = if app.is_editor_focused {
            Style::default().fg(Color::Yellow).bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let editor_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if app.is_editor_focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) });

        let editor_cursor = if app.is_editor_focused && (now / 500) % 2 == 0 { "_" } else { " " };
        let editor_display = format!(" {}{} ", app.editor_input, editor_cursor);
        f.render_widget(Paragraph::new(editor_display).style(editor_style).block(editor_block), chunks[5]);

        // --- Status/Error Section ---
        if let Some(err) = &app.login_error {
            f.render_widget(Paragraph::new(Span::styled(err, Style::default().fg(Color::Red))), chunks[6]);
        } else if app.auth_token.is_some() {
            f.render_widget(Paragraph::new(Span::styled("✓ Authenticated", Style::default().fg(Color::Green))), chunks[6]);
        } else {
            f.render_widget(Paragraph::new(Span::styled("Waiting for input...", Style::default().fg(Color::DarkGray))), chunks[6]);
        }
    }

    if app.show_upload_confirm {
        let area = centered_rect_fixed(50, 10, size);
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Span::styled(" Upload Modified Gist? ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block.clone(), area);
        let inner = block.inner(area);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Filename
                Constraint::Length(2), // Question
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Buttons
                Constraint::Min(0),
            ])
            .split(inner);

        let filename = app.pending_gist_file.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy()).unwrap_or_default();
        f.render_widget(Paragraph::new(format!("File '{}' was modified locally.", filename)).alignment(ratatui::layout::Alignment::Center), chunks[0]);
        f.render_widget(Paragraph::new("\nUpload changes to GitHub Gist?").alignment(ratatui::layout::Alignment::Center), chunks[1]);

        let button_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(chunks[3]);

        let is_yes_hovered = if let Some((mx, my)) = app.mouse_pos {
            button_chunks[0].contains(Position::new(mx, my))
        } else { false };
        let is_no_hovered = if let Some((mx, my)) = app.mouse_pos {
            button_chunks[1].contains(Position::new(mx, my))
        } else { false };

        let yes_style = if is_yes_hovered {
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green).bg(Color::Rgb(20, 40, 20))
        };
        let no_style = if is_no_hovered {
            Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red).bg(Color::Rgb(40, 20, 20))
        };

        f.render_widget(Paragraph::new(" [Y] Yes, Upload ").style(yes_style).alignment(ratatui::layout::Alignment::Center), button_chunks[0]);
        f.render_widget(Paragraph::new(" [N] No, Keep Local ").style(no_style).alignment(ratatui::layout::Alignment::Center), button_chunks[1]);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)].as_ref())
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)].as_ref())
        .split(popup_layout[1])[1]
}

fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(r.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);
        
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(r.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(popup_layout[1])[1]
}
