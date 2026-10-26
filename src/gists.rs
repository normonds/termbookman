//! Local gist cache.
//!
//! Fetched files are stored as `<gist_dir>/<gist_id>/<filename>`, so equal filenames in
//! different gists never overwrite each other. Scripts created locally live at the top level
//! until they are uploaded. `.manifest.json` remembers, per file, the hash of the content last
//! synced with GitHub and the gist id, which lets a fetch tell local edits from remote changes.

use crate::entry::MANIFEST_FILE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub hash: String,
    #[serde(default)]
    pub gist_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub files: BTreeMap<String, ManifestEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    /// Local file missing or unmodified since the last sync: take the remote content.
    Write,
    /// Local and remote content are identical.
    Unchanged,
    /// The local file has edits that were never uploaded: keep it.
    KeepLocal,
}

pub fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Decides what a fetch should do with one file.
pub fn decide(local: Option<&[u8]>, synced_hash: Option<&str>, remote: &[u8]) -> SyncAction {
    match local {
        None => SyncAction::Write,
        Some(local) if local == remote => SyncAction::Unchanged,
        Some(local) if synced_hash == Some(content_hash(local).as_str()) => SyncAction::Write,
        Some(_) => SyncAction::KeepLocal,
    }
}

/// Makes a GitHub file name safe to use as a single path component.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

pub fn cache_path(gist_dir: &Path, gist_id: &str, filename: &str) -> PathBuf {
    gist_dir
        .join(sanitize_filename(gist_id))
        .join(sanitize_filename(filename))
}

