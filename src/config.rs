use crate::utils::log_debug;
use serde::{Deserialize, Serialize};
use std::error::Error;

use std::path::PathBuf;

/// Directory holding config.toml, commands.txt, the gist cache and debug.log.
///
/// `TERMBOOKMAN_CONFIG_DIR` overrides it. Under sudo the invoking user's directory is used so
/// that sudo and non-sudo runs share the same files.
pub fn get_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TERMBOOKMAN_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        if let Some(home) = home_dir_of(&sudo_user) {
            return Some(home.join(".config").join("termbookman"));
        }
    }

    directories::ProjectDirs::from("", "", "termbookman").map(|p| p.config_dir().to_path_buf())
}

/// Cached gist files live here.
pub fn gist_dir() -> PathBuf {
    get_config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gists")
}

pub fn commands_txt_path() -> Option<PathBuf> {
    get_config_dir().map(|d| d.join("commands.txt"))
}

#[cfg(unix)]
pub fn home_dir_of(user: &str) -> Option<PathBuf> {
    let name = std::ffi::CString::new(user).ok()?;
    // SAFETY: getpwnam returns a pointer to static storage or null; we copy out immediately.
    unsafe {
        let pw = libc::getpwnam(name.as_ptr());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        Some(PathBuf::from(dir.to_string_lossy().into_owned()))
    }
}

#[cfg(not(unix))]
pub fn home_dir_of(_user: &str) -> Option<PathBuf> {
    None
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub statusbar: StatusBarConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default = "default_editor")]
    pub external_editor: String,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default = "default_scrollback")]
    pub scrollback: usize,
    #[serde(default = "default_menu_items")]
    pub menu: Vec<MenuItem>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub key: char,
    pub label: String,
    pub command: String,
}

pub fn default_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem {
            key: 'b',
            label: "Run Btop".to_string(),
            command: "btop".to_string(),
        },
        MenuItem {
            key: 's',
            label: "Check Space".to_string(),
            command: "df -h".to_string(),
        },
    ]
}

