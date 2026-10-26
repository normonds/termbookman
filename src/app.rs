use crate::config::{
    commands_txt_path, gist_dir, load_config, ConditionType, Config, StatusBarItem,
};
use crate::entry::{filter_indices, load_entries, parse_file_entries, Entry, EntryKind};
use crate::utils::log_debug;
use crossterm::event::Event;
use ratatui::widgets::ListState;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use sysinfo::System;
use vt100::Parser;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SidebarMode {
    Commands,
    History,
    Gists,
}

pub enum Message {
    Event(Event),
    PtyData,
    Tick,
    DeviceCodeSuccess(crate::github::DeviceCode),
    AuthSuccess(String), // access_token
    AuthError(String),
    /// The device login flow ended (successfully or not).
    DeviceFlowFinished,
    GistFetchError(String),
    FetchGists,
    DeleteGist(PathBuf),
    CreateNewGist,
    ConfirmNewGistName(String), // custom gist name
    /// Fetched entries and the names of files whose local edits were kept.
    GistsFetched(Vec<Entry>, Vec<String>),
    GistUploadStatus(String, bool), // (message, is_success)
    /// An upload succeeded: (local path, uploaded content, gist id).
    GistUploaded(PathBuf, String, String),
    UpdateBinary,
}

/// Default commands written to commands.txt on first run.
pub const DEFAULT_COMMANDS: &str = include_str!("../commands.txt");

/// Loads commands.txt and the gist cache from the config directory.
pub fn load_all_entries() -> Vec<Entry> {
    let cmd_path = commands_txt_path().unwrap_or_else(|| PathBuf::from("commands.txt"));
    log_debug(&format!("Loading commands from: {:?}", cmd_path));
    load_entries(&cmd_path, &gist_dir(), DEFAULT_COMMANDS)
}

pub struct App {
    pub cpu_usage: f32,
    pub mem_usage: (u64, u64),
    pub git_info: Option<String>,
    git_counter: u32,
    pub sidebar_state: ListState,
    pub is_dragging_sidebar_scrollbar: bool,
    pub is_selecting: bool,
    pub selection_start: Option<(u16, u16)>,
    pub selection_end: Option<(u16, u16)>,
    pub sidebar_entries: Vec<Entry>,
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
    pub history_entries: Vec<Entry>,
    pub gist_entries: Vec<Entry>,
    pub shell_pid: u32,
    pub is_pty_busy: bool,
    pub config: Config,

    // Settings / Auth State
    pub show_settings_modal: bool,
    pub github_user_code: Option<String>,
    pub github_verification_uri: Option<String>,
    pub auth_token: Option<String>,
    pub login_error: Option<String>,
    /// True while a device-login request or polling thread is running.
    pub device_flow_running: bool,
    /// Set to stop the device-login polling thread.
    pub device_flow_cancel: Arc<AtomicBool>,
    pub pat_input: String,
    pub is_pat_focused: bool,
    pub update_url_input: String,
    pub is_update_url_focused: bool,
    pub editor_input: String,
    pub is_editor_focused: bool,
    pub loading_gist: bool,
    pub last_click_time: Option<Instant>,
    pub last_clicked_index: Option<usize>,
    pub update_disabled_reason: Option<String>,

    // Editing Tracking
    pub editing_file: Option<(PathBuf, std::time::SystemTime)>,
    pub last_pty_busy: bool,

    // New Gist Dialog
    pub show_new_gist_dialog: bool,
    pub new_gist_name_input: String,

    // Delete Gist Confirmation
    pub show_delete_confirm: bool,
    pub gist_to_delete: Option<PathBuf>,

    // commands.txt sync tracking
    pub commands_txt_path: Option<PathBuf>,
    pub commands_txt_mtime: Option<std::time::SystemTime>,
    pub commands_txt_changed: bool,

    // GitHub sync status notification
    pub github_status_msg: Option<String>,
    pub github_status_is_error: bool,
    pub github_status_until: Option<std::time::Instant>,
}

fn check_update_disabled() -> Option<String> {
    // List of privileged directories where auto-updates might cause permission issues
    let privileged_dirs = [
        "/bin",
        "/usr/bin",
        "/usr/local/bin",
        "/sbin",
        "/usr/sbin",
        "/usr/local/sbin",
        "/opt/bin",
    ];

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let in_privileged_dir = privileged_dirs.iter().any(|dir| exe_dir.starts_with(dir));

            if in_privileged_dir {
                #[cfg(unix)]
                let is_root = unsafe { libc::geteuid() == 0 };
                #[cfg(not(unix))]
                let is_root = false;

                if !is_root {
                    return Some(
                        "⚠ App in restricted directory. Run with 'sudo' to enable updates."
                            .to_string(),
                    );
                }
            }
        }
    }
    None
}

