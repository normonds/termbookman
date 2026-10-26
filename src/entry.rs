use crate::utils::{dedup_label, parse_lines, parse_script_content};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Name of the gist cache manifest; never shown as an entry.
pub const MANIFEST_FILE: &str = ".manifest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// One line of a commands file, run with `bash -c`.
    Command,
    /// A whole file starting with a shebang, run with `bash <file>`.
    Script,
    /// A gist file that is neither; running it prints its contents.
    Raw,
    /// A line of shell history, typed into the shell as-is.
    History,
}

/// One row of the sidebar.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Display label, unique across everything loaded together.
    pub label: String,
    /// Label within its own file. Together with `path` it identifies the entry unambiguously.
    pub key: String,
    /// Shell command for `Command`/`History`, file path for `Script`/`Raw`.
    pub command: String,
    pub info: String,
    pub kind: EntryKind,
    pub path: Option<PathBuf>,
    /// Modification time of the cached gist file, if this came from the gist cache.
    pub mtime: Option<SystemTime>,
    /// File name on GitHub, if this came from a gist.
    pub remote_name: Option<String>,
}

impl Entry {
    pub fn history(cmd: &str) -> Entry {
        Entry {
            label: cmd.to_string(),
            key: cmd.to_string(),
            command: cmd.to_string(),
            info: String::new(),
            kind: EntryKind::History,
            path: None,
            mtime: None,
            remote_name: None,
        }
    }

    pub fn is_script(&self) -> bool {
        self.kind == EntryKind::Script
    }

    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        self.label.to_lowercase().contains(&q) || self.info.to_lowercase().contains(&q)
    }
}

/// Indices of the entries matching `query`, in order.
pub fn filter_indices(entries: &[Entry], query: &str) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.matches(query))
        .map(|(i, _)| i)
        .collect()
}

fn script_name_from_filename(filename: &str) -> String {
    let stem = filename
        .trim_start_matches("script")
        .trim_start_matches(['-', '_'])
        .trim();
    if stem.is_empty() {
        filename.to_string()
    } else {
        stem.to_string()
    }
}

/// Builds the entries for one file. `label_counts` is shared by everything shown together
/// so display labels stay unique; keys only depend on the file itself.
pub fn parse_file_entries(
    path: &Path,
    content: &str,
    remote_name: Option<&str>,
    mtime: Option<SystemTime>,
    label_counts: &mut HashMap<String, usize>,
) -> Vec<Entry> {
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let remote_name = remote_name.map(|s| s.to_string());

    if content.contains("# termbookman") || remote_name.is_none() && filename == "commands.txt" {
        let (keys, cmds, infos) = parse_lines(content, "cmd", &mut HashMap::new());
        return keys
            .into_iter()
            .zip(cmds)
            .zip(infos)
            .map(|((key, command), info)| Entry {
                label: dedup_label(&key, &key, label_counts),
                key,
                command,
                info,
                kind: EntryKind::Command,
                path: Some(path.to_path_buf()),
                mtime,
                remote_name: remote_name.clone(),
            })
            .collect();
    }

    let display_name = remote_name.clone().unwrap_or_else(|| filename.clone());
    if content.trim_start().starts_with("#!") {
        let (id_opt, desc, preview) = parse_script_content(content);
        let name = id_opt.unwrap_or_else(|| script_name_from_filename(&display_name));
        let info = [desc, preview]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        vec![Entry {
            label: dedup_label(&name, &name, label_counts),
            key: filename,
            command: path.to_string_lossy().into_owned(),
            info,
            kind: EntryKind::Script,
            path: Some(path.to_path_buf()),
            mtime,
            remote_name,
        }]
    } else {
        vec![Entry {
            label: dedup_label(&display_name, &display_name, label_counts),
            key: filename,
            command: path.to_string_lossy().into_owned(),
            info: format!("GIST: {}", display_name),
            kind: EntryKind::Raw,
            path: Some(path.to_path_buf()),
            mtime,
            remote_name,
        }]
    }
}

