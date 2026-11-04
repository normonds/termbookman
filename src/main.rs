mod app;
mod cli;
mod config;
mod entry;
mod gists;
mod github;
mod handlers;
mod input;
mod ui;
mod updater;
mod utils;

use crate::app::{load_all_entries, App, Message};
use crate::config::{gist_dir, save_config};
use crate::entry::{Entry, EntryKind};
use crate::utils::log_debug;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::{
    error::Error,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
use vt100::Parser;

const DEFAULT_SIDEBAR_WIDTH: u16 = 40;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = cli::run(&args, load_all_entries) {
        std::process::exit(code);
    }
    run_tui()
}

/// Puts the outer terminal back into its normal state. Safe to call more than once.
fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b[?1006l\x1b[?1000l");
    let _ = execute!(
        stdout,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = stdout.flush();
    let _ = disable_raw_mode();
}

fn run_tui() -> Result<(), Box<dyn Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or("Could not find executable directory")?
        .to_path_buf();

    let entries = load_all_entries();
    log_debug("--- Starting termbookman ---");

    // Restore the terminal before a panic message is printed, or it would be unreadable.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        log_debug(&format!("PANIC: {}", info));
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &exe_dir, entries);

    restore_terminal();
    // Drop input that arrived while shutting down so it doesn't leak into the parent shell.
    while event::poll(Duration::from_millis(10)).unwrap_or(false) {
        let _ = event::read();
    }
    log_debug("App exited.");
    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    exe_dir: &std::path::Path,
    entries: Vec<Entry>,
) -> Result<(), Box<dyn Error>> {
    let size = terminal.size()?;
    let term_area = ui::calculate_layout(size, DEFAULT_SIDEBAR_WIDTH).term_area;
    let (rows, cols) = (term_area.height.max(1), term_area.width.max(1));

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let (early_cfg, _) = config::load_config();
    let shell_prog = early_cfg
        .shell
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "bash".to_string());
    let mut cmd = CommandBuilder::new(&shell_prog);
    cmd.cwd(app::initial_shell_dir(exe_dir));
    cmd.env("TERM", "xterm-256color");
    let current_lang = std::env::var("LANG").unwrap_or_default();
    if !current_lang.to_lowercase().contains("utf-8")
        && !current_lang.to_lowercase().contains("utf8")
    {
        cmd.env("LANG", "C.UTF-8");
    }
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERMBOOKMAN", "1");
    if shell_prog.ends_with("bash") || shell_prog == "bash" {
        let existing = std::env::var("PROMPT_COMMAND").unwrap_or_default();
        let prompt_cmd = if existing.contains("history -a") {
            existing
        } else if existing.trim().is_empty() {
            "history -a".to_string()
        } else {
            format!("history -a; {existing}")
        };
        cmd.env("PROMPT_COMMAND", prompt_cmd);
    }
    if let Ok(exe) = std::env::current_exe() {
        cmd.env(
            "TERMBOOKMAN_BIN",
            crate::handlers::sanitize_exe_path(&exe)
                .to_string_lossy()
                .to_string(),
        );
    }
    let mut child = pair.slave.spawn_command(cmd)?;

    let mut reader = pair.master.try_clone_reader()?;
    let pty_write = pair.master.take_writer()?;
    let master = pair.master;

    let scrollback = early_cfg.scrollback.max(100);
    let parser = Arc::new(Mutex::new(Parser::new(rows, cols, scrollback)));
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
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        parser_clone
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .process(&buffer[..n]);
                    }));
                    if pty_tx.send(Message::PtyData).is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });

    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if event_tx.send(Message::Event(event)).is_err() {
                break;
            }
        }
    });

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(1000));
        if tick_tx.send(Message::Tick).is_err() {
            break;
        }
    });

    let mut app = App::new(
        pty_write,
        master,
        parser,
        entries,
        child.process_id().unwrap_or(0),
    );
    app.sidebar_width = DEFAULT_SIDEBAR_WIDTH;

    if let Some(ref err) = app.config_error {
        app.set_github_status(format!("⚠ {}", err), true, 15);
    }

    if app.auth_token.is_some() {
        let _ = tx.send(Message::FetchGists { switch_tab: false });
    }

    let mut sys = System::new_all();

    loop {
        if app.should_quit {
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }

        terminal.draw(|f| ui::render(f, &mut app))?;

        // Animations (cursor blink, spinner) need regular redraws; otherwise wait for input.
        let animating = app.is_search_focused
            || app.loading_gist
            || app.show_new_gist_dialog
            || app.prompt_state.is_some()
            || app.is_pat_focused
            || app.is_editor_focused
            || app.is_update_url_focused;
        let first = if animating {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(msg) => Some(msg),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            Some(rx.recv()?)
        };

        // Handle everything that is already queued before drawing again, so a burst of PTY
        // output causes one redraw instead of one per read.
        let size = terminal.size()?;
        let mut next = first;
        while let Some(msg) = next {
            handle_message(&mut app, msg, &tx, &mut sys, size);
            if app.should_quit {
                break;
            }
            next = rx.try_recv().ok();
        }
    }

    app.device_flow_cancel.store(true, Ordering::Relaxed);
    let _ = child.kill();
    Ok(())
}