/// Current working directory of a process (Linux), used to show git info for the shell's cwd.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()
}

/// "GIT: branch (clean|modified)" for the repository containing `dir`, if any.
pub fn git_info_for(dir: &Path) -> Option<String> {
    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())?;
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| {
            if out.stdout.iter().all(u8::is_ascii_whitespace) {
                "clean"
            } else {
                "modified"
            }
        })
        .unwrap_or("unknown");
    Some(format!("GIT: {} ({})", branch, status))
}

impl App {
    pub fn set_github_status(&mut self, msg: impl Into<String>, is_error: bool, secs: u64) {
        self.github_status_msg = Some(msg.into());
        self.github_status_is_error = is_error;
        self.github_status_until =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(secs));
    }

    pub fn is_item_visible(&self, item: &StatusBarItem) -> bool {
        match &item.condition {
            Some(ConditionType::HasGit) => self.git_info.is_some(),
            Some(ConditionType::HasSelection) => {
                self.selection_start.is_some() && self.selection_end.is_some() && !self.is_selecting
            }
            None => true,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        match self.sidebar_mode {
            SidebarMode::Commands => &self.sidebar_entries,
            SidebarMode::History => &self.history_entries,
            SidebarMode::Gists => &self.gist_entries,
        }
    }

    /// Indices into `entries()` that match the search query, in display order.
    pub fn filtered_indices(&self) -> Vec<usize> {
        filter_indices(self.entries(), &self.search_query)
    }

    /// The entry under the sidebar selection (the selection indexes the filtered list).
    pub fn selected_entry(&self) -> Option<&Entry> {
        let visual = self.sidebar_state.selected()?;
        let idx = *self.filtered_indices().get(visual)?;
        self.entries().get(idx)
    }

    pub fn reload_sidebar(&mut self) {
        self.sidebar_entries = load_all_entries();
    }

    pub fn new(
        pty_write: Box<dyn Write + Send>,
        master: Box<dyn portable_pty::MasterPty + Send>,
        parser: Arc<Mutex<Parser>>,
        sidebar_entries: Vec<Entry>,
        shell_pid: u32,
    ) -> App {
        let mut state = ListState::default();
        state.select(Some(0));
        let config = load_config();
        let auth_token = config
            .auth
            .personal_access_token
            .clone()
            .filter(|t| !t.is_empty() && t != "YOUR_TOKEN_HERE");
        let commands_txt_path = commands_txt_path();
        let commands_txt_mtime = commands_txt_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        App {
            cpu_usage: 0.0,
            mem_usage: (0, 0),
            git_info: None,
            git_counter: 0,
            sidebar_state: state,
            sidebar_entries,
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
            is_dragging_sidebar_scrollbar: false,
            is_selecting: false,
            selection_start: None,
            selection_end: None,
            search_query: String::new(),
            is_search_focused: false,
            sidebar_mode: SidebarMode::Commands,
            history_entries: Vec::new(),
            gist_entries: Vec::new(),
            shell_pid,
            is_pty_busy: false,
            config: config.clone(),
            show_settings_modal: false,
            github_user_code: None,
            github_verification_uri: None,
            auth_token: auth_token.clone(),
            login_error: None,
            device_flow_running: false,
            device_flow_cancel: Arc::new(AtomicBool::new(false)),
            pat_input: auth_token.unwrap_or_default(),
            is_pat_focused: false,
            update_url_input: crate::updater::resolve_update_url(
                &config.auth.update_url,
                std::env::consts::ARCH == "aarch64",
            ),
            is_update_url_focused: false,
            editor_input: config.external_editor.clone(),
            is_editor_focused: false,
            loading_gist: false,
            last_click_time: None,
            last_clicked_index: None,
            editing_file: None,
            last_pty_busy: false,
            show_new_gist_dialog: false,
            new_gist_name_input: String::new(),
            show_delete_confirm: false,
            gist_to_delete: None,
            update_disabled_reason: check_update_disabled(),
            commands_txt_path,
            commands_txt_mtime,
            commands_txt_changed: false,
            github_status_msg: None,
            github_status_is_error: false,
            github_status_until: None,
        }
    }

    pub fn refresh_history(&mut self) {
        let home = std::env::var("HOME").unwrap_or_default();
        let history_path = Path::new(&home).join(".bash_history");
        let content = std::fs::read(history_path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        self.history_entries = dedup_history(&content, 500)
            .iter()
            .map(|c| Entry::history(c))
            .collect();
    }

    /// Re-parses an edited gist file and updates its rows in the Gists tab.
    fn refresh_gist_entries_for(&mut self, path: &Path) {
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        let Some(pos) = self
            .gist_entries
            .iter()
            .position(|e| e.path.as_deref() == Some(path))
        else {
            return;
        };
        let old = self.gist_entries[pos].clone();
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let filename = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let remote = old.remote_name.clone().unwrap_or(filename);
        // Count labels of all other rows so the new labels stay unique.
        let mut counts = std::collections::HashMap::new();
        for e in self.gist_entries.iter().filter(|e| e.path.as_deref() != Some(path)) {
            counts.insert(e.label.clone(), 1);
        }
        let fresh = parse_file_entries(path, &content, Some(&remote), mtime, &mut counts);
        self.gist_entries.retain(|e| e.path.as_deref() != Some(path));
        let at = pos.min(self.gist_entries.len());
        self.gist_entries.splice(at..at, fresh);
    }

    pub fn update_stats(&mut self, sys: &mut System) -> Option<PathBuf> {
        // Clear expired github status notifications
        if let Some(until) = self.github_status_until {
            if std::time::Instant::now() >= until {
                self.github_status_msg = None;
                self.github_status_until = None;
            }
        }

        sys.refresh_cpu();
        sys.refresh_memory();
        sys.refresh_processes();

        self.cpu_usage = sys.global_cpu_info().cpu_usage();
        self.mem_usage = (sys.used_memory(), sys.total_memory());

        // The shell is busy while it has child processes.
        let shell_pid = sysinfo::Pid::from_u32(self.shell_pid);
        self.is_pty_busy = sys
            .processes()
            .values()
            .any(|p| p.parent() == Some(shell_pid));

        let mut gist_to_upload = None;

        // Detect transition from busy to not busy (editor closed)
        if self.last_pty_busy && !self.is_pty_busy {
            if let Some((path, old_mtime)) = self.editing_file.take() {
                let new_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if new_mtime.is_some_and(|m| m > old_mtime) {
                    log_debug(&format!("File modified: {:?}", path));
                    if path.starts_with(gist_dir()) {
                        self.refresh_gist_entries_for(&path);
                        gist_to_upload = Some(path.clone());
                        self.reload_sidebar();
                    } else if Some(&path) == self.commands_txt_path.as_ref() {
                        log_debug("Local commands modified, reloading...");
                        self.reload_sidebar();
                    }
                }
            }
        }
        self.last_pty_busy = self.is_pty_busy;

        // Check if commands.txt was modified externally and signal for Gist sync
        if let Some(cmd_path) = &self.commands_txt_path {
            if let Ok(new_mtime) = std::fs::metadata(cmd_path).and_then(|m| m.modified()) {
                if Some(new_mtime) != self.commands_txt_mtime {
                    log_debug("commands.txt modified");
                    self.commands_txt_mtime = Some(new_mtime);
                    self.commands_txt_changed = true;
                }
            }
        }

        // Ticks are 1s apart; refresh git info every 4 ticks.
        self.git_counter += 1;
        if self.git_counter < 4 && self.git_info.is_some() {
            return gist_to_upload;
        }
        self.git_counter = 0;

        let dir = process_cwd(self.shell_pid)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        self.git_info = git_info_for(&dir);

        gist_to_upload
    }

    pub fn copy_selection(&mut self) {
        log_debug("Copy selection triggered");
        let (Some(start), Some(end)) = (self.selection_start, self.selection_end) else {
            log_debug("No selection to copy");
            return;
        };
        let text = {
            let parser = self.parser.lock().unwrap();
            selection_text(parser.screen(), start, end)
        };
        log_debug(&format!("Copying {} chars", text.chars().count()));
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(e) = clipboard.set_text(text) {
                    log_debug(&format!("Clipboard set_text error: {:?}", e));
                }
            }
            Err(e) => log_debug(&format!("Clipboard init error: {:?}", e)),
        }
    }
}

