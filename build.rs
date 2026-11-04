use chrono::{DateTime, Utc};
use std::process::Command;

fn main() {
    let build_time = if let Ok(epoch_str) = std::env::var("SOURCE_DATE_EPOCH") {
        if let Ok(epoch) = epoch_str.trim().parse::<i64>() {
            DateTime::<Utc>::from_timestamp(epoch, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string())
        } else {
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
        }
    } else {
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
    };
    println!("cargo:rustc-env=BUILD_DATE_TIME={}", build_time);

    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_SHA={}", git_sha);
}