/// Lists cached gist files: top-level files (local or legacy) and `<gist_id>/<file>`,
/// sorted so that loading order, and therefore label numbering, is deterministic.
pub fn cached_gist_files(gist_dir: &Path) -> Vec<PathBuf> {
    fn is_listed(p: &Path) -> bool {
        p.file_name()
            .map(|n| !n.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
    }
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(gist_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_listed(&path) {
                continue;
            }
            if path.is_dir() {
                if let Ok(sub) = std::fs::read_dir(&path) {
                    files.extend(
                        sub.flatten()
                            .map(|e| e.path())
                            .filter(|p| p.is_file() && is_listed(p)),
                    );
                }
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Loads commands.txt (created from `default_commands` if missing) followed by the cached gists.
pub fn load_entries(cmd_path: &Path, gist_dir: &Path, default_commands: &str) -> Vec<Entry> {
    let mut label_counts = HashMap::new();
    if !cmd_path.exists() {
        let _ = crate::utils::write_file(cmd_path, default_commands);
    }
    let content =
        std::fs::read_to_string(cmd_path).unwrap_or_else(|_| default_commands.to_string());
    let mut entries = parse_file_entries(cmd_path, &content, None, None, &mut label_counts);

    for path in cached_gist_files(gist_dir) {
        let filename = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        // A synced copy of commands.txt would only duplicate the entries above.
        if filename == "commands.txt" {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        entries.extend(parse_file_entries(
            &path,
            &content,
            Some(&filename),
            mtime,
            &mut label_counts,
        ));
    }
    entries
}

/// Re-reads a single file and returns the entry with the given key.
pub fn find_in_file(path: &Path, key: &str) -> Option<Entry> {
    let content = std::fs::read_to_string(path).ok()?;
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let filename = path.file_name()?.to_string_lossy().into_owned();
    let remote = (filename != "commands.txt").then_some(filename.as_str());
    // Only cached gists carry a cache time.
    let mtime = remote.and(mtime);
    parse_file_entries(path, &content, remote, mtime, &mut HashMap::new())
        .into_iter()
        .find(|e| e.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_dir;

    #[test]
    fn commands_file_entries_have_stable_keys_and_unique_labels() {
        let mut counts = HashMap::new();
        let a = parse_file_entries(
            Path::new("/x/commands.txt"),
            "# up start\ndocker compose up\n",
            None,
            None,
            &mut counts,
        );
        let b = parse_file_entries(
            Path::new("/x/gists/1/more.txt"),
            "# termbookman\n# up other\nvagrant up\n",
            Some("more.txt"),
            None,
            &mut counts,
        );
        assert_eq!(a[0].label, "up");
        assert_eq!(b[0].label, "up2");
        assert_eq!(b[0].key, "up", "key must not depend on other files");
        assert_eq!(b[0].kind, EntryKind::Command);
    }

    #[test]
    fn script_and_raw_gists_are_detected() {
        let mut counts = HashMap::new();
        let s = parse_file_entries(
            Path::new("/g/abc/script-backup.sh"),
            "#!/bin/bash\n# backup Copy files\nrsync -a a b\n",
            Some("script-backup.sh"),
            None,
            &mut counts,
        );
        assert_eq!(s[0].kind, EntryKind::Script);
        assert_eq!(s[0].label, "backup");
        assert_eq!(s[0].info, "Copy files rsync -a a b");
        assert_eq!(s[0].command, "/g/abc/script-backup.sh");

        let unnamed = parse_file_entries(
            Path::new("/g/abc/script-hello.sh"),
            "#!/bin/sh\necho hi\n",
            Some("script-hello.sh"),
            None,
            &mut counts,
        );
        assert_eq!(unnamed[0].label, "hello.sh");

        let r = parse_file_entries(
            Path::new("/g/def/notes.md"),
            "just notes",
            Some("notes.md"),
            None,
            &mut counts,
        );
        assert_eq!(r[0].kind, EntryKind::Raw);
        assert_eq!(r[0].command, "/g/def/notes.md");
    }

    #[test]
    fn load_entries_is_deterministic_and_skips_manifest_and_commands_copy() {
        let dir = test_dir("load");
        let cmd = dir.join("commands.txt");
        let gists = dir.join("gists");
        std::fs::create_dir_all(gists.join("bbb")).unwrap();
        std::fs::create_dir_all(gists.join("aaa")).unwrap();
        std::fs::write(gists.join(MANIFEST_FILE), "{}").unwrap();
        std::fs::write(gists.join("aaa").join("commands.txt"), "# termbookman\nls\n").unwrap();
        std::fs::write(gists.join("bbb").join("x.sh"), "#!/bin/sh\n# ls second\nls\n").unwrap();
        std::fs::write(gists.join("aaa").join("y.sh"), "#!/bin/sh\n# ls first\nls\n").unwrap();

        let entries = load_entries(&cmd, &gists, "# ls default\nls -la\n");
        assert!(cmd.exists(), "commands.txt must be created from defaults");
        let labels: Vec<_> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["ls", "ls2", "ls3"]);
        assert_eq!(entries[1].path.as_deref(), Some(gists.join("aaa").join("y.sh").as_path()));
        assert_eq!(entries, load_entries(&cmd, &gists, ""));
    }

    #[test]
    fn find_in_file_resolves_by_key() {
        let dir = test_dir("find");
        let f = dir.join("commands.txt");
        std::fs::write(&f, "# a first\necho 1\n# a second\necho 2\n").unwrap();
        let found = find_in_file(&f, "a2").unwrap();
        assert_eq!(found.command, "echo 2");
        assert_eq!(found.mtime, None, "commands.txt entries are not cached gists");
        let g = dir.join("tool.sh");
        std::fs::write(&g, "#!/bin/sh\necho\n").unwrap();
        assert!(find_in_file(&g, "tool.sh").unwrap().mtime.is_some());
        assert!(find_in_file(&f, "missing").is_none());
    }

    #[test]
    fn filter_matches_label_or_info_case_insensitively() {
        let mut counts = HashMap::new();
        let entries = parse_file_entries(
            Path::new("/c/commands.txt"),
            "# Deploy push site\nmake deploy\n# test run tests\nmake test\n",
            None,
            None,
            &mut counts,
        );
        assert_eq!(filter_indices(&entries, "PUSH"), vec![0]);
        assert_eq!(filter_indices(&entries, "test"), vec![1]);
        assert_eq!(filter_indices(&entries, ""), vec![0, 1]);
    }
}