/// Newest-first, de-duplicated, non-empty history lines, at most `limit`.
pub fn dedup_history(content: &str, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    content
        .lines()
        .rev()
        .map(str::trim)
        // Lines like "#1700000000" are timestamps written with HISTTIMEFORMAT.
        .filter(|l| !l.is_empty() && !is_history_timestamp(l))
        .filter(|l| seen.insert(l.to_string()))
        .take(limit)
        .map(str::to_string)
        .collect()
}

fn is_history_timestamp(line: &str) -> bool {
    line.strip_prefix('#')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// Text of the screen between two (row, col) positions, in either order.
pub fn selection_text(screen: &vt100::Screen, start: (u16, u16), end: (u16, u16)) -> String {
    let (_, cols) = screen.size();
    let ((min_row, min_col), (max_row, max_col)) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };

    let mut lines = Vec::new();
    for r in min_row..=max_row {
        let start_c = if r == min_row { min_col } else { 0 };
        let end_c = if r == max_row {
            max_col
        } else {
            cols.saturating_sub(1)
        };
        let mut line = String::new();
        for c in start_c..=end_c {
            match screen.cell(r, c) {
                Some(cell) if cell.is_wide_continuation() => {}
                Some(cell) if cell.has_contents() => line.push_str(cell.contents()),
                _ => line.push(' '),
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

/// Default file name for a new script; `.sh` is added only if missing.
pub fn new_script_filename(custom_name: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let name = custom_name.trim().replace(' ', "_");
    if name.is_empty() {
        format!("script_{}.sh", now.format("%Y%m%d_%H%M%S"))
    } else {
        let name = crate::gists::sanitize_filename(name.strip_suffix(".sh").unwrap_or(&name));
        format!("script-{}.sh", name)
    }
}

/// True if `entry` is a gist script that can be deleted from the Gists tab.
pub fn is_deletable(entry: &Entry) -> bool {
    entry.kind == EntryKind::Script && entry.remote_name.is_some()
}

/// The directory new shells start in: where termbookman was launched.
pub fn initial_shell_dir(exe_dir: &Path) -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| exe_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_newest_first_deduped_and_limited() {
        let content = "ls\ncd /tmp\n#1700000000\nls\n  \ngit status\ncd /tmp\n";
        assert_eq!(dedup_history(content, 10), vec!["cd /tmp", "git status", "ls"]);
        assert_eq!(dedup_history(content, 2), vec!["cd /tmp", "git status"]);
    }

    #[test]
    fn new_script_names_do_not_double_extension() {
        let now = chrono::Local::now();
        assert_eq!(new_script_filename("backup.sh", now), "script-backup.sh");
        assert_eq!(new_script_filename("my tool", now), "script-my_tool.sh");
        assert_eq!(new_script_filename("../x", now), "script-_x.sh");
        assert!(new_script_filename("  ", now).starts_with("script_"));
    }

    #[test]
    fn selection_text_extracts_rows_in_either_direction() {
        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process(b"hello\r\nworld  \r\nabc");
        let forward = selection_text(parser.screen(), (0, 1), (1, 2));
        assert_eq!(forward, "ello\nwor");
        assert_eq!(selection_text(parser.screen(), (1, 2), (0, 1)), forward);
        assert_eq!(selection_text(parser.screen(), (2, 0), (2, 9)), "abc");
    }

    #[test]
    fn selection_text_handles_wide_chars() {
        let mut parser = vt100::Parser::new(1, 10, 0);
        parser.process("日本x".as_bytes());
        assert_eq!(selection_text(parser.screen(), (0, 0), (0, 9)), "日本x");
    }

    #[test]
    fn git_info_reports_branch_for_a_repo() {
        let dir = crate::utils::test_dir("git");
        let ok = Command::new("git")
            .args(["init", "-q", "-b", "trunk"])
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // git not available
        }
        // An unborn branch has no HEAD commit, so rev-parse fails; that's "no info".
        assert_eq!(git_info_for(&dir), None);
        let commit = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(commit.success());
        assert_eq!(git_info_for(&dir).as_deref(), Some("GIT: trunk (clean)"));
        std::fs::write(dir.join("f"), "x").unwrap();
        assert_eq!(git_info_for(&dir).as_deref(), Some("GIT: trunk (modified)"));
    }

    #[test]
    fn process_cwd_reads_own_cwd() {
        if cfg!(target_os = "linux") {
            assert_eq!(
                process_cwd(std::process::id()).map(|p| p.canonicalize().unwrap()),
                std::env::current_dir().ok().map(|p| p.canonicalize().unwrap())
            );
        }
    }
}