pub fn try_save_config(app: &mut App) {
    if let Some(err) = app.config_error.clone() {
        app.set_github_status(
            "Refusing to save: config.toml has parse error (backup in config.toml.bak)",
            true,
            10,
        );
        log_debug(&format!("Refusing to save config: {}", err));
        return;
    }
    if let Err(e) = save_config(&app.config) {
        app.set_github_status(format!("✗ Failed to save config: {}", e), true, 8);
        log_debug(&format!("Failed to save config: {}", e));
    }
}

fn spawn_upload(
    tx: &mpsc::Sender<Message>,
    token: String,
    path: PathBuf,
    remote_name: String,
    gist_id: Option<String>,
    expected_hash: Option<String>,
    verb: &'static str,
) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Message::GistUploadStatus(
                    format!("✗ {} failed: {}", verb, e),
                    false,
                ));
                return;
            }
        };
        log_debug(&format!("{} gist file: {}", verb, remote_name));
        let _ = tx.send(Message::GistUploadStatus(
            format!("[{}: {}]", verb, remote_name),
            true,
        ));
        match github::upload_gist(
            &token,
            &content,
            &remote_name,
            gist_id.as_deref(),
            expected_hash.as_deref(),
        ) {
            Ok(id) => {
                log_debug(&format!("✓ {} succeeded: {}", verb, remote_name));
                let _ = tx.send(Message::GistUploaded(path, content, id));
                let _ = tx.send(Message::GistUploadStatus(
                    format!("✓ {} succeeded: {}", verb, remote_name),
                    true,
                ));
            }
            Err(e) => {
                log_debug(&format!("✗ {} failed: {}", verb, e));
                let _ = tx.send(Message::GistUploadStatus(
                    format!("✗ {} failed: {}", verb, e),
                    false,
                ));
            }
        }
    });
}

/// Applies a key press to a single-line text input. Returns true on Enter.
fn edit_text(input: &mut String, key: &KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('u') => {
                input.clear();
                return false;
            }
            KeyCode::Char('w') => {
                let trimmed =
                    input.trim_end_matches(|c: char| c.is_whitespace() || c == '/' || c == '-');
                if let Some(pos) =
                    trimmed.rfind(|c: char| c.is_whitespace() || c == '/' || c == '-')
                {
                    input.truncate(pos + 1);
                } else {
                    input.clear();
                }
                return false;
            }
            _ => return false,
        }
    }
    match key.code {
        KeyCode::Backspace | KeyCode::Delete => {
            input.pop();
        }
        KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::ALT) => {
            input.push(c);
        }
        KeyCode::Enter => return true,
        _ => {}
    }
    false
}

/// The text input that currently has keyboard focus, if any.
fn focused_input(app: &mut App) -> Option<&mut String> {
    if let Some(prompt) = &mut app.prompt_state {
        Some(&mut prompt.current_input)
    } else if app.show_new_gist_dialog {
        Some(&mut app.new_gist_name_input)
    } else if app.show_settings_modal && app.is_pat_focused {
        Some(&mut app.pat_input)
    } else if app.show_settings_modal && app.is_update_url_focused {
        Some(&mut app.update_url_input)
    } else if app.show_settings_modal && app.is_editor_focused {
        Some(&mut app.editor_input)
    } else if app.is_search_focused {
        Some(&mut app.search_query)
    } else {
        None
    }
}

