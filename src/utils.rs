use ratatui::style::Color;
use std::io::Write;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthChar;

static LOG_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Number of lines kept when the log is trimmed.
const LOG_MAX_LINES: usize = 1000;
/// The log is only trimmed once it grows past this size, so a normal write is a single append.
const LOG_TRIM_THRESHOLD_BYTES: u64 = 256 * 1024;

/// The debug log lives next to config.toml, never in the current working directory.
pub fn log_path() -> PathBuf {
    crate::config::get_config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("debug.log")
}

pub fn log_debug(msg: &str) {
    let _lock = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = create_dir_all_owned(dir);
    }
    let existed = path.exists();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        let _ = writeln!(
            f,
            "[{}] {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            msg
        );
    }
    if !existed {
        set_private_permissions(&path);
        fix_owner(&path);
    }

    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > LOG_TRIM_THRESHOLD_BYTES {
        let _ = trim_log_file(&path, LOG_MAX_LINES);
    }
}

/// Keeps only the last `max_lines` lines of the file.
pub fn trim_log_file(path: &Path, max_lines: usize) -> std::io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > max_lines {
        let trimmed = lines[lines.len() - max_lines..].join("\n") + "\n";
        std::fs::write(path, trimmed)?;
    }
    Ok(())
}

/// When running under sudo, files we create in the invoking user's home must stay owned by
/// that user, otherwise later non-sudo runs cannot write them.
pub fn fix_owner(path: &Path) {
    #[cfg(unix)]
    {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let uid = std::env::var("SUDO_UID").ok().and_then(|v| v.parse::<u32>().ok());
        let gid = std::env::var("SUDO_GID").ok().and_then(|v| v.parse::<u32>().ok());
        if let (Some(uid), Some(gid)) = (uid, gid) {
            let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

pub fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

pub fn create_dir_all_owned(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    // Create missing ancestors one by one so each new directory gets the right owner.
    let mut missing = Vec::new();
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.exists() {
            break;
        }
        missing.push(d.to_path_buf());
        cur = d.parent();
    }
    std::fs::create_dir_all(dir)?;
    for d in missing.iter().rev() {
        fix_owner(d);
    }
    Ok(())
}

/// Writes a file, creating parent directories, and keeps ownership correct under sudo.
pub fn write_file(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        create_dir_all_owned(dir)?;
    }
    std::fs::write(path, content)?;
    fix_owner(path);
    Ok(())
}

/// Like `write_file`, but the file is readable by the owner only (used for secrets).
pub fn write_private_file(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        create_dir_all_owned(dir)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(content.as_ref())?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, content)?;
    // `mode` only applies on creation; tighten files that already existed.
    set_private_permissions(path);
    fix_owner(path);
    Ok(())
}

/// Quotes a string for safe use as a single bash word.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Truncates `s` so it occupies at most `max_width` terminal columns, appending ".." when cut.
/// Never splits a UTF-8 character.
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    let total: usize = s.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total <= max_width {
        return s.to_string();
    }
    let budget = max_width.saturating_sub(2);
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    if max_width >= 2 {
        out.push_str("..");
    }
    out
}

pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Replaces every `<prompt:Label:Default>` tag in `cmd` with the value returned by `ask`.
/// Values are inserted verbatim and never re-scanned, so a value containing `<prompt:` is safe.
pub fn substitute_prompts<F>(cmd: &str, mut ask: F) -> String
where
    F: FnMut(&str, &str) -> String,
{
    let mut out = String::new();
    let mut rest = cmd;
    while let Some(start) = rest.find("<prompt:") {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        out.push_str(&rest[..start]);
        let tag = &rest[start + "<prompt:".len()..start + end];
        let mut parts = tag.splitn(2, ':');
        let label = parts.next().filter(|l| !l.is_empty()).unwrap_or("Value");
        let default = parts.next().unwrap_or("");
        out.push_str(&ask(label, default));
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn format_time_passed(t: std::time::SystemTime) -> String {
    let now = std::time::SystemTime::now();
    let duration = now.duration_since(t).unwrap_or_default();
    let secs = duration.as_secs();

    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if mins > 0 || parts.is_empty() {
        parts.push(format!("{}m", mins));
    }

    parts.join("")
}

pub fn parse_script_content(content: &str) -> (Option<String>, String, String) {
    let mut identifier = None;
    let mut description = String::new();
    let mut code_preview = String::new();
    let mut found_shebang = false;
    let mut first_comment = true;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if !found_shebang {
            if trimmed.starts_with("#!") {
                found_shebang = true;
            }
            continue;
        }

        if let Some(desc_text) = trimmed.strip_prefix('#') {
            let desc_text = desc_text.trim_start_matches('#').trim();
            if !desc_text.is_empty() {
                if first_comment {
                    let parts: Vec<&str> = desc_text.split_whitespace().collect();
                    if let Some(id) = parts.first() {
                        identifier = Some(id.to_string());
                        let desc = parts[1..].join(" ");
                        if !desc.is_empty() {
                            description = desc;
                        }
                    }
                    first_comment = false;
                } else {
                    if !description.is_empty() {
                        description.push(' ');
                    }
                    description.push_str(desc_text);
                }
            }
        } else {
            if !code_preview.is_empty() {
                code_preview.push(' ');
            }
            code_preview.push_str(trimmed);
            if code_preview.chars().count() > 100 {
                code_preview = code_preview.chars().take(97).collect();
                code_preview.push_str("...");
                break;
            }
        }
    }

    (identifier, description, code_preview)
}

pub fn parse_lines(
    content: &str,
    default_label: &str,
    label_counts: &mut std::collections::HashMap<String, usize>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut labels = Vec::new();
    let mut commands = Vec::new();
    let mut infos = Vec::new();
    let mut last_label_base: Option<String> = None;
    let mut last_info = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.contains("# termbookman") {
            continue;
        }

        if let Some(comment) = trimmed.strip_prefix('#') {
            let parts: Vec<&str> = comment.split_whitespace().collect();
            if let Some(first) = parts.first() {
                last_label_base = Some(first.to_string());
                last_info = parts[1..].join(" ");
            }
        } else {
            let base_name = last_label_base.take().unwrap_or_else(|| {
                trimmed
                    .split_whitespace()
                    .next()
                    .unwrap_or(default_label)
                    .to_string()
            });

            labels.push(dedup_label(&base_name, &base_name, label_counts));
            commands.push(trimmed.to_string());
            infos.push(std::mem::take(&mut last_info));
        }
    }
    (labels, commands, infos)
}

/// Returns `base` the first time `key` is seen and `base2`, `base3`, ... afterwards.
pub fn dedup_label(
    key: &str,
    base: &str,
    label_counts: &mut std::collections::HashMap<String, usize>,
) -> String {
    let count = label_counts.entry(key.to_string()).or_insert(0);
    *count += 1;
    if *count > 1 {
        format!("{}{}", base, *count)
    } else {
        base.to_string()
    }
}

