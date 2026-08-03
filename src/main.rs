mod app;
mod config;
mod github;
mod handlers;
mod ui;
mod utils;

use crate::app::{load_commands, App, Message};
use crate::config::save_config;
use crate::utils::log_debug;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::sync::mpsc;
use std::{
    error::Error,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
use vt100::Parser;

fn main() -> Result<(), Box<dyn Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or("Could not find executable directory")?;
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        let action = &args[1];
        if action == "-v" || action == "--version" {
            println!("build date/time: {}", env!("BUILD_DATE_TIME"));
            return Ok(());
        }
    }

    let (labels, sidebar_commands, sidebar_infos, sidebar_mtimes, sidebar_paths) =
        load_commands(exe_dir);

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
                        format!(
                            " [gist from local cache, saved {} ago]",
                            utils::format_time_passed(mtime)
                        )
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
                        format!(
                            " [gist from local cache, saved {} ago]",
                            utils::format_time_passed(mtime)
                        )
                    } else {
                        String::new()
                    };

                    if sidebar_infos[i].starts_with("__SCRIPT__") {
                        // Script entry: `bash <file>` (no -c) so only read permission is needed.
                        println!("\x1b[90mExecuting script{}: {}\x1b[0m", gist_info, cmd);
                        std::process::Command::new("bash").arg(cmd).status()?;
                    } else if cmd.contains("<prompt:") {
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

                                let val = if input.is_empty() {
                                    default.to_string()
                                } else {
                                    input.to_string()
                                };
                                final_cmd.replace_range(start..start + end + 1, &val);
                            } else {
                                break;
                            }
                        }
                        println!("\x1b[90mExecuting{}: {}\x1b[0m", gist_info, final_cmd);
                        std::process::Command::new("bash")
                            .arg("-c")
                            .arg(&final_cmd)
                            .status()?;
                    } else {
                        // Plain command: `bash -c <cmd>`
                        println!("\x1b[90mExecuting{}: {}\x1b[0m", gist_info, cmd);
                        std::process::Command::new("bash")
                            .arg("-c")
                            .arg(cmd)
                            .status()?;
                    }
                    return Ok(());
                }
            } else {
                // Label not found anywhere — nothing to run.
                eprintln!(
                    "\x1b[90mtermbookman: command not found: {}\x1b[0m",
                    search_label
                );
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
    let term_area = if size.height < 26 {
        ratatui::layout::Rect::new(0, 0, size.width, size.height)
    } else {
        ratatui::layout::Rect::new(0, 0, size.width, size.height.saturating_sub(2))
    };

    // We don't use full ui::calculate_layout here because we need to spawn PTY first
    let rows = term_area.height;
    let cols = if size.width < 106 {
        size.width
    } else {
        size.width.saturating_sub(25)
    };

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

    std::thread::spawn(move || loop {
        if let Ok(event) = event::read() {
            if let Err(_) = event_tx.send(Message::Event(event)) {
                break;
            }
        }
    });

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(1000));
        if let Err(_) = tick_tx.send(Message::Tick) {
            break;
        }
    });

    let _ = master.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    });

    let mut app = App::new(
        pty_write,
        master,
        parser,
        labels,
        sidebar_commands,
        sidebar_infos,
        sidebar_mtimes,
        sidebar_paths,
        child.process_id().unwrap_or(0),
    );

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

        terminal.draw(|f| {
            ui::render(f, &mut app);
        })?;

        match rx.recv()? {
            Message::PtyData => {
                // Just wake up to redraw
            }
            Message::Tick => {
                if let Some(path) = app.update_stats(&mut sys) {
                    if let Some(token) = &app.auth_token {
                        // Find remote name
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
                            let token = token.clone();
                            let display_name = remote_name.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(Message::GistUploadStatus(
                                    format!("[Auto-Uploading Gist: {}]", display_name),
                                    true,
                                ));
                                if let Err(e) = github::upload_gist(&token, &path, &remote_name) {
                                    let _ = tx.send(Message::GistUploadStatus(
                                        format!("✗ Auto-upload failed: {}", e),
                                        false,
                                    ));
                                } else {
                                    let _ = tx.send(Message::GistUploadStatus(
                                        format!("✓ Auto-upload success: {}", display_name),
                                        true,
                                    ));
                                }
                            });
                        }
                    }
                }
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

                        match client
                            .post(url)
                            .header("Accept", "application/json")
                            .form(&payload)
                            .send()
                        {
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
                                let _ = tx.send(Message::AuthError(
                                    "Network error polling token.".to_string(),
                                ));
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
                            .build()
                            .unwrap();

                        match client
                            .get(url)
                            .header("Authorization", format!("token {}", token))
                            .header("Accept", "application/vnd.github.v3+json")
                            .send()
                        {
                            Ok(res) => {
                                let status = res.status();
                                match res.text() {
                                    Ok(body) => {
                                        log_debug(&format!(
                                            "Gist fetch response ({}): {}",
                                            status, body
                                        ));
                                        if status.is_success() {
                                            if let Ok(json) =
                                                serde_json::from_str::<serde_json::Value>(&body)
                                            {
                                                if let Some(gists) = json.as_array() {
                                                    let mut fetched = Vec::new();
                                                    let mut label_counts =
                                                        std::collections::HashMap::new();
                                                    for gist in gists {
                                                        if let Some(files) =
                                                            gist["files"].as_object()
                                                        {
                                                            for (filename, file_info) in files {
                                                                let raw_url = file_info["raw_url"]
                                                                    .as_str()
                                                                    .unwrap_or("");
                                                                if !raw_url.is_empty() {
                                                                    if let Ok(resp) =
                                                                        client.get(raw_url).send()
                                                                    {
                                                                        if let Ok(content) =
                                                                            resp.text()
                                                                        {
                                                                            let gist_dir = directories::ProjectDirs::from("", "", "termbookman").map(|pd| pd.config_dir().join("gists")).unwrap_or_else(|| std::path::PathBuf::from("gists"));
                                                                            let _ = std::fs::create_dir_all(&gist_dir);
                                                                            let safe_filename =
                                                                                filename.replace(
                                                                                    ' ', "_",
                                                                                );
                                                                            let gist_file =
                                                                                gist_dir.join(
                                                                                    &safe_filename,
                                                                                );
                                                                            let _ = std::fs::write(
                                                                                &gist_file,
                                                                                &content,
                                                                            );

                                                                            log_debug(&format!("Processing Gist file: {} (saved as {})", filename, safe_filename));
                                                                            if content.contains(
                                                                                "# termbookman",
                                                                            ) {
                                                                                let (labels, cmds, infos) = utils::parse_lines(&content, "cmd", &mut label_counts);
                                                                                for ((label, cmd), info) in labels.into_iter().zip(cmds.into_iter()).zip(infos.into_iter()) {
                                                                                    fetched.push((label, info, cmd, Some(std::time::SystemTime::now()), Some(gist_file.clone()), filename.clone()));
                                                                                }
                                                                            } else if content
                                                                                .trim_start()
                                                                                .starts_with("#!")
                                                                            {
                                                                                log_debug("Detected script gist");
                                                                                let (id_opt, desc, code_preview) = utils::parse_script_content(&content);
                                                                                let name = id_opt.unwrap_or_else(|| {
                                                                                    filename.trim_start_matches("script").trim_start_matches('-').trim_start_matches('_').trim().to_string()
                                                                                });
                                                                                let count = label_counts.entry(name.clone()).or_insert(0);
                                                                                *count += 1;
                                                                                let final_label =
                                                                                    if *count > 1 {
                                                                                        format!(
                                                                                            "{}{}",
                                                                                            name,
                                                                                            *count
                                                                                        )
                                                                                    } else {
                                                                                        name
                                                                                    };

                                                                                let mut info =
                                                                                    "__SCRIPT__"
                                                                                        .to_string(
                                                                                        );
                                                                                if !desc.is_empty()
                                                                                {
                                                                                    info.push(' ');
                                                                                    info.push_str(
                                                                                        &desc,
                                                                                    );
                                                                                }
                                                                                if !code_preview
                                                                                    .is_empty()
                                                                                {
                                                                                    info.push(' ');
                                                                                    info.push_str(&code_preview);
                                                                                }
                                                                                fetched.push((final_label.clone(), info, gist_file.to_string_lossy().to_string(), Some(std::time::SystemTime::now()), Some(gist_file.clone()), filename.clone()));
                                                                            } else {
                                                                                log_debug("Detected default gist");
                                                                                fetched.push((filename.clone(), filename.clone(), format!("curl -sL {}", raw_url), Some(std::time::SystemTime::now()), Some(gist_file.clone()), filename.clone()));
                                                                            }
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
                                            let _ = tx.send(Message::AuthError(format!(
                                                "Gist fetch failed ({})",
                                                status
                                            )));
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Message::AuthError(format!(
                                            "Gist response read error: {}",
                                            e
                                        )));
                                    }
                                }
                            }
                            Err(e) => {
                                log_debug(&format!("Gist fetch network error: {}", e));
                                let _ = tx
                                    .send(Message::AuthError(format!("Gist network error: {}", e)));
                            }
                        }
                    });
                } else {
                    app.login_error = Some("Not logged in to GitHub.".to_string());
                    app.show_settings_modal = true;
                }
            }
            Message::DeleteGist(idx) => {
                if idx < app.gist_paths.len() {
                    let path_opt = app.gist_paths[idx].clone();
                    let remote_name = app.gist_remote_names[idx].clone();
                    let token_opt = app.auth_token.clone();
                    let tx = tx.clone();

                    // Delete locally
                    if let Some(path) = path_opt {
                        let _ = std::fs::remove_file(path);
                    }

                    // Remove from app state
                    app.gist_items.remove(idx);
                    app.gist_infos.remove(idx);
                    app.gist_commands.remove(idx);
                    app.gist_mtimes.remove(idx);
                    app.gist_paths.remove(idx);
                    app.gist_remote_names.remove(idx);

                    // Delete from GitHub if token is available
                    if let Some(token) = token_opt {
                        std::thread::spawn(move || {
                            let _ = tx.send(Message::GistUploadStatus(
                                format!("[Deleting Gist: {}]", remote_name),
                                true,
                            ));
                            match crate::github::delete_gist(&token, &remote_name) {
                                Ok(_) => {
                                    let _ = tx.send(Message::GistUploadStatus(
                                        format!("✓ Gist deleted: {}", remote_name),
                                        true,
                                    ));
                                }
                                Err(e) => {
                                    let _ = tx.send(Message::GistUploadStatus(
                                        format!("✗ Gist deletion failed: {}", e),
                                        false,
                                    ));
                                }
                            }
                        });
                    }
                }
            }
            Message::CreateNewGist => {
                app.show_new_gist_dialog = true;
                app.new_gist_name_input.clear();
            }
            Message::ConfirmNewGistName(custom_name) => {
                let gist_dir = directories::ProjectDirs::from("", "", "termbookman")
                    .map(|pd| pd.config_dir().join("gists"))
                    .unwrap_or_else(|| std::path::PathBuf::from("gists"));
                let _ = std::fs::create_dir_all(&gist_dir);
                let filename = if custom_name.trim().is_empty() {
                    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                    format!("script_{}.sh", timestamp)
                } else {
                    let name = custom_name.trim().replace(" ", "_");
                    if name.ends_with(".sh") {
                        format!("script-{}.sh", name)
                    } else {
                        format!("script-{}.sh", name)
                    }
                };
                let path = gist_dir.join(&filename);
                let content = "#!/bin/bash\n# IDENTIFIER description tags\n\necho 'Hello World'\n";
                if let Ok(_) = std::fs::write(&path, content) {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if let Ok(mtime) = metadata.modified() {
                            app.editing_file = Some((path.clone(), mtime));
                            let p_str = path.to_string_lossy().replace("'", "'\\''");
                            let cmd_str = format!("{} '{}'\r", app.config.external_editor, p_str);
                            let _ = app.pty_write.write_all(cmd_str.as_bytes());
                            let _ = app.pty_write.flush();

                            // To make sure it shows up in the list after editing
                            app.gist_items.insert(0, "NEW SCRIPT".to_string());
                            // Format: __SCRIPT__ {description} {code_preview}
                            app.gist_infos.insert(0, "__SCRIPT__".to_string());
                            app.gist_commands
                                .insert(0, path.to_string_lossy().to_string());
                            app.gist_mtimes.insert(0, Some(mtime));
                            app.gist_paths.insert(0, Some(path.clone()));
                            app.gist_remote_names.insert(0, filename.clone());
                        }
                    }
                }
                app.show_new_gist_dialog = false;
            }
            Message::UpdateBinary => {
                let update_url = app.update_url_input.clone();
                let tx = tx.clone();

                std::thread::spawn(move || {
                    let _ = tx.send(Message::GistUploadStatus(
                        "[Update: Checking for latest binary...]".to_string(),
                        true,
                    ));

                    let exe_path = match std::env::current_exe() {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = tx.send(Message::GistUploadStatus(
                                format!("✗ Update failed: Could not get current exe path ({})", e),
                                false,
                            ));
                            return;
                        }
                    };

                    let is_arm = std::env::consts::ARCH == "aarch64";
                    let final_url = if is_arm && !update_url.contains("/tbm.arm") {
                        update_url.replace("/download/tbm", "/download/tbm.arm")
                    } else {
                        update_url
                    };

                    let _ = tx.send(Message::GistUploadStatus(
                        format!("[Update: Downloading from GitHub...]"),
                        true,
                    ));

                    let client = reqwest::blocking::Client::builder()
                        .timeout(std::time::Duration::from_secs(60))
                        .redirect(reqwest::redirect::Policy::limited(10))
                        .user_agent("termbookman-updater/0.1")
                        .build()
                        .unwrap();

                    match client.get(&final_url).send() {
                        Ok(response) => {
                            if !response.status().is_success() {
                                let _ = tx.send(Message::GistUploadStatus(
                                    format!(
                                        "✗ Update failed: GitHub returned status {}",
                                        response.status()
                                    ),
                                    false,
                                ));
                                return;
                            }

                            let bytes = match response.bytes() {
                                Ok(b) => b,
                                Err(e) => {
                                    let _ = tx.send(Message::GistUploadStatus(
                                        format!(
                                            "✗ Update failed: Error reading response bytes ({})",
                                            e
                                        ),
                                        false,
                                    ));
                                    return;
                                }
                            };

                            let temp_path = exe_path.with_extension("tmp_update");
                            if let Err(e) = std::fs::write(&temp_path, &bytes) {
                                let _ = tx.send(Message::GistUploadStatus(
                                    format!(
                                        "✗ Update failed: Could not write temporary file ({})",
                                        e
                                    ),
                                    false,
                                ));
                                return;
                            }

                            // Set executable permissions
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(
                                    &temp_path,
                                    std::fs::Permissions::from_mode(0o755),
                                );
                            }

                            // Replace current binary
                            if let Err(e) = std::fs::rename(&temp_path, &exe_path) {
                                let _ = tx.send(Message::GistUploadStatus(format!("✗ Update failed: Could not replace binary (Permission error? Try running as sudo) ({})", e), false));
                                let _ = std::fs::remove_file(&temp_path);
                                return;
                            }

                            let _ = tx.send(Message::GistUploadStatus(
                                "✓ Update successful! Please restart the app.".to_string(),
                                true,
                            ));
                        }
                        Err(e) => {
                            let _ = tx.send(Message::GistUploadStatus(
                                format!("✗ Update failed: Network error ({})", e),
                                false,
                            ));
                        }
                    }
                });
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
                app.sidebar_mode = app::SidebarMode::Gists;
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

                        if (key.code == KeyCode::Char('C')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL)
                                && key.modifiers.contains(KeyModifiers::SHIFT))
                        {
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
                                    if let (Some(path), Some(token)) =
                                        (app.pending_gist_file.take(), app.auth_token.clone())
                                    {
                                        // Find original remote name
                                        let mut remote_name = None;
                                        for (i, p) in app.gist_paths.iter().enumerate() {
                                            if let Some(p) = p {
                                                if p == &path {
                                                    remote_name =
                                                        Some(app.gist_remote_names[i].clone());
                                                    break;
                                                }
                                            }
                                        }

                                        if let Some(remote_name) = remote_name {
                                            let tx = tx.clone();
                                            let display_name = remote_name.clone();
                                            std::thread::spawn(move || {
                                                let start_msg =
                                                    format!("[Uploading Gist: {}]", display_name);
                                                let _ = tx.send(Message::GistUploadStatus(
                                                    start_msg, true,
                                                ));

                                                if let Err(e) =
                                                    github::upload_gist(&token, &path, &remote_name)
                                                {
                                                    log_debug(&format!("Gist upload error: {}", e));
                                                    let msg =
                                                        format!("✗ Gist upload failed: {}", e);
                                                    let _ = tx.send(Message::GistUploadStatus(
                                                        msg, false,
                                                    ));
                                                } else {
                                                    log_debug("Gist uploaded successfully");
                                                    let msg =
                                                        format!("✓ Gist updated: {}", display_name);
                                                    let _ = tx
                                                        .send(Message::GistUploadStatus(msg, true));
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

                        if app.show_delete_confirm {
                            match key.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') => {
                                    if let Some(idx) = app.gist_index_to_delete {
                                        let _ = tx.send(Message::DeleteGist(idx));
                                    }
                                    app.show_delete_confirm = false;
                                    app.gist_index_to_delete = None;
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    app.show_delete_confirm = false;
                                    app.gist_index_to_delete = None;
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if app.show_new_gist_dialog {
                            match key.code {
                                KeyCode::Esc => {
                                    app.show_new_gist_dialog = false;
                                    app.new_gist_name_input.clear();
                                }
                                KeyCode::Backspace => {
                                    app.new_gist_name_input.pop();
                                }
                                KeyCode::Char(c) => {
                                    app.new_gist_name_input.push(c);
                                }
                                KeyCode::Enter => {
                                    let name = app.new_gist_name_input.clone();
                                    app.new_gist_name_input.clear();
                                    let _ = tx.send(Message::ConfirmNewGistName(name));
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if app.show_settings_modal {
                            if app.is_pat_focused {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.is_pat_focused = false;
                                    }
                                    KeyCode::Backspace => {
                                        app.pat_input.pop();
                                    }
                                    KeyCode::Char(c) => {
                                        app.pat_input.push(c);
                                    }
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
                            } else if app.is_update_url_focused {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.is_update_url_focused = false;
                                    }
                                    KeyCode::Backspace => {
                                        app.update_url_input.pop();
                                    }
                                    KeyCode::Char(c) => {
                                        app.update_url_input.push(c);
                                    }
                                    KeyCode::Enter => {
                                        if !app.update_url_input.trim().is_empty() {
                                            app.config.auth.update_url =
                                                app.update_url_input.trim().to_string();
                                            let _ = save_config(&app.config);
                                            app.is_update_url_focused = false;
                                        }
                                    }
                                    _ => {}
                                }
                            } else if app.is_editor_focused {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.is_editor_focused = false;
                                    }
                                    KeyCode::Backspace => {
                                        app.editor_input.pop();
                                    }
                                    KeyCode::Char(c) => {
                                        app.editor_input.push(c);
                                    }
                                    KeyCode::Enter => {
                                        if !app.editor_input.trim().is_empty() {
                                            app.config.external_editor =
                                                app.editor_input.trim().to_string();
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
                                KeyCode::Esc => {
                                    app.is_search_focused = false;
                                }
                                KeyCode::Backspace => {
                                    app.search_query.pop();
                                    app.sidebar_state.select(Some(0));
                                }
                                KeyCode::Char(c) => {
                                    app.search_query.push(c);
                                    app.sidebar_state.select(Some(0));
                                }
                                KeyCode::Enter => {
                                    app.is_search_focused = false;
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
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
                                KeyCode::Char(c) => {
                                    let _ = write!(app.pty_write, "{}", c);
                                }
                                KeyCode::Enter => {
                                    let _ = app.pty_write.write_all(b"\r");
                                }
                                KeyCode::Backspace => {
                                    let _ = app.pty_write.write_all(b"\x08");
                                }
                                KeyCode::Tab => {
                                    let _ = app.pty_write.write_all(b"\x09");
                                }
                                KeyCode::Esc => {
                                    let _ = app.pty_write.write_all(b"\x1b");
                                }
                                KeyCode::Up => {
                                    let _ = app.pty_write.write_all(b"\x1b[A");
                                }
                                KeyCode::Down => {
                                    let _ = app.pty_write.write_all(b"\x1b[B");
                                }
                                KeyCode::Right => {
                                    let _ = app.pty_write.write_all(b"\x1b[C");
                                }
                                KeyCode::Left => {
                                    let _ = app.pty_write.write_all(b"\x1b[D");
                                }
                                KeyCode::Home => {
                                    let _ = app.pty_write.write_all(b"\x1b[H");
                                }
                                KeyCode::End => {
                                    let _ = app.pty_write.write_all(b"\x1b[F");
                                }
                                KeyCode::Delete => {
                                    let _ = app.pty_write.write_all(b"\x1b[3~");
                                }
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
                        app.mouse_pos = Some((mouse.column, mouse.row));
                        handlers::handle_click(&mut app, mouse, terminal.size()?, &tx);
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

    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b[?1006l\x1b[?1000l");
    let _ = stdout.flush();
    execute!(stdout, DisableMouseCapture, LeaveAlternateScreen)?;

    while event::poll(Duration::from_millis(10))? {
        let _ = event::read()?;
    }

    disable_raw_mode()?;
    terminal.show_cursor()?;
    log_debug("App exited cleanly.");
    Ok(())
}