fn handle_message(
    app: &mut App,
    msg: Message,
    tx: &mpsc::Sender<Message>,
    sys: &mut System,
    size: Rect,
) {
    match msg {
        Message::PtyData => {
            // Just wake up to redraw
        }
        Message::GitInfo(info) => {
            app.git_info = info;
            app.git_query_running = false;
        }
        Message::Tick => {
            if let Some(path) = app.update_stats(sys, tx) {
                if let Some(token) = app.auth_token.clone() {
                    let dir = gist_dir();
                    let manifest = gists::Manifest::load(&dir);
                    let gist_id = manifest.gist_id(&dir, &path);
                    let expected_hash = manifest.get(&dir, &path).map(|e| e.hash.clone());
                    let remote_name = app
                        .find_entry_by_path(&path)
                        .and_then(|e| e.remote_name.clone())
                        .unwrap_or_else(|| {
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        });
                    spawn_upload(
                        tx,
                        token,
                        path,
                        remote_name,
                        gist_id,
                        expected_hash,
                        "Auto-upload",
                    );
                }
            }

            if app.commands_txt_changed {
                app.commands_txt_changed = false;
                if app.config.auth.sync_commands_to_gist {
                    if let (Some(token), Some(cmd_path)) =
                        (app.auth_token.clone(), app.commands_txt_path.clone())
                    {
                        let dir = gist_dir();
                        let manifest = gists::Manifest::load(&dir);
                        let gist_id = manifest.gist_id(&dir, &cmd_path);
                        let expected_hash = manifest.get(&dir, &cmd_path).map(|e| e.hash.clone());
                        spawn_upload(
                            tx,
                            token,
                            cmd_path,
                            "commands.txt".to_string(),
                            gist_id,
                            expected_hash,
                            "commands.txt sync",
                        );
                    }
                }
            }
        }
        Message::GistUploaded(path, content, id) => {
            let dir = gist_dir();
            gists::with_manifest(&dir, |m| {
                m.record_synced(&dir, &path, content.as_bytes(), Some(id))
            });
        }
        Message::DeviceCodeSuccess(code) => {
            app.github_user_code = Some(code.user_code.clone());
            app.github_verification_uri = Some(code.verification_uri.clone());
            app.login_error = None;

            let tx = tx.clone();
            let client_id = app.config.auth.github_client_id.clone().unwrap_or_default();
            let cancel = Arc::clone(&app.device_flow_cancel);
            std::thread::spawn(move || {
                let result =
                    github::poll_device_token(&client_id, &code, || cancel.load(Ordering::Relaxed));
                let _ = tx.send(match result {
                    Ok(token) => Message::AuthSuccess(token),
                    Err(e) => Message::AuthError(e),
                });
                let _ = tx.send(Message::DeviceFlowFinished);
            });
        }
        Message::DeviceFlowFinished => {
            app.device_flow_running = false;
        }
        Message::AuthSuccess(token) => {
            app.auth_token = Some(token.clone());
            app.pat_input = token.clone();
            app.config.auth.personal_access_token = Some(token);
            try_save_config(app);
            app.show_settings_modal = false;
            app.login_error = None;
            app.github_user_code = None;
            let _ = tx.send(Message::FetchGists { switch_tab: true });
        }
        Message::AuthError(err) => {
            log_debug(&format!("✗ GitHub login error: {}", err));
            app.set_github_status(format!("✗ GitHub login failed: {}", err), true, 8);
            app.login_error = Some(err);
        }
        Message::GistFetchError(err) => {
            log_debug(&format!("✗ Gist fetch error: {}", err));
            app.loading_gist = false;
            app.set_github_status(format!("✗ Gist fetch failed: {}", err), true, 8);
        }
        Message::FetchGists { switch_tab } => {
            let Some(token) = app.auth_token.clone() else {
                app.login_error = Some("Not logged in to GitHub.".to_string());
                app.show_settings_modal = true;
                return;
            };
            if app.loading_gist {
                return;
            }
            log_debug("Fetching gists from GitHub...");
            app.loading_gist = true;
            app.set_github_status("⟳ Fetching gists...", false, 30);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(match gists::fetch_all(&token, &gist_dir()) {
                    Ok((entries, kept)) => Message::GistsFetched(entries, kept, switch_tab),
                    Err(e) => Message::GistFetchError(e),
                });
            });
        }
        Message::DeleteGist(path) => {
            let dir = gist_dir();
            let entry = app.find_entry_by_path(&path).cloned();
            let remote_name = entry.and_then(|e| e.remote_name).unwrap_or_else(|| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            let gist_id = gists::Manifest::load(&dir).gist_id(&dir, &path);

            match (app.auth_token.clone(), gist_id) {
                (Some(token), Some(gist_id)) => {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        log_debug(&format!("Deleting remote gist file: {}", remote_name));
                        let _ = tx.send(Message::GistUploadStatus(
                            format!("[Deleting: {}]", remote_name),
                            true,
                        ));
                        match github::delete_gist_file(&token, Some(&gist_id), &remote_name) {
                            Ok(github::DeletePlan::RemoveFile) => {
                                let _ = tx.send(Message::GistUploadStatus(
                                    format!("✓ Removed {} from its gist", remote_name),
                                    true,
                                ));
                                let _ = tx.send(Message::GistDeletedLocally(path));
                            }
                            Ok(_) => {
                                let _ = tx.send(Message::GistUploadStatus(
                                    format!("✓ Gist deleted: {}", remote_name),
                                    true,
                                ));
                                let _ = tx.send(Message::GistDeletedLocally(path));
                            }
                            Err(e) => {
                                let _ = tx.send(Message::GistUploadStatus(
                                    format!("✗ Remote delete failed: {}; local file kept", e),
                                    false,
                                ));
                            }
                        }
                    });
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                    gists::with_manifest(&dir, |m| m.remove(&dir, &path));
                    app.gist_entries.retain(|e| e.path.as_ref() != Some(&path));
                    app.sidebar_entries
                        .retain(|e| e.path.as_ref() != Some(&path));
                    app.recompute_filtered_indices();
                    let len = app.filtered_indices().len();
                    if app.sidebar_state.selected().is_some_and(|s| s >= len) {
                        app.sidebar_state.select(Some(len.saturating_sub(1)));
                    }
                    app.set_github_status(
                        format!("✓ Deleted local file {}", remote_name),
                        false,
                        4,
                    );
                }
            }
        }
        Message::GistDeletedLocally(path) => {
            let dir = gist_dir();
            let _ = std::fs::remove_file(&path);
            gists::with_manifest(&dir, |m| m.remove(&dir, &path));
            app.gist_entries.retain(|e| e.path.as_ref() != Some(&path));
            app.sidebar_entries
                .retain(|e| e.path.as_ref() != Some(&path));
            app.recompute_filtered_indices();
            let len = app.filtered_indices().len();
            if app.sidebar_state.selected().is_some_and(|s| s >= len) {
                app.sidebar_state.select(Some(len.saturating_sub(1)));
            }
        }
        Message::CreateNewGist => {
            app.show_new_gist_dialog = true;
            app.new_gist_name_input.clear();
        }
        Message::ConfirmNewGistName(custom_name) => {
            app.show_new_gist_dialog = false;
            let dir = gist_dir();
            let mut filename = app::new_script_filename(&custom_name, chrono::Local::now());
            let base = filename.trim_end_matches(".sh").to_string();
            let mut n = 2;
            // Never overwrite an existing script.
            while dir.join(&filename).exists() {
                filename = format!("{}-{}.sh", base, n);
                n += 1;
            }
            let path = dir.join(&filename);
            let content = "#!/bin/bash\n# IDENTIFIER description tags\n\necho 'Hello World'\n";
            log_debug(&format!("Creating script gist locally: {:?}", path));
            if let Err(e) = utils::write_file(&path, content) {
                app.set_github_status(format!("✗ Could not create script: {}", e), true, 8);
                return;
            }
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            if let Some(mtime) = mtime {
                app.editing_file = Some((path.clone(), mtime));
            }
            handlers::open_in_editor(app, &path);
            app.gist_entries.insert(
                0,
                Entry {
                    label: "NEW SCRIPT".to_string(),
                    key: filename.clone(),
                    command: path.to_string_lossy().into_owned(),
                    info: String::new(),
                    kind: EntryKind::Script,
                    path: Some(path),
                    mtime,
                    remote_name: Some(filename),
                },
            );
        }
        Message::UpdateBinary => {
            if app.is_updating {
                return;
            }
            app.is_updating = true;
            let update_url = app.update_url_input.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let progress = |msg: String| {
                    let _ = tx.send(Message::GistUploadStatus(msg, true));
                };
                let res = updater::run_update(&update_url, progress);
                let _ = tx.send(Message::UpdateFinished(res));
            });
        }
        Message::UpdateFinished(res) => {
            app.is_updating = false;
            let msg = match res {
                Ok(()) => Message::GistUploadStatus(
                    "✓ Update check finished (see status above)".to_string(),
                    true,
                ),
                Err(e) => Message::GistUploadStatus(format!("✗ Update failed: {}", e), false),
            };
            let _ = tx.send(msg);
        }
        Message::GistsFetched(entries, kept, switch_tab) => {
            log_debug(&format!(
                "Gists fetched successfully: {} items",
                entries.len()
            ));
            app.loading_gist = false;
            if kept.is_empty() {
                app.set_github_status("✓ Gists loaded", false, 3);
            } else {
                app.set_github_status(
                    format!(
                        "✓ Gists loaded; kept local edits (not uploaded): {}",
                        kept.join(", ")
                    ),
                    false,
                    10,
                );
            }
            app.gist_entries = entries;
            if switch_tab {
                app.sidebar_mode = app::SidebarMode::Gists;
                app.sidebar_state.select(Some(0));
            }
            app.reload_sidebar();
        }
        Message::GistUploadStatus(msg, is_success) => {
            let secs = if is_success { 4 } else { 8 };
            app.set_github_status(&msg, !is_success, secs);
        }
        Message::UploadUnsynced => {
            if let Some(token) = app.auth_token.clone() {
                let dir = gist_dir();
                let manifest = gists::Manifest::load(&dir);
                let files = app.unsynced_files.clone();
                for path in files {
                    let gist_id = manifest.gist_id(&dir, &path);
                    let expected_hash = manifest.get(&dir, &path).map(|e| e.hash.clone());
                    let remote_name = app
                        .find_entry_by_path(&path)
                        .and_then(|e| e.remote_name.clone())
                        .unwrap_or_else(|| {
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        });
                    spawn_upload(
                        tx,
                        token.clone(),
                        path,
                        remote_name,
                        gist_id,
                        expected_hash,
                        "Sync unsynced edits",
                    );
                }
            } else {
                app.set_github_status("✗ Cannot upload: no GitHub token configured", true, 6);
            }
        }
        Message::Event(event) => handle_event(app, event, tx, size),
    }
}

