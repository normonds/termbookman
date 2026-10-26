//! Command-line mode: `termbookman <label>` and friends. The TUI runs sidebar items by typing
//! one of these invocations into its shell.

use crate::entry::{find_in_file, Entry, EntryKind};
use crate::utils::{format_time_passed, substitute_prompts};
use std::io::Write;
use std::path::Path;

/// Handles command-line arguments. Returns the exit code if the arguments were a CLI request,
/// or `None` to start the TUI.
pub fn run(args: &[String], load: impl Fn() -> Vec<Entry>) -> Option<i32> {
    let action = args.get(1)?;
    let code = match (action.as_str(), args.get(2), args.get(3)) {
        ("-v" | "--version", _, _) => {
            println!("build date/time: {}", env!("BUILD_DATE_TIME"));
            0
        }
        ("print", Some(kind), Some(label)) if kind == "script" => {
            match find_label(&load(), label, true) {
                Some(e) => match std::fs::read_to_string(&e.command) {
                    Ok(content) => {
                        print!("{}", content);
                        0
                    }
                    Err(err) => {
                        eprintln!("Error reading script: {}", err);
                        1
                    }
                },
                None => not_found(label),
            }
        }
        ("print", Some(label), _) => match find_label(&load(), label, false) {
            Some(e) => {
                println!("{}", e.command);
                0
            }
            None => not_found(label),
        },
        ("script", Some(label), _) => match find_label(&load(), label, true) {
            Some(e) => execute(&e),
            None => not_found(label),
        },
        // Exact reference used by the TUI: the file the entry came from and its key in that file.
        ("run", Some(path), Some(key)) => match find_in_file(Path::new(path), key) {
            Some(e) => execute(&e),
            None => not_found(key),
        },
        // Ad-hoc command with `<prompt:...>` tags, used by status bar buttons.
        ("exec", Some(cmd), _) => execute(&Entry {
            label: String::new(),
            key: String::new(),
            command: cmd.clone(),
            info: String::new(),
            kind: EntryKind::Command,
            path: None,
            mtime: None,
            remote_name: None,
        }),
        (label, _, _) => match find_label(&load(), label, false) {
            Some(e) => execute(&e),
            None => not_found(label),
        },
    };
    Some(code)
}

fn not_found(label: &str) -> i32 {
    eprintln!("\x1b[90mtermbookman: command not found: {}\x1b[0m", label);
    127
}

pub fn find_label(entries: &[Entry], label: &str, scripts_only: bool) -> Option<Entry> {
    entries
        .iter()
        .find(|e| e.label == label && (!scripts_only || e.is_script()))
        .cloned()
}

fn ask_stdin(label: &str, default: &str) -> String {
    print!("{}: (Default: {}) ", label, default);
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
    let input = input.trim();
    if input.is_empty() {
        default.to_string()
    } else {
        input.to_string()
    }
}

/// What `execute` will run: the program and its arguments.
pub fn command_line(entry: &Entry, ask: impl FnMut(&str, &str) -> String) -> Vec<String> {
    match entry.kind {
        // `bash <file>` only needs read permission on the script.
        EntryKind::Script => vec!["bash".into(), entry.command.clone()],
        EntryKind::Raw => vec!["cat".into(), "--".into(), entry.command.clone()],
        EntryKind::Command | EntryKind::History => vec![
            "bash".into(),
            "-c".into(),
            substitute_prompts(&entry.command, ask),
        ],
    }
}

fn execute(entry: &Entry) -> i32 {
    let gist_info = entry
        .mtime
        .map(|m| format!(" [gist from local cache, saved {} ago]", format_time_passed(m)))
        .unwrap_or_default();
    let argv = command_line(entry, ask_stdin);
    let shown = match entry.kind {
        EntryKind::Script => format!("Executing script{}: {}", gist_info, entry.command),
        EntryKind::Raw => format!("Showing gist{}: {}", gist_info, entry.command),
        _ => format!("Executing{}: {}", gist_info, argv[2]),
    };
    println!("\x1b[90m{}\x1b[0m", shown);
    match std::process::Command::new(&argv[0]).args(&argv[1..]).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("termbookman: failed to run {}: {}", argv[0], e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(label: &str, kind: EntryKind, command: &str) -> Entry {
        Entry {
            label: label.into(),
            key: label.into(),
            command: command.into(),
            info: String::new(),
            kind,
            path: Some(PathBuf::from("/tmp/x")),
            mtime: None,
            remote_name: None,
        }
    }

    #[test]
    fn find_label_can_restrict_to_scripts() {
        let entries = vec![
            entry("deploy", EntryKind::Command, "make deploy"),
            entry("deploy2", EntryKind::Script, "/g/deploy.sh"),
        ];
        assert_eq!(find_label(&entries, "deploy", false).unwrap().command, "make deploy");
        assert!(find_label(&entries, "deploy", true).is_none());
        assert_eq!(find_label(&entries, "deploy2", true).unwrap().command, "/g/deploy.sh");
    }

    #[test]
    fn command_lines_per_kind() {
        let no_ask = |_: &str, _: &str| -> String { panic!("should not prompt") };
        assert_eq!(
            command_line(&entry("s", EntryKind::Script, "/g/a.sh"), no_ask),
            vec!["bash", "/g/a.sh"]
        );
        assert_eq!(
            command_line(&entry("r", EntryKind::Raw, "/g/notes.md"), no_ask),
            vec!["cat", "--", "/g/notes.md"]
        );
        assert_eq!(
            command_line(&entry("c", EntryKind::Command, "ls -la"), no_ask),
            vec!["bash", "-c", "ls -la"]
        );
        assert_eq!(
            command_line(
                &entry("p", EntryKind::Command, "ping <prompt:Host:1.1.1.1>"),
                |_, d| d.to_string()
            ),
            vec!["bash", "-c", "ping 1.1.1.1"]
        );
    }

    #[test]
    fn run_returns_none_without_arguments() {
        assert_eq!(run(&["tbm".to_string()], Vec::new), None);
    }

    #[test]
    fn unknown_label_is_not_found() {
        let args = vec!["tbm".to_string(), "definitely-missing".to_string()];
        assert_eq!(run(&args, Vec::new), Some(127));
    }

    #[test]
    fn run_by_file_and_key_executes_that_entry() {
        let dir = crate::utils::test_dir("clirun");
        let f = dir.join("commands.txt");
        std::fs::write(&f, "# ok\ntrue\n# fail\nexit 3\n").unwrap();
        let p = f.to_string_lossy().to_string();
        let args = |k: &str| vec!["tbm".to_string(), "run".into(), p.clone(), k.into()];
        assert_eq!(run(&args("ok"), Vec::new), Some(0));
        assert_eq!(run(&args("fail"), Vec::new), Some(3));
        assert_eq!(run(&args("nope"), Vec::new), Some(127));
    }
}
