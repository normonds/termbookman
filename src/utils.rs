use chrono;
use ratatui::style::Color;
use std::io::Write;

pub fn log_debug(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open("debug.log")
    {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg);
    }
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

        if trimmed.starts_with('#') {
            let desc_text = trimmed.trim_start_matches('#').trim();
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
            if code_preview.len() > 100 {
                code_preview.truncate(97);
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

        if trimmed.starts_with('#') {
            let parts: Vec<&str> = trimmed[1..].trim().split_whitespace().collect();
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

            let count = label_counts.entry(base_name.clone()).or_insert(0);
            *count += 1;

            let final_label = if *count > 1 {
                format!("{}{}", base_name, *count)
            } else {
                base_name
            };

            labels.push(final_label);
            commands.push(trimmed.to_string());
            infos.push(last_info.clone());
            last_info = String::new();
        }
    }
    (labels, commands, infos)
}

pub fn parse_color(color: &str) -> Color {
    let color = color.to_lowercase();
    if color.starts_with('#') && (color.len() == 7 || color.len() == 4) {
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
