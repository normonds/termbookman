//! Self-update: download the release binary, verify it, and atomically replace the running exe.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

/// Release assets are `tbm` (x86_64) and `tbm.arm` (aarch64).
pub fn resolve_update_url(url: &str, is_arm: bool) -> String {
    if is_arm && !url.contains("/tbm.arm") {
        url.replace("/download/tbm", "/download/tbm.arm")
    } else {
        url.to_string()
    }
}

/// Extracts the hex digest from `sha256sum` output (`<hex>  <name>`) or a bare digest.
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?.to_ascii_lowercase();
    (token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())).then_some(token)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Checks that `bytes` is a 64-bit little-endian ELF executable for `arch`.
pub fn check_elf(bytes: &[u8], arch: &str) -> Result<(), String> {
    if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" {
        return Err("downloaded file is not a Linux executable".to_string());
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err("downloaded file is not a 64-bit little-endian executable".to_string());
    }
    let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
    let expected = match arch {
        "x86_64" => 0x3e,
        "aarch64" => 0xb7,
        other => return Err(format!("self-update is not supported on {}", other)),
    };
    if machine != expected {
        return Err(format!(
            "downloaded binary is for the wrong architecture (machine 0x{:x}, need {})",
            machine, arch
        ));
    }
    Ok(())
}

/// Downloads and installs the update. `progress` receives human-readable status lines.
pub fn run_update(update_url: &str, progress: impl Fn(String)) -> Result<(), String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Could not get current exe path ({})", e))?;
    let url = resolve_update_url(update_url, std::env::consts::ARCH == "aarch64");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("termbookman-updater/0.1")
        .build()
        .map_err(|e| e.to_string())?;

    progress("[Update: Downloading checksum...]".to_string());
    let checksum_url = format!("{}.sha256", url);
    let res = client
        .get(&checksum_url)
        .send()
        .map_err(|e| format!("Network error ({})", e))?;
    if !res.status().is_success() {
        return Err(format!(
            "no checksum published at {} ({})",
            checksum_url,
            res.status()
        ));
    }
    let expected = parse_sha256_file(&res.text().map_err(|e| e.to_string())?)
        .ok_or("checksum file is malformed")?;

    progress("[Update: Downloading binary...]".to_string());
    let res = client
        .get(&url)
        .send()
        .map_err(|e| format!("Network error ({})", e))?;
    if !res.status().is_success() {
        return Err(format!("GitHub returned status {}", res.status()));
    }
    let bytes = res
        .bytes()
        .map_err(|e| format!("Error reading response bytes ({})", e))?;

    let actual = sha256_hex(&bytes);
    if actual != expected {
        return Err(format!(
            "checksum mismatch (expected {}, got {})",
            expected, actual
        ));
    }
    check_elf(&bytes, std::env::consts::ARCH)?;

    install(&exe_path, &bytes)?;
    Ok(())
}

/// Writes the new binary next to `exe_path`, checks that it runs, then renames it into place.
fn install(exe_path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp_path = exe_path.with_extension("tmp_update");
    std::fs::write(&temp_path, bytes)
        .map_err(|e| format!("Could not write temporary file ({})", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755));
    }

    let runs = std::process::Command::new(&temp_path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !runs {
        let _ = std::fs::remove_file(&temp_path);
        return Err("downloaded binary failed to start".to_string());
    }

    if let Err(e) = std::fs::rename(&temp_path, exe_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "Could not replace binary (Permission error? Try running as sudo) ({})",
            e
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_url_rewrite_is_idempotent() {
        let base = "https://github.com/o/r/releases/latest/download/tbm";
        assert_eq!(resolve_update_url(base, false), base);
        let arm = resolve_update_url(base, true);
        assert!(arm.ends_with("/download/tbm.arm"));
        assert_eq!(resolve_update_url(&arm, true), arm);
    }

    #[test]
    fn sha256_file_parsing() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha256_file(&format!("{}  tbm\n", h)), Some(h.clone()));
        assert_eq!(parse_sha256_file(&h.to_uppercase()), Some(h));
        assert_eq!(parse_sha256_file("nothex"), None);
        assert_eq!(parse_sha256_file(""), None);
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn elf(machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[18..20].copy_from_slice(&machine.to_le_bytes());
        b
    }

    #[test]
    fn elf_check_rejects_wrong_files() {
        assert!(check_elf(&elf(0x3e), "x86_64").is_ok());
        assert!(check_elf(&elf(0xb7), "aarch64").is_ok());
        assert!(check_elf(&elf(0xb7), "x86_64").is_err());
        assert!(check_elf(b"<html>Not Found</html>", "x86_64").is_err());
    }

    #[test]
    fn running_binary_is_a_valid_elf_for_this_arch() {
        if cfg!(target_os = "linux") {
            let me = std::fs::read(std::env::current_exe().unwrap()).unwrap();
            check_elf(&me, std::env::consts::ARCH).unwrap();
        }
    }
}
