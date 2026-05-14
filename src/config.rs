use serde::{Deserialize, Serialize};
use std::error::Error;
use crate::utils::log_debug;

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

fn default_terminal_bg() -> String { "#000000".to_string() }
fn default_sidebar_bg() -> String { "#000000".to_string() }
fn default_statusbar_bg() -> String { "#000000".to_string() }

pub fn default_editor() -> String { "nano".to_string() }

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct AuthConfig {
    pub github_client_id: Option<String>,
    pub personal_access_token: Option<String>,
    #[serde(default)]
    pub scope: String,
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
pub enum ItemType { #[default] Button, Spacer, SystemStats, GitInfo, TimeAndScroll, SelectedCommandInfo }

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType { ToggleMenu, CopySelection, SendCommand, Quit, ShowSettingsModal, FetchGists }

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionType { HasGit, HasSelection }

pub fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
        let config_dir = proj_dirs.config_dir();
        if !config_dir.exists() {
            std::fs::create_dir_all(config_dir)?;
        }
        let config_file = config_dir.join("config.toml");
        let content = toml::to_string_pretty(config)?;
        std::fs::write(config_file, content)?;
    }
    Ok(())
}

pub fn load_config() -> Config {
    let default_config = r##"
external_editor = "nano"

[ui]
terminal_bg = "#000000"
sidebar_bg = "#000000"
upper_statusbar_bg = "#000000"
lower_statusbar_bg = "#000000"

[auth]
scope = "gist"
personal_access_token = "YOUR_TOKEN_HERE"

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
label = " GISTS "
action = "fetch_gists"
color = "yellow"
hover_color = "light_yellow"

[[statusbar.lower]]
label = " EXIT "
action = "quit"
color = "red"
hover_color = "light_red"
"##;
    
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "termbookman") {
        let config_dir = proj_dirs.config_dir();
        let config_file = config_dir.join("config.toml");
        log_debug(&format!("Checking config at: {:?}", config_file));
        
        if config_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_file) {
                match toml::from_str(&content) {
                    Ok(config) => {
                        log_debug("Config loaded successfully from file");
                        return config;
                    },
                    Err(e) => {
                        log_debug(&format!("Config parse error: {}", e));
                    }
                }
            }
        } else {
            if !config_dir.exists() {
                let _ = std::fs::create_dir_all(config_dir);
            }
            log_debug("Config file not found, writing default");
            let _ = std::fs::write(&config_file, default_config);
        }
    } else {
        log_debug("Could not determine project directories");
    }
    
    log_debug("Falling back to default config");
    toml::from_str(default_config).unwrap_or_default()
}
