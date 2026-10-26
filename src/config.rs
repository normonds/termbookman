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
fn home_dir_of(user: &str) -> Option<PathBuf> {
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
fn home_dir_of(_user: &str) -> Option<PathBuf> {
    None
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub statusbar: StatusBarConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default = "default_editor")]
    pub external_editor: String,
    #[serde(default)]
    pub ui: UiConfig,
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

pub fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    if let Some(config_dir) = get_config_dir() {
        let config_file = config_dir.join("config.toml");
        let content = toml::to_string_pretty(config)?;
        // config.toml holds the GitHub token, so keep it private.
        crate::utils::write_private_file(&config_file, content)?;
    }
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
"##;

pub fn load_config() -> Config {
    if let Some(config_dir) = get_config_dir() {
        let config_file = config_dir.join("config.toml");
        log_debug(&format!("Checking config at: {:?}", config_file));

        if config_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_file) {
                match toml::from_str(&content) {
                    Ok(config) => {
                        log_debug("Config loaded successfully from file");
                        return config;
                    }
                    Err(e) => {
                        log_debug(&format!("Config parse error: {}", e));
                    }
                }
            }
        } else {
            log_debug("Config file not found, writing default");
            let _ = crate::utils::write_private_file(&config_file, DEFAULT_CONFIG);
        }
    } else {
        log_debug("Could not determine project directories");
    }

    log_debug("Falling back to default config");
    toml::from_str(DEFAULT_CONFIG).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_and_is_safe_by_default() {
        let config: Config = toml::from_str(DEFAULT_CONFIG).expect("default config must parse");
        assert!(!config.auth.sync_commands_to_gist);
        assert_eq!(config.external_editor, "nano");
        assert!(config.statusbar.lower.iter().any(|i| i.action == Some(ActionType::Quit)));
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
}