fn default_scrollback() -> usize {
    1000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            statusbar: StatusBarConfig::default(),
            auth: AuthConfig::default(),
            external_editor: default_editor(),
            ui: UiConfig::default(),
            shell: None,
            scrollback: 1000,
            menu: default_menu_items(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct UiConfig {
    #[serde(default = "default_terminal_bg")]
    pub terminal_bg: String,
    #[serde(default = "default_sidebar_bg")]
    pub sidebar_bg: String,
    #[serde(default = "default_statusbar_bg")]
    pub upper_statusbar_bg: String,
    #[serde(default = "default_statusbar_bg")]
    pub lower_statusbar_bg: String,
}

fn default_terminal_bg() -> String {
    "#000000".to_string()
}
fn default_sidebar_bg() -> String {
    "#000000".to_string()
}
fn default_statusbar_bg() -> String {
    "#000000".to_string()
}

pub fn default_editor() -> String {
    "nano".to_string()
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct AuthConfig {
    pub github_client_id: Option<String>,
    pub personal_access_token: Option<String>,
    #[serde(default = "default_repo_url")]
    pub github_repo_url: String,
    #[serde(default = "default_update_url")]
    pub update_url: String,
    #[serde(default)]
    pub scope: String,
    /// Set to true in config.toml to upload commands.txt to a private Gist whenever it changes.
    #[serde(default = "default_sync_commands")]
    pub sync_commands_to_gist: bool,
}

fn default_repo_url() -> String {
    "https://github.com/normonds/termbookman".to_string()
}
fn default_update_url() -> String {
    "https://github.com/normonds/termbookman/releases/latest/download/tbm".to_string()
}
fn default_sync_commands() -> bool {
    false
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct StatusBarConfig {
    #[serde(default)]
    pub upper: Vec<StatusBarItem>,
    #[serde(default)]
    pub lower: Vec<StatusBarItem>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct StatusBarItem {
    #[serde(default)]
    pub type_: ItemType,
    pub label: Option<String>,
    pub action: Option<ActionType>,
    pub command: Option<String>,
    pub color: Option<String>,
    pub hover_color: Option<String>,
    pub condition: Option<ConditionType>,
    pub width: Option<u16>,
}

#[derive(Deserialize, Serialize, Clone, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    #[default]
    Button,
    Spacer,
    SystemStats,
    GitInfo,
    TimeAndScroll,
    SelectedCommandInfo,
    GithubStatus,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    ToggleMenu,
    CopySelection,
    SendCommand,
    Quit,
    ShowSettingsModal,
    FetchGists,
    OpenLog,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionType {
    HasGit,
    HasSelection,
}

pub fn update_config_in_place(existing: &str, config: &Config) -> Option<String> {
    let mut lines: Vec<String> = existing.lines().map(|s| s.to_string()).collect();

    let update_key = |lines: &mut Vec<String>, key: &str, new_val: Option<&str>| {
        let prefix = format!("{} = ", key);
        let mut found = false;
        let mut i = 0;
        while i < lines.len() {
            let trimmed = lines[i].trim();
            if trimmed.starts_with(&prefix) {
                found = true;
                if let Some(val) = new_val {
                    lines[i] = format!(
                        "{} = \"{}\"",
                        key,
                        val.replace('\\', "\\\\").replace('"', "\\\"")
                    );
                    i += 1;
                } else {
                    lines.remove(i);
                }
            } else {
                i += 1;
            }
        }
        if !found {
            if let Some(val) = new_val {
                lines.push(format!(
                    "{} = \"{}\"",
                    key,
                    val.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
        }
    };

    update_key(&mut lines, "external_editor", Some(&config.external_editor));
    update_key(&mut lines, "update_url", Some(&config.auth.update_url));
    update_key(
        &mut lines,
        "personal_access_token",
        config.auth.personal_access_token.as_deref(),
    );

    let result = lines.join("\n") + "\n";
    if toml::from_str::<Config>(&result).is_ok() {
        Some(result)
    } else {
        None
    }
}

pub fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    if let Some(config_dir) = get_config_dir() {
        save_config_in_dir(&config_dir, config)?;
    }
    Ok(())
}

pub fn save_config_in_dir(
    config_dir: &std::path::Path,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let config_file = config_dir.join("config.toml");
    if config_file.exists() {
        let bak = config_dir.join("config.toml.bak");
        let _ = std::fs::copy(&config_file, &bak);
    }
    let content = if config_file.exists() {
        std::fs::read_to_string(&config_file)
            .ok()
            .and_then(|orig| update_config_in_place(&orig, config))
            .unwrap_or_else(|| toml::to_string_pretty(config).unwrap_or_default())
    } else {
        toml::to_string_pretty(config)?
    };
    // config.toml holds the GitHub token, so keep it private.
    crate::utils::write_private_file(&config_file, content)?;
    Ok(())
}

pub const DEFAULT_CONFIG: &str = r##"
external_editor = "nano"

[ui]
terminal_bg = "#000000"
sidebar_bg = "#000000"
upper_statusbar_bg = "#000000"
lower_statusbar_bg = "#000000"

[auth]
scope = "gist"
personal_access_token = "YOUR_TOKEN_HERE"
update_url = "https://github.com/normonds/termbookman/releases/latest/download/tbm"
# Upload commands.txt to a private Gist whenever it changes.
sync_commands_to_gist = false

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
label = " CLEAR "
action = "send_command"
command = "clear\r"
color = "dark_gray"
hover_color = "gray"

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
type_ = "github_status"

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
label = " LOG "
action = "open_log"
color = "yellow"
hover_color = "light_yellow"

[[statusbar.lower]]
label = " EXIT "
action = "quit"
color = "red"
hover_color = "light_red"

[[menu]]
key = "b"
label = "Run Btop"
command = "btop"

[[menu]]
key = "s"
label = "Check Space"
command = "df -h"
"##;

pub fn load_config() -> (Config, Option<String>) {
    get_config_dir().map_or_else(
        || (toml::from_str(DEFAULT_CONFIG).unwrap_or_default(), None),
        |dir| load_config_from_dir(&dir),
    )
}

pub fn load_config_from_dir(config_dir: &std::path::Path) -> (Config, Option<String>) {
    let config_file = config_dir.join("config.toml");
    log_debug(&format!("Checking config at: {:?}", config_file));

    if config_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&config_file) {
            match toml::from_str(&content) {
                Ok(config) => {
                    log_debug("Config loaded successfully from file");
                    return (config, None);
                }
                Err(e) => {
                    let err = format!("Config parse error: {}", e);
                    log_debug(&err);
                    let bak = config_dir.join("config.toml.bak");
                    let _ = std::fs::copy(&config_file, &bak);
                    return (
                        toml::from_str(DEFAULT_CONFIG).unwrap_or_default(),
                        Some(err),
                    );
                }
            }
        }
    } else {
        log_debug("Config file not found, writing default");
        let _ = crate::utils::write_private_file(&config_file, DEFAULT_CONFIG);
    }

    (toml::from_str(DEFAULT_CONFIG).unwrap_or_default(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_and_is_safe_by_default() {
        let config: Config = toml::from_str(DEFAULT_CONFIG).expect("default config must parse");
        assert!(!config.auth.sync_commands_to_gist);
        assert_eq!(config.external_editor, "nano");
        assert!(config
            .statusbar
            .lower
            .iter()
            .any(|i| i.action == Some(ActionType::Quit)));
    }

    #[test]
    fn missing_sync_key_defaults_to_off() {
        let config: Config = toml::from_str("[auth]\nscope = \"gist\"\n").unwrap();
        assert!(!config.auth.sync_commands_to_gist);
        assert_eq!(config.external_editor, "nano");
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        config.auth.personal_access_token = Some("abc".into());
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.auth.personal_access_token.as_deref(), Some("abc"));
        assert_eq!(back.statusbar.upper.len(), config.statusbar.upper.len());
    }

    #[test]
    fn bad_config_is_backed_up_and_flagged_as_error() {
        let dir = crate::utils::test_dir("bad_config");
        let cfg_file = dir.join("config.toml");
        std::fs::write(&cfg_file, "this is definitely not valid toml = [").unwrap();

        let (_config, err) = load_config_from_dir(&dir);
        assert!(err.is_some(), "Parse error must be returned");
        let bak_file = dir.join("config.toml.bak");
        assert!(bak_file.exists(), "Broken config must be backed up to .bak");
        assert_eq!(
            std::fs::read_to_string(&bak_file).unwrap(),
            "this is definitely not valid toml = ["
        );
    }

    #[test]
    fn update_config_in_place_preserves_comments() {
        let original = "# Custom Header\nexternal_editor = \"nano\"\n# Keep this comment\n[auth]\nupdate_url = \"https://example.com/tbm\"\n";
        let mut cfg: Config = toml::from_str(original).unwrap();
        cfg.external_editor = "vim".to_string();
        cfg.auth.personal_access_token = Some("ghp_secret".to_string());

        let updated = update_config_in_place(original, &cfg).unwrap();
        assert!(updated.contains("# Custom Header"));
        assert!(updated.contains("# Keep this comment"));
        assert!(updated.contains("external_editor = \"vim\""));
        assert!(updated.contains("personal_access_token = \"ghp_secret\""));
    }
}
