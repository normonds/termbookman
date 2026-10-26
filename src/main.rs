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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
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

    let mut cmd = CommandBuilder::new("bash");
    cmd.cwd(app::initial_shell_dir(exe_dir));
    cmd.env("TERM", "xterm-256color");
    cmd.env("LANG", "C.UTF-8");
    cmd.env("COLORTERM", "truecolor");
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
                    parser_clone.lock().unwrap().process(&buffer[..n]);
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

    if app.auth_token.is_some() {
        let _ = tx.send(Message::FetchGists);
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

fn spawn_upload(
    tx: &mpsc::Sender<Message>,
    token: String,
    path: PathBuf,
    remote_name: String,
    gist_id: Option<String>,
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
        match github::upload_gist(&token, &content, &remote_name, gist_id.as_deref()) {
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
    match key.code {
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            input.push(c)
        }
        KeyCode::Enter => return true,
        _ => {}
    }
    false
}

/// The text input that currently has keyboard focus, if any.
fn focused_input(app: &mut App) -> Option<&mut String> {
    if app.show_new_gist_dialog {
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
        Message::Tick => {
            if let Some(path) = app.update_stats(sys) {
                if let Some(token) = app.auth_token.clone() {
                    let dir = gist_dir();
                    let gist_id = gists::Manifest::load(&dir).gist_id(&dir, &path);
                    let remote_name = app
                        .gist_entries
                        .iter()
                        .chain(app.sidebar_entries.iter())
                        .find(|e| e.path.as_ref() == Some(&path))
                        .and_then(|e| e.remote_name.clone())
                        .unwrap_or_else(|| {
                            path.file_name().unwrap_or_default().to_string_lossy().into_owned()
                        });
                    spawn_upload(tx, token, path, remote_name, gist_id, "Auto-upload");
                }
            }

            if app.commands_txt_changed {
                app.commands_txt_changed = false;
                if app.config.auth.sync_commands_to_gist {
                    if let (Some(token), Some(cmd_path)) =
                        (app.auth_token.clone(), app.commands_txt_path.clone())
                    {
                        let dir = gist_dir();
                        let gist_id = gists::Manifest::load(&dir).gist_id(&dir, &cmd_path);
                        spawn_upload(
                            tx,
                            token,
                            cmd_path,
                            "commands.txt".to_string(),
                            gist_id,
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
                let result = github::poll_device_token(&client_id, &code, || {
                    cancel.load(Ordering::Relaxed)
                });
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
            let _ = save_config(&app.config);
            app.show_settings_modal = false;
            app.login_error = None;
            app.github_user_code = None;
            let _ = tx.send(Message::FetchGists);
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
        Message::FetchGists => {
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
                    Ok((entries, kept)) => Message::GistsFetched(entries, kept),
                    Err(e) => Message::GistFetchError(e),
                });
            });
        }
        Message::DeleteGist(path) => {
            let dir = gist_dir();
            let entry = app
                .gist_entries
                .iter()
                .chain(app.sidebar_entries.iter())
                .find(|e| e.path.as_ref() == Some(&path))
                .cloned();
            let remote_name = entry
                .and_then(|e| e.remote_name)
                .unwrap_or_else(|| path.file_name().unwrap_or_default().to_string_lossy().into_owned());
            let gist_id = gists::Manifest::load(&dir).gist_id(&dir, &path);

            let _ = std::fs::remove_file(&path);
            gists::with_manifest(&dir, |m| m.remove(&dir, &path));
            app.gist_entries.retain(|e| e.path.as_ref() != Some(&path));
            app.sidebar_entries.retain(|e| e.path.as_ref() != Some(&path));
            let len = app.filtered_indices().len();
            if app.sidebar_state.selected().is_some_and(|s| s >= len) {
                app.sidebar_state.select(Some(len.saturating_sub(1)));
            }

            // Only touch GitHub when we know which gist the file belongs to; a script that was
            // never uploaded exists only locally, and matching by name could hit another gist.
            match (app.auth_token.clone(), gist_id) {
                (Some(token), Some(gist_id)) => {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        log_debug(&format!("Deleting gist file: {}", remote_name));
                        let _ = tx.send(Message::GistUploadStatus(
                            format!("[Deleting: {}]", remote_name),
                            true,
                        ));
                        let msg = match github::delete_gist_file(&token, Some(&gist_id), &remote_name) {
                            Ok(github::DeletePlan::RemoveFile) => {
                                (format!("✓ Removed {} from its gist", remote_name), true)
                            }
                            Ok(_) => (format!("✓ Gist deleted: {}", remote_name), true),
                            Err(e) => (format!("✗ Gist deletion failed: {}", e), false),
                        };
                        log_debug(&msg.0);
                        let _ = tx.send(Message::GistUploadStatus(msg.0, msg.1));
                    });
                }
                _ => app.set_github_status(format!("✓ Deleted local file {}", remote_name), false, 4),
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
            let update_url = app.update_url_input.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let progress = |msg: String| {
                    let _ = tx.send(Message::GistUploadStatus(msg, true));
                };
                let _ = tx.send(match updater::run_update(&update_url, progress) {
                    Ok(()) => Message::GistUploadStatus(
                        "✓ Update successful! Please restart the app.".to_string(),
                        true,
                    ),
                    Err(e) => Message::GistUploadStatus(format!("✗ Update failed: {}", e), false),
                });
            });
        }
        Message::GistsFetched(entries, kept) => {
            log_debug(&format!("Gists fetched successfully: {} items", entries.len()));
            app.loading_gist = false;
            if kept.is_empty() {
                app.set_github_status("✓ Gists loaded", false, 3);
            } else {
                app.set_github_status(
                    format!("✓ Gists loaded; kept local edits (not uploaded): {}", kept.join(", ")),
                    false,
                    10,
                );
            }
            app.gist_entries = entries;
            app.sidebar_mode = app::SidebarMode::Gists;
            app.sidebar_state.select(Some(0));
            app.reload_sidebar();
        }
        Message::GistUploadStatus(msg, is_success) => {
            let secs = if is_success { 4 } else { 8 };
            app.set_github_status(&msg, !is_success, secs);
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
                let mut p = app.parser.lock().unwrap();
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
                    if edit_text(&mut app.pat_input, &key) && !app.pat_input.trim().is_empty() {
                        let token = app.pat_input.trim().to_string();
                        app.auth_token = Some(token.clone());
                        app.config.auth.personal_access_token = Some(token);
                        let _ = save_config(&app.config);
                        app.is_pat_focused = false;
                        app.login_error = None;
                        // A manual token makes a pending device login pointless.
                        app.device_flow_cancel.store(true, Ordering::Relaxed);
                        let _ = tx.send(Message::FetchGists);
                    }
                } else if app.is_update_url_focused {
                    if edit_text(&mut app.update_url_input, &key)
                        && !app.update_url_input.trim().is_empty()
                    {
                        app.config.auth.update_url = app.update_url_input.trim().to_string();
                        let _ = save_config(&app.config);
                        app.is_update_url_focused = false;
                    }
                } else if app.is_editor_focused
                    && edit_text(&mut app.editor_input, &key)
                    && !app.editor_input.trim().is_empty()
                {
                    app.config.external_editor = app.editor_input.trim().to_string();
                    let _ = save_config(&app.config);
                    app.is_editor_focused = false;
                }
                return;
            }

            if app.is_search_focused {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => app.is_search_focused = false,
                    _ => {
                        edit_text(&mut app.search_query, &key);
                        app.sidebar_state.select(Some(0));
                    }
                }
                return;
            }

            handlers::forward_key(app, &key);
        }
        Event::Paste(text) => {
            if let Some(input) = focused_input(app) {
                input.extend(text.chars().filter(|c| !c.is_control()));
                return;
            }
            let bracketed = {
                let mut p = app.parser.lock().unwrap();
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
    fn edit_text_ignores_control_chords() {
        let mut s = String::from("ab");
        assert!(!edit_text(&mut s, &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(s, "ab");
        edit_text(&mut s, &KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT));
        edit_text(&mut s, &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        edit_text(&mut s, &KeyEvent::new(KeyCode::Char('ā'), KeyModifiers::NONE));
        assert_eq!(s, "abā");
        assert!(edit_text(&mut s, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    }
}
