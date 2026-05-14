use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::process::Command;
use ratatui::widgets::ListState;
use vt100::Parser;
use sysinfo::System;
use portable_pty;
use crossterm::event::Event;
use crate::config::{Config, load_config, StatusBarItem, ConditionType};
use crate::utils::{log_debug, parse_script_content, parse_lines};

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SidebarMode { Commands, History, Gists }

pub enum Message {
    Event(Event),
    PtyData,
    Tick,
    DeviceCodeSuccess(String, String, String), // device_code, user_code, verification_uri
    AuthSuccess(String), // access_token
    AuthError(String),
    FetchGists,
    DeleteGist(usize),
    CreateNewGist,
    ConfirmNewGistName(String), // custom gist name
    GistsFetched(Vec<(String, String, String, Option<std::time::SystemTime>, Option<std::path::PathBuf>, String)>), // (label, info, command, mtime, path, remote_name)
    GistUploadStatus(String, bool), // (message, is_success)
}

pub struct App {
    pub cpu_usage: f32,
    pub mem_usage: (u64, u64),
    pub git_info: Option<String>,
    pub sidebar_state: ListState,
    pub is_dragging_scrollbar: bool,
    pub is_dragging_sidebar_scrollbar: bool,
    pub is_selecting: bool,
    pub selection_start: Option<(u16, u16)>,
    pub selection_end: Option<(u16, u16)>,
    pub sidebar_items: Vec<String>,
    pub sidebar_commands: Vec<String>,
    pub sidebar_infos: Vec<String>,
    pub sidebar_mtimes: Vec<Option<std::time::SystemTime>>,
    pub sidebar_paths: Vec<Option<std::path::PathBuf>>,
    pub sidebar_width: u16,
    pub is_dragging_sidebar: bool,
    pub is_dragging_term_scrollbar: bool,
    pub show_menu: bool,
    pub mouse_pos: Option<(u16, u16)>,
    pub parser: Arc<Mutex<Parser>>,
    pub pty_write: Box<dyn Write + Send>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub should_quit: bool,
    pub last_activity: Instant,
    pub start_time: Instant,
    pub search_query: String,
    pub is_search_focused: bool,
    pub sidebar_mode: SidebarMode,
    pub history_items: Vec<String>,
    pub history_commands: Vec<String>,
    pub gist_items: Vec<String>,
    pub gist_commands: Vec<String>,
    pub gist_infos: Vec<String>,
    pub gist_mtimes: Vec<Option<std::time::SystemTime>>,
    pub gist_paths: Vec<Option<std::path::PathBuf>>,
    pub gist_remote_names: Vec<String>,
    pub shell_pid: u32,
    pub is_pty_busy: bool,
    pub config: Config,
    
    // Settings / Auth State
    pub show_settings_modal: bool,
    pub github_user_code: Option<String>,
    pub github_verification_uri: Option<String>,
    pub github_device_code: Option<String>,
    pub auth_token: Option<String>,
    pub login_error: Option<String>,
    pub pat_input: String,
    pub is_pat_focused: bool,
    pub editor_input: String,
    pub is_editor_focused: bool,
    pub loading_gist: bool,
    pub last_click_time: Option<Instant>,
    pub last_clicked_index: Option<usize>,
    
    // Editing Tracking
    pub editing_file: Option<(std::path::PathBuf, std::time::SystemTime)>,
    pub last_pty_busy: bool,
    pub show_upload_confirm: bool,
    pub pending_gist_file: Option<std::path::PathBuf>,
    
    // New Gist Dialog
    pub show_new_gist_dialog: bool,
    pub new_gist_name_input: String,
    
    // Delete Gist Confirmation
    pub show_delete_confirm: bool,
    pub gist_index_to_delete: Option<usize>,
}

pub fn load_commands(exe_dir: &std::path::Path) -> (Vec<String>, Vec<String>, Vec<String>, Vec<Option<std::time::SystemTime>>, Vec<Option<std::path::PathBuf>>) {
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
                            let (id_opt, desc, code_preview) = parse_script_content(&content);
                            let name = id_opt.unwrap_or_else(|| {
                                filename.trim_start_matches("script").trim_start_matches('-').trim().to_string()
                            });
                            let dedup_key = format!("__script__{}", name);
                            let count = label_counts.entry(dedup_key).or_insert(0);
                            *count += 1;
                            let final_label = if *count > 1 { format!("{}{}", name, *count) } else { name.clone() };

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
                        }
 else {
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

impl App {
    pub fn is_item_visible(&self, item: &StatusBarItem) -> bool {
        match &item.condition {
            Some(ConditionType::HasGit) => self.git_info.is_some(),
            Some(ConditionType::HasSelection) => self.selection_start.is_some() && self.selection_end.is_some() && !self.is_selecting,
            None => true,
        }
    }

    pub fn process_prompt(&self, cmd: &mut String) {
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

    pub fn new(
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
            show_new_gist_dialog: false,
            new_gist_name_input: String::new(),
            show_delete_confirm: false,
            gist_index_to_delete: None,
        }
    }

    pub fn refresh_history(&mut self) {
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

    pub fn update_stats(&mut self, sys: &mut System) -> Option<std::path::PathBuf> {
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

        let mut gist_to_upload = None;

        // Detect transition from busy to not busy (editor closed)
        if self.last_pty_busy && !self.is_pty_busy {
            if let Some((path, old_mtime)) = self.editing_file.take() {
                if let Ok(metadata) = std::fs::metadata(&path) {
                    if let Ok(new_mtime) = metadata.modified() {
                        if new_mtime > old_mtime {
                            log_debug(&format!("File modified: {:?}", path));
                            
                            let current_path = path.clone();

                            // Check if it's a Gist file
                            if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
                                let gist_dir = proj_dirs.config_dir().join("gists");
                                if path.starts_with(&gist_dir) {
                                    // Update labels and info from content without renaming file
                                    if let Ok(content) = std::fs::read_to_string(&path) {
                                        let (id_opt, desc, preview) = parse_script_content(&content);
                                        
                                        // Update Gist tab items
                                        for idx in 0..self.gist_paths.len() {
                                            if self.gist_paths[idx].as_ref() == Some(&path) {
                                                if let Some(id) = id_opt.clone() {
                                                    self.gist_items[idx] = id;
                                                }
                                                
                                                let mut info = "__SCRIPT__".to_string();
                                                if !desc.is_empty() { info.push(' '); info.push_str(&desc); }
                                                if !preview.is_empty() { info.push(' '); info.push_str(&preview); }
                                                self.gist_infos[idx] = info;
                                            }
                                        }
                                    }

                                    gist_to_upload = Some(current_path.clone());
                                    
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
                return gist_to_upload;
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

        gist_to_upload
    }

    pub fn copy_selection(&mut self) {
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