fn handle_event(app: &mut App, event: Event, tx: &mpsc::Sender<Message>, size: Rect) {
    match event {
        Event::Key(key) => {
            if key.kind == event::KeyEventKind::Release {
                return;
            }
            app.last_activity = Instant::now();

            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            if ctrl
                && (key.code == KeyCode::Char('C')
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::SHIFT)))
            {
                app.copy_selection();
                return;
            }

            if key.modifiers.contains(KeyModifiers::SHIFT)
                && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
            {
                let mut p = app.lock_parser();
                let current = p.screen().scrollback();
                let target = if key.code == KeyCode::PageUp {
                    current + 20
                } else {
                    current.saturating_sub(20)
                };
                p.screen_mut().set_scrollback(target);
                return;
            }

            if app.show_delete_confirm {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(path) = app.gist_to_delete.take() {
                            let _ = tx.send(Message::DeleteGist(path));
                        }
                        app.show_delete_confirm = false;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        app.show_delete_confirm = false;
                        app.gist_to_delete = None;
                    }
                    _ => {}
                }
                return;
            }

            if app.show_menu {
                handlers::handle_menu_key(app, &key);
                return;
            }

            if let Some(prompt) = &mut app.prompt_state {
                if key.code == KeyCode::Esc {
                    app.prompt_state = None;
                } else if edit_text(&mut prompt.current_input, &key) {
                    let answer = prompt.current_input.clone();
                    prompt.answers.push(answer);
                    prompt.current_idx += 1;
                    if prompt.current_idx < prompt.prompts.len() {
                        prompt.current_input = prompt.prompts[prompt.current_idx].1.clone();
                    } else {
                        let template = prompt.template.clone();
                        let mut answers_iter = prompt.answers.clone().into_iter();
                        let final_cmd =
                            crate::utils::substitute_prompts(&template, |_label, _default| {
                                answers_iter.next().unwrap_or_default()
                            });
                        app.prompt_state = None;
                        let line = if final_cmd.ends_with('\r') || final_cmd.ends_with('\n') {
                            final_cmd
                        } else {
                            format!("{}\r", final_cmd)
                        };
                        let _ = app.pty_write.write_all(line.as_bytes());
                        let _ = app.pty_write.flush();
                    }
                }
                return;
            }

            if app.show_new_gist_dialog {
                if key.code == KeyCode::Esc {
                    app.show_new_gist_dialog = false;
                    app.new_gist_name_input.clear();
                } else if edit_text(&mut app.new_gist_name_input, &key) {
                    let name = std::mem::take(&mut app.new_gist_name_input);
                    let _ = tx.send(Message::ConfirmNewGistName(name));
                }
                return;
            }

            if app.show_settings_modal {
                if key.code == KeyCode::Esc {
                    if app.is_pat_focused || app.is_update_url_focused || app.is_editor_focused {
                        app.is_pat_focused = false;
                        app.is_update_url_focused = false;
                        app.is_editor_focused = false;
                    } else {
                        app.show_settings_modal = false;
                    }
                } else if app.is_pat_focused {
                    if edit_text(&mut app.pat_input, &key) {
                        let trimmed = app.pat_input.trim();
                        if trimmed.is_empty() {
                            app.auth_token = None;
                            app.config.auth.personal_access_token = None;
                            try_save_config(app);
                            app.is_pat_focused = false;
                            app.set_github_status("✓ Token cleared", false, 3);
                        } else {
                            let token = trimmed.to_string();
                            app.auth_token = Some(token.clone());
                            app.config.auth.personal_access_token = Some(token);
                            try_save_config(app);
                            app.is_pat_focused = false;
                            app.login_error = None;
                            // A manual token makes a pending device login pointless.
                            app.device_flow_cancel.store(true, Ordering::Relaxed);
                            let _ = tx.send(Message::FetchGists { switch_tab: true });
                        }
                    }
                } else if app.is_update_url_focused {
                    if edit_text(&mut app.update_url_input, &key)
                        && !app.update_url_input.trim().is_empty()
                    {
                        app.config.auth.update_url = app.update_url_input.trim().to_string();
                        try_save_config(app);
                        app.is_update_url_focused = false;
                    }
                } else if app.is_editor_focused
                    && edit_text(&mut app.editor_input, &key)
                    && !app.editor_input.trim().is_empty()
                {
                    app.config.external_editor = app.editor_input.trim().to_string();
                    try_save_config(app);
                    app.is_editor_focused = false;
                }
                return;
            }

            if app.is_search_focused {
                match key.code {
                    KeyCode::Esc => app.is_search_focused = false,
                    KeyCode::Enter => {
                        app.is_search_focused = false;
                        if let Some(entry) = app.selected_entry().cloned() {
                            let line = handlers::run_line(&entry);
                            let _ = app.pty_write.write_all(line.as_bytes());
                            let _ = app.pty_write.flush();
                        }
                    }
                    _ => {
                        edit_text(&mut app.search_query, &key);
                        app.recompute_filtered_indices();
                        app.sidebar_state.select(Some(0));
                    }
                }
                return;
            }

            if key.code == KeyCode::F(2) {
                app.is_sidebar_focused = !app.is_sidebar_focused;
                return;
            }

            if app.is_sidebar_focused {
                match key.code {
                    KeyCode::Esc => app.is_sidebar_focused = false,
                    KeyCode::Up => {
                        let sel = app.sidebar_state.selected().unwrap_or(0);
                        app.sidebar_state.select(Some(sel.saturating_sub(1)));
                    }
                    KeyCode::Down => {
                        let len = app.filtered_indices().len();
                        let sel = app.sidebar_state.selected().unwrap_or(0);
                        if len > 0 {
                            app.sidebar_state.select(Some((sel + 1).min(len - 1)));
                        }
                    }
                    KeyCode::PageUp => {
                        let sel = app.sidebar_state.selected().unwrap_or(0);
                        app.sidebar_state.select(Some(sel.saturating_sub(10)));
                    }
                    KeyCode::PageDown => {
                        let len = app.filtered_indices().len();
                        let sel = app.sidebar_state.selected().unwrap_or(0);
                        if len > 0 {
                            app.sidebar_state.select(Some((sel + 10).min(len - 1)));
                        }
                    }
                    KeyCode::Home => app.sidebar_state.select(Some(0)),
                    KeyCode::End => {
                        let len = app.filtered_indices().len();
                        if len > 0 {
                            app.sidebar_state.select(Some(len - 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(entry) = app.selected_entry().cloned() {
                            if entry.command.contains("<prompt:") {
                                app.open_prompt_dialog(&entry.command);
                            } else {
                                let line = handlers::run_line(&entry);
                                let _ = app.pty_write.write_all(line.as_bytes());
                                let _ = app.pty_write.flush();
                            }
                        }
                    }
                    KeyCode::Tab => {
                        app.sidebar_mode = match app.sidebar_mode {
                            app::SidebarMode::Commands => app::SidebarMode::History,
                            app::SidebarMode::History => app::SidebarMode::Gists,
                            app::SidebarMode::Gists => app::SidebarMode::Commands,
                        };
                        if app.sidebar_mode == app::SidebarMode::History {
                            app.refresh_history();
                        }
                        app.sidebar_state.select(Some(0));
                        app.recompute_filtered_indices();
                    }
                    KeyCode::Char('/') => {
                        app.is_search_focused = true;
                        app.is_sidebar_focused = false;
                    }
                    KeyCode::Delete => {
                        if let Some(entry) = app.selected_entry().cloned() {
                            if app::is_deletable(&entry) {
                                if let Some(path) = entry.path {
                                    app.gist_to_delete = Some(path);
                                    app.show_delete_confirm = true;
                                }
                            }
                        }
                    }
                    KeyCode::Char('s') => {
                        if app.sidebar_mode == app::SidebarMode::History {
                            if let Some(entry) = app.selected_entry().cloned() {
                                if let Some(cmd_path) = &app.commands_txt_path {
                                    let line = format!("\n# {}\n{}\n", entry.label, entry.command);
                                    if let Ok(mut file) = std::fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(cmd_path)
                                    {
                                        use std::io::Write;
                                        let _ = file.write_all(line.as_bytes());
                                        app.set_github_status(
                                            format!("✓ Saved '{}' to commands.txt", entry.label),
                                            false,
                                            4,
                                        );
                                        app.reload_sidebar();
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('e') => {
                        if let Some(entry) = app.selected_entry().cloned() {
                            if let Some(path) = entry
                                .path
                                .as_ref()
                                .filter(|_| entry.kind != crate::entry::EntryKind::History)
                            {
                                if let Ok(mtime) =
                                    std::fs::metadata(path).and_then(|m| m.modified())
                                {
                                    app.editing_file = Some((path.clone(), mtime));
                                }
                                handlers::open_in_editor(app, path);
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            handlers::forward_key(app, &key);
        }
        Event::Paste(text) => {
            if let Some(input) = focused_input(app) {
                input.extend(text.chars().filter(|c| !c.is_control()));
                if app.is_search_focused {
                    app.recompute_filtered_indices();
                }
                return;
            }
            let bracketed = {
                let mut p = app.lock_parser();
                p.screen_mut().set_scrollback(0);
                p.screen().bracketed_paste()
            };
            let bytes = input::paste_to_bytes(&text, bracketed);
            let _ = app.pty_write.write_all(&bytes);
            let _ = app.pty_write.flush();
        }
        Event::Mouse(mouse) => {
            app.mouse_pos = Some((mouse.column, mouse.row));
            handlers::handle_click(app, mouse, size, tx);
        }
        // The PTY is resized to the new layout on the next draw.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_text(bytes: &[u8]) -> String {
        let mut parser = Parser::new(2, 20, 0);
        parser.process(bytes);
        parser.screen().contents()
    }

    #[test]
    fn vt100_patch_handles_strikethrough_and_blink() {
        let mut parser = Parser::new(2, 20, 0);
        parser.process(b"\x1b[9mstrike\x1b[29m \x1b[5mblink\x1b[25m");
        let screen = parser.screen();
        assert!(screen.cell(0, 0).unwrap().strikethrough());
        assert!(!screen.cell(0, 7).unwrap().strikethrough());
        assert!(screen.cell(0, 7).unwrap().blink());
    }

    #[test]
    fn vt100_patch_maps_line_drawing_charset() {
        assert_eq!(screen_text(b"\x1b(0lqk\x1b(Bq"), "┌─┐q");
        // SO/SI switch to and from G1.
        assert_eq!(screen_text(b"\x1b)0x\x0ex\x0fx"), "x│x");
    }

    #[test]
    fn vt100_patch_full_reset_restores_ascii() {
        assert_eq!(screen_text(b"\x1b(0\x1bcq"), "q");
    }

    #[test]
    fn vt100_patch_keeps_scrollback_under_apt_progress_bar() {
        // apt reserves the last row with a top-anchored scroll region and redraws its bar
        // there; output scrolling inside the region must still reach scrollback.
        let mut parser = Parser::new(3, 20, 10);
        parser.process(b"\x1b[1;2r");
        for i in 0..4 {
            parser.process(format!("line{i}\r\n").as_bytes());
            parser.process(b"\x1b7\x1b[3;1Hbar\x1b8");
        }
        let screen = parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        assert_eq!(screen.scrollback(), 3);
        assert_eq!(screen.contents(), "line0\nline1\nline2");
        screen.set_scrollback(0);
        assert_eq!(screen.contents(), "line3\n\nbar");

        // A region below the top of the screen must not feed scrollback.
        let mut parser = Parser::new(3, 20, 10);
        parser.process(b"top\x1b[2;3r\x1b[2;1H");
        parser.process(b"a\r\nb\r\nc\r\n");
        parser.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(parser.screen().scrollback(), 0);
    }

    #[test]
    fn edit_text_ignores_control_chords() {
        let mut s = String::from("ab");
        assert!(!edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
        assert_eq!(s, "ab");
        edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT),
        );
        edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('ā'), KeyModifiers::NONE),
        );
        assert_eq!(s, "abā");
        assert!(edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
    }

    #[test]
    fn edit_text_ctrl_u_clears_buffer() {
        let mut s = String::from("hello world");
        assert!(!edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
        ));
        assert_eq!(s, "");
    }

    #[test]
    fn edit_text_ctrl_w_deletes_last_word() {
        let mut s = String::from("git checkout -b feature/test");
        assert!(!edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
        ));
        assert_eq!(s, "git checkout -b feature/");

        assert!(!edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
        ));
        assert_eq!(s, "git checkout -b ");

        assert!(!edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
        ));
        assert_eq!(s, "git checkout -");

        let mut single_word = String::from("singleword");
        edit_text(
            &mut single_word,
            &KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        );
        assert_eq!(single_word, "");
    }

    #[test]
    fn edit_text_backspace_and_delete_remove_character() {
        let mut s = String::from("cat");
        edit_text(&mut s, &KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(s, "ca");
        edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(s, "c");
    }

    #[test]
    fn edit_text_ignores_alt_modified_chars() {
        let mut s = String::from("initial");
        assert!(!edit_text(
            &mut s,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)
        ));
        assert_eq!(s, "initial");
    }

    #[test]
    fn test_backend_renders_sidebar_and_statusbar() {
        use crate::entry::{Entry, EntryKind};
        use ratatui::backend::TestBackend;
        use ratatui::layout::Position;
        use ratatui::Terminal;

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        let entries = vec![Entry {
            label: "ping-test".into(),
            key: "ping-test".into(),
            command: "ping -c 1 1.1.1.1".into(),
            info: "".into(),
            kind: EntryKind::Command,
            path: None,
            mtime: None,
            remote_name: None,
        }];

        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut app = App::new(
            Box::new(std::io::sink()),
            pair.master,
            Arc::new(Mutex::new(Parser::new(24, 80, 0))),
            entries,
            std::process::id(),
        );

        terminal.draw(|f| ui::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();

        let mut found_ping = false;
        let mut found_commands_tab = false;
        let mut found_status = false;

        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer.get(x, y).symbol());
            }
            if line.contains("ping-test") {
                found_ping = true;
            }
            if line.contains("COMMANDS") {
                found_commands_tab = true;
            }
            if line.contains("EXIT") || line.contains("LOG") || line.contains("CPU") {
                found_status = true;
            }
        }

        assert!(found_commands_tab, "Sidebar must render Commands tab");
        assert!(found_ping, "Sidebar must render ping-test entry");
        assert!(found_status, "Status bar must render status buttons/stats");

        let layout = ui::calculate_layout(Rect::new(0, 0, 100, 30), app.sidebar_width);
        let s_rects = ui::sidebar::sidebar_layout(layout.sidebar_area, false, false);
        assert!(s_rects
            .list
            .contains(Position::new(layout.sidebar_area.x + 2, s_rects.list.y + 1)));
    }

    #[test]
    fn handle_message_state_transitions() {
        use crate::app::SidebarMode;
        use crate::entry::{Entry, EntryKind};

        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut app = App::new(
            Box::new(std::io::sink()),
            pair.master,
            Arc::new(Mutex::new(Parser::new(24, 80, 0))),
            Vec::new(),
            std::process::id(),
        );

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut sys = sysinfo::System::new();
        let size = Rect::new(0, 0, 100, 30);

        // 1. GitInfo
        app.git_query_running = true;
        handle_message(
            &mut app,
            Message::GitInfo(Some("GIT: main (clean)".to_string())),
            &tx,
            &mut sys,
            size,
        );
        assert_eq!(app.git_info.as_deref(), Some("GIT: main (clean)"));
        assert!(!app.git_query_running);

        // 2. AuthError
        handle_message(
            &mut app,
            Message::AuthError("bad credentials".to_string()),
            &tx,
            &mut sys,
            size,
        );
        assert_eq!(app.login_error.as_deref(), Some("bad credentials"));

        // 3. GistsFetched with switch_tab = false (background fetch)
        app.sidebar_mode = SidebarMode::Commands;
        let test_gist = Entry {
            label: "test_gist".to_string(),
            key: "test_gist".to_string(),
            command: "echo 1".to_string(),
            info: "".to_string(),
            kind: EntryKind::Script,
            path: None,
            mtime: None,
            remote_name: None,
        };
        handle_message(
            &mut app,
            Message::GistsFetched(vec![test_gist.clone()], vec![], false),
            &tx,
            &mut sys,
            size,
        );
        assert_eq!(app.sidebar_mode, SidebarMode::Commands);
        assert_eq!(app.gist_entries.len(), 1);

        // 4. GistsFetched with switch_tab = true (user-initiated fetch)
        handle_message(
            &mut app,
            Message::GistsFetched(vec![test_gist], vec![], true),
            &tx,
            &mut sys,
            size,
        );
        assert_eq!(app.sidebar_mode, SidebarMode::Gists);

        // 5. AuthSuccess
        app.show_settings_modal = true;
        handle_message(
            &mut app,
            Message::AuthSuccess("ghp_mock_token_abc".to_string()),
            &tx,
            &mut sys,
            size,
        );
        assert_eq!(app.auth_token.as_deref(), Some("ghp_mock_token_abc"));
        assert_eq!(app.pat_input, "ghp_mock_token_abc");
        assert!(!app.show_settings_modal);
    }
}