fn rel_key(gist_dir: &Path, path: &Path) -> String {
    path.strip_prefix(gist_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

impl Manifest {
    pub fn load(gist_dir: &Path) -> Manifest {
        std::fs::read_to_string(gist_dir.join(MANIFEST_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, gist_dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::utils::write_file(&gist_dir.join(MANIFEST_FILE), text)
    }

    pub fn get(&self, gist_dir: &Path, path: &Path) -> Option<&ManifestEntry> {
        self.files.get(&rel_key(gist_dir, path))
    }

    pub fn gist_id(&self, gist_dir: &Path, path: &Path) -> Option<String> {
        if let Some(id) = self.get(gist_dir, path).and_then(|e| e.gist_id.clone()) {
            return Some(id);
        }
        // Files in `<gist_dir>/<id>/` belong to gist `<id>` even without a manifest entry.
        let parent = path.parent()?;
        if parent != gist_dir && parent.parent() == Some(gist_dir) {
            return parent.file_name().map(|n| n.to_string_lossy().into_owned());
        }
        None
    }

    pub fn record_synced(
        &mut self,
        gist_dir: &Path,
        path: &Path,
        content: &[u8],
        gist_id: Option<String>,
    ) {
        self.files.insert(
            rel_key(gist_dir, path),
            ManifestEntry {
                hash: content_hash(content),
                gist_id,
            },
        );
    }

    pub fn remove(&mut self, gist_dir: &Path, path: &Path) {
        self.files.remove(&rel_key(gist_dir, path));
    }
}

static MANIFEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Loads the manifest, applies `f`, and saves it, serialized across threads.
pub fn with_manifest<T>(gist_dir: &Path, f: impl FnOnce(&mut Manifest) -> T) -> T {
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut manifest = Manifest::load(gist_dir);
    let out = f(&mut manifest);
    if let Err(e) = manifest.save(gist_dir) {
        crate::utils::log_debug(&format!("Could not save gist manifest: {}", e));
    }
    out
}

/// Stores one fetched file in the cache without clobbering local edits.
/// Returns the cache path and what was done.
pub fn store_remote_file(
    gist_dir: &Path,
    manifest: &mut Manifest,
    gist_id: &str,
    filename: &str,
    remote: &[u8],
) -> std::io::Result<(PathBuf, SyncAction)> {
    let path = cache_path(gist_dir, gist_id, filename);

    // Older versions stored every gist file flat as `<gist_dir>/<name with _ for spaces>`.
    // Move such a file into its gist folder; any difference from GitHub is a local edit.
    let legacy = gist_dir.join(sanitize_filename(&filename.replace(' ', "_")));
    let legacy_is_untracked = manifest
        .get(gist_dir, &legacy)
        .map(|e| e.gist_id.is_none() || e.gist_id.as_deref() == Some(gist_id))
        .unwrap_or(true);
    if !path.exists() && legacy.is_file() && legacy_is_untracked {
        crate::utils::create_dir_all_owned(path.parent().unwrap_or(gist_dir))?;
        std::fs::rename(&legacy, &path)?;
        let old = manifest.get(gist_dir, &legacy).cloned();
        manifest.remove(gist_dir, &legacy);
        if let Some(old) = old {
            manifest.files.insert(rel_key(gist_dir, &path), old);
        }
    }

    let local = std::fs::read(&path).ok();
    let synced = manifest.get(gist_dir, &path).map(|e| e.hash.clone());
    let action = decide(local.as_deref(), synced.as_deref(), remote);
    match action {
        SyncAction::Write => {
            crate::utils::write_file(&path, remote)?;
            manifest.record_synced(gist_dir, &path, remote, Some(gist_id.to_string()));
        }
        SyncAction::Unchanged => {
            manifest.record_synced(gist_dir, &path, remote, Some(gist_id.to_string()));
        }
        SyncAction::KeepLocal => {
            // Keep the hash of the last synced version so the edit still counts as local,
            // but remember which gist the file belongs to.
            let entry = manifest.files.entry(rel_key(gist_dir, &path)).or_default();
            entry.gist_id = Some(gist_id.to_string());
        }
    }
    Ok((path, action))
}

/// Downloads every gist file into the cache and returns the Gists-tab entries plus the names
/// of files whose unuploaded local edits were kept.
pub fn fetch_all(
    token: &str,
    gist_dir: &Path,
) -> Result<(Vec<crate::entry::Entry>, Vec<String>), String> {
    use crate::utils::log_debug;
    let gists = crate::github::list_gists(token).map_err(|e| e.to_string())?;
    log_debug(&format!("Listed {} gists", gists.len()));

    // Download first, outside the manifest lock.
    let mut downloaded = Vec::new();
    for gist in &gists {
        let (Some(id), Some(files)) = (gist["id"].as_str(), gist["files"].as_object()) else {
            continue;
        };
        for (filename, info) in files {
            let Some(url) = info["raw_url"].as_str() else {
                continue;
            };
            match crate::github::download_text(url) {
                Ok(content) => downloaded.push((id.to_string(), filename.clone(), content)),
                Err(e) => log_debug(&format!("Could not download {}: {}", filename, e)),
            }
        }
    }

    let stored: Vec<(PathBuf, String, SyncAction)> = with_manifest(gist_dir, |m| {
        downloaded
            .iter()
            .filter_map(|(id, name, content)| {
                match store_remote_file(gist_dir, m, id, name, content.as_bytes()) {
                    Ok((path, action)) => Some((path, name.clone(), action)),
                    Err(e) => {
                        log_debug(&format!("Could not cache {}: {}", name, e));
                        None
                    }
                }
            })
            .collect()
    });

    let mut counts = std::collections::HashMap::new();
    let mut entries = Vec::new();
    let mut kept = Vec::new();
    for (path, name, action) in stored {
        if action == SyncAction::KeepLocal {
            kept.push(name.clone());
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        entries.extend(crate::entry::parse_file_entries(
            &path,
            &content,
            Some(&name),
            mtime,
            &mut counts,
        ));
    }
    Ok((entries, kept))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_dir;

    #[test]
    fn decide_covers_all_cases() {
        let synced = content_hash(b"v1");
        assert_eq!(decide(None, None, b"v2"), SyncAction::Write);
        assert_eq!(decide(Some(b"v2"), Some(&synced), b"v2"), SyncAction::Unchanged);
        assert_eq!(decide(Some(b"v1"), Some(&synced), b"v2"), SyncAction::Write);
        assert_eq!(decide(Some(b"edited"), Some(&synced), b"v2"), SyncAction::KeepLocal);
        assert_eq!(decide(Some(b"unknown"), None, b"v2"), SyncAction::KeepLocal);
    }

    #[test]
    fn sanitize_blocks_path_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("a/b"), "a_b");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename(""), "_");
        assert_eq!(sanitize_filename("my script.sh"), "my script.sh");
    }

    #[test]
    fn same_filename_in_two_gists_does_not_collide() {
        let dir = test_dir("collide");
        let mut m = Manifest::default();
        let (a, _) = store_remote_file(&dir, &mut m, "g1", "run.sh", b"one").unwrap();
        let (b, _) = store_remote_file(&dir, &mut m, "g2", "run.sh", b"two").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(std::fs::read(&b).unwrap(), b"two");
        assert_eq!(m.gist_id(&dir, &a).as_deref(), Some("g1"));
    }

    #[test]
    fn fetch_keeps_unuploaded_local_edits_and_takes_remote_updates() {
        let dir = test_dir("edits");
        let mut m = Manifest::default();
        let (p, _) = store_remote_file(&dir, &mut m, "g", "a.sh", b"v1").unwrap();

        // Remote changed, local untouched -> update.
        let (_, act) = store_remote_file(&dir, &mut m, "g", "a.sh", b"v2").unwrap();
        assert_eq!(act, SyncAction::Write);
        assert_eq!(std::fs::read(&p).unwrap(), b"v2");

        // Local edit not uploaded -> keep, even when the remote changes again.
        std::fs::write(&p, b"local edit").unwrap();
        let (_, act) = store_remote_file(&dir, &mut m, "g", "a.sh", b"v3").unwrap();
        assert_eq!(act, SyncAction::KeepLocal);
        assert_eq!(std::fs::read(&p).unwrap(), b"local edit");

        // After an upload the edit is the synced version.
        m.record_synced(&dir, &p, b"local edit", Some("g".into()));
        let (_, act) = store_remote_file(&dir, &mut m, "g", "a.sh", b"v4").unwrap();
        assert_eq!(act, SyncAction::Write);
    }

    #[test]
    fn legacy_flat_files_are_migrated_without_losing_edits() {
        let dir = test_dir("legacy");
        std::fs::write(dir.join("my_tool.sh"), b"edited locally").unwrap();
        std::fs::write(dir.join("same.sh"), b"remote").unwrap();
        let mut m = Manifest::default();

        let (p, act) = store_remote_file(&dir, &mut m, "g1", "my tool.sh", b"remote").unwrap();
        assert_eq!(act, SyncAction::KeepLocal);
        assert_eq!(std::fs::read(&p).unwrap(), b"edited locally");
        assert!(!dir.join("my_tool.sh").exists());

        let (p, act) = store_remote_file(&dir, &mut m, "g2", "same.sh", b"remote").unwrap();
        assert_eq!(act, SyncAction::Unchanged);
        assert!(p.ends_with("g2/same.sh"));
    }

    #[test]
    fn manifest_round_trips_and_infers_gist_id_from_folder() {
        let dir = test_dir("manifest");
        let mut m = Manifest::default();
        let local = dir.join("script-new.sh");
        m.record_synced(&dir, &local, b"x", Some("abc".into()));
        m.save(&dir).unwrap();
        let loaded = Manifest::load(&dir);
        assert_eq!(loaded, m);
        assert_eq!(loaded.gist_id(&dir, &local).as_deref(), Some("abc"));
        assert_eq!(loaded.gist_id(&dir, &dir.join("zzz").join("f")).as_deref(), Some("zzz"));
        assert_eq!(loaded.gist_id(&dir, &dir.join("untracked.sh")), None);
    }
}