pub fn parse_color(color: &str) -> Color {
    let color = color.to_lowercase();
    if color.starts_with('#') && (color.len() == 7 || color.len() == 4) && color.is_ascii() {
        if color.len() == 7 {
            let r = u8::from_str_radix(&color[1..3], 16).unwrap_or(0);
            let g = u8::from_str_radix(&color[3..5], 16).unwrap_or(0);
            let b = u8::from_str_radix(&color[5..7], 16).unwrap_or(0);
            return Color::Rgb(r, g, b);
        } else {
            let r_raw = u8::from_str_radix(&color[1..2], 16).unwrap_or(0);
            let g_raw = u8::from_str_radix(&color[2..3], 16).unwrap_or(0);
            let b_raw = u8::from_str_radix(&color[3..4], 16).unwrap_or(0);
            return Color::Rgb(r_raw * 17, g_raw * 17, b_raw * 17);
        }
    }

    match color.as_str() {
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

/// Creates a fresh, empty temporary directory for tests.
#[cfg(test)]
pub fn test_dir(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "termbookman-test-{}-{}-{}",
        std::process::id(),
        name,
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn truncate_never_splits_multibyte_chars() {
        let s = "ābcdēfģhījklmn ✓✓✓";
        for w in 0..25 {
            let t = truncate_to_width(s, w);
            assert!(display_width(&t) <= w.max(2), "width {} -> {:?}", w, t);
        }
        assert_eq!(truncate_to_width("short", 10), "short");
        assert_eq!(truncate_to_width("✓✓✓✓✓✓", 5), "✓✓✓..");
    }

    #[test]
    fn truncate_counts_wide_chars_as_two_columns() {
        assert_eq!(truncate_to_width("日本語テキスト", 6), "日本..");
        assert_eq!(display_width("日本"), 4);
    }

    #[test]
    fn shell_quote_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("/usr/bin/tbm"), "/usr/bin/tbm");
        assert_eq!(shell_quote("/home/a b/tbm"), "'/home/a b/tbm'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a;rm -rf ~"), "'a;rm -rf ~'");
    }

    #[test]
    fn substitute_prompts_replaces_tags_with_answers() {
        let mut asked = Vec::new();
        let out = substitute_prompts("ssh <prompt:Host:localhost> -p <prompt:Port>", |l, d| {
            asked.push((l.to_string(), d.to_string()));
            if l == "Host" { "example.com".into() } else { d.into() }
        });
        assert_eq!(out, "ssh example.com -p ");
        assert_eq!(
            asked,
            vec![("Host".into(), "localhost".into()), ("Port".into(), "".into())]
        );
    }

    #[test]
    fn substitute_prompts_does_not_rescan_answers() {
        let out = substitute_prompts("echo <prompt:X>", |_, _| "<prompt:X>".into());
        assert_eq!(out, "echo <prompt:X>");
    }

    #[test]
    fn substitute_prompts_keeps_unterminated_tag() {
        assert_eq!(substitute_prompts("echo <prompt:X", |_, _| "v".into()), "echo <prompt:X");
    }

    #[test]
    fn parse_lines_uses_comment_labels_and_dedups() {
        let mut counts = HashMap::new();
        let content = "# termbookman\n# ll list files\nls -la\nls\n\n# ll again\nls -l\n";
        let (labels, cmds, infos) = parse_lines(content, "cmd", &mut counts);
        assert_eq!(labels, vec!["ll", "ls", "ll2"]);
        assert_eq!(cmds, vec!["ls -la", "ls", "ls -l"]);
        assert_eq!(infos, vec!["list files", "", "again"]);
    }

    #[test]
    fn parse_script_content_extracts_id_description_and_preview() {
        let script = "#!/bin/bash\n# deploy Push the site\n# second line\necho one\necho two\n";
        let (id, desc, preview) = parse_script_content(script);
        assert_eq!(id.as_deref(), Some("deploy"));
        assert_eq!(desc, "Push the site second line");
        assert_eq!(preview, "echo one echo two");
    }

    #[test]
    fn parse_script_content_truncates_unicode_preview_safely() {
        let script = format!("#!/bin/sh\necho {}\n", "ā".repeat(200));
        let (_, _, preview) = parse_script_content(&script);
        assert_eq!(preview.chars().count(), 100);
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn parse_color_supports_hex_and_names() {
        assert_eq!(parse_color("#ff8000"), Color::Rgb(255, 128, 0));
        assert_eq!(parse_color("#f80"), Color::Rgb(255, 136, 0));
        assert_eq!(parse_color("Light_Cyan"), Color::LightCyan);
        assert_eq!(parse_color("#ééé"), Color::White);
        assert_eq!(parse_color("nonsense"), Color::White);
    }

    #[test]
    fn format_time_passed_formats_units() {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(86400 + 3600 * 2 + 60 * 5);
        assert_eq!(format_time_passed(t), "1d2h5m");
        assert_eq!(format_time_passed(std::time::SystemTime::now()), "0m");
    }

    #[test]
    fn trim_log_file_keeps_last_lines() {
        let dir = test_dir("trimlog");
        let path = dir.join("debug.log");
        let content: String = (0..50).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(&path, content).unwrap();
        trim_log_file(&path, 10).unwrap();
        let kept = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "line40");
        assert_eq!(lines[9], "line49");
    }

    #[test]
    fn write_private_file_is_owner_only() {
        let dir = test_dir("private");
        let path = dir.join("nested").join("secret.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old").unwrap();
        write_private_file(&path, "token").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
