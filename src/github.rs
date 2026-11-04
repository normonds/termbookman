use serde_json::{json, Value};
use std::error::Error;
use std::time::Duration;

const API: &str = "https://api.github.com";
const PER_PAGE: usize = 100;
const MAX_PAGES: usize = 50;

fn client() -> Result<reqwest::blocking::Client, Box<dyn Error>> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("termbookman/0.1.0")
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn authed(
    req: reqwest::blocking::RequestBuilder,
    token: &str,
) -> reqwest::blocking::RequestBuilder {
    req.header("Authorization", format!("token {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
}

pub fn http_error_message(status: reqwest::StatusCode, body: &str) -> String {
    if (status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS)
        && (body.contains("rate limit") || body.contains("API rate limit exceeded"))
    {
        return format!("GitHub API rate limit exceeded ({})", status);
    }
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = val.get("message").and_then(|m| m.as_str()) {
            return format!("{}: {}", status, msg);
        }
    }
    format!("{}", status)
}

pub fn get_hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = vec![0u8; 256];
        let res = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if res == 0 {
            if let Some(pos) = buf.iter().position(|&b| b == 0) {
                buf.truncate(pos);
            }
            if let Ok(s) = String::from_utf8(buf) {
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "remote".to_string())
}

/// Lists all of the user's gists, following pagination.
pub fn list_gists(token: &str) -> Result<Vec<Value>, Box<dyn Error>> {
    let client = client()?;
    let mut all = Vec::new();
    for page in 1..=MAX_PAGES {
        let res = authed(
            client.get(format!("{}/gists?per_page={}&page={}", API, PER_PAGE, page)),
            token,
        )
        .send()?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("Failed to list gists ({})", status).into());
        }
        let batch: Value = res.json()?;
        let batch = batch.as_array().cloned().unwrap_or_default();
        let len = batch.len();
        all.extend(batch);
        if len < PER_PAGE {
            break;
        }
    }
    Ok(all)
}

pub fn download_text(url: &str) -> Result<String, Box<dyn Error>> {
    let res = client()?.get(url).send()?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("Download failed ({})", status).into());
    }
    Ok(res.text()?)
}

/// Id of the first gist containing a file named `filename`.
#[allow(dead_code)]
pub fn find_gist_id(gists: &[Value], filename: &str) -> Option<String> {
    gists
        .iter()
        .find(|g| {
            g["files"]
                .as_object()
                .map(|f| f.contains_key(filename))
                .unwrap_or(false)
        })
        .and_then(|g| g["id"].as_str().map(|s| s.to_string()))
}

/// `{"files": {name: {"content": ...}}}`, or `{name: null}` to remove the file.
pub fn files_body(filename: &str, content: Option<&str>) -> Value {
    let file = match content {
        Some(c) => json!({ "content": c }),
        None => Value::Null,
    };
    json!({ "files": { filename: file } })
}

pub fn create_body(filename: &str, content: &str) -> Value {
    let mut body = files_body(filename, Some(content));
    body["description"] = json!("Uploaded via termbookman");
    body["public"] = json!(false);
    body
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeletePlan {
    /// The file is the gist's only file: delete the gist.
    DeleteGist,
    /// The gist has other files: remove just this one.
    RemoveFile,
    /// The file is not in the gist.
    NotFound,
}

pub fn deletion_plan(gist: &Value, filename: &str) -> DeletePlan {
    match gist["files"].as_object() {
        Some(files) if files.contains_key(filename) => {
            if files.len() > 1 {
                DeletePlan::RemoveFile
            } else {
                DeletePlan::DeleteGist
            }
        }
        _ => DeletePlan::NotFound,
    }
}

pub fn resolve_gist_id(gist_id: Option<&str>) -> Option<String> {
    gist_id.map(|id| id.to_string())
}

/// Uploads `content` as `remote_filename`, updating the given gist (with conflict detection)
/// or creating a new private gist if the gist id is unknown. Returns the gist id.
pub fn upload_gist(
    token: &str,
    content: &str,
    remote_filename: &str,
    gist_id: Option<&str>,
    expected_hash: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    let client = client()?;

    if let Some(id) = resolve_gist_id(gist_id) {
        let mut target_filename = remote_filename.to_string();

        // Conflict check: if gist exists, check whether remote content differs from expected_hash
        let check_res = authed(client.get(format!("{}/gists/{}", API, id)), token).send()?;
        if check_res.status().is_success() {
            let gist_val: Value = check_res.json()?;
            if let Some(file_obj) = gist_val["files"].get(remote_filename) {
                let remote_content = if file_obj["truncated"].as_bool() == Some(true) {
                    if let Some(raw_url) = file_obj["raw_url"].as_str() {
                        download_text(raw_url).ok()
                    } else {
                        None
                    }
                } else {
                    file_obj["content"].as_str().map(|s| s.to_string())
                };

                if let Some(rc) = remote_content {
                    let remote_hash = crate::gists::content_hash(rc.as_bytes());
                    if let Some(exp) = expected_hash {
                        if exp != remote_hash && rc != content {
                            target_filename =
                                format!("{}.conflict-{}", remote_filename, get_hostname());
                            crate::utils::log_debug(&format!(
                                "Upload conflict for {}: remote hash {} != manifest hash {}. Saving as {}",
                                remote_filename, remote_hash, exp, target_filename
                            ));
                        }
                    }
                }
            }
        } else if check_res.status() != reqwest::StatusCode::NOT_FOUND {
            let status = check_res.status();
            let body = check_res.text().unwrap_or_default();
            return Err(format!(
                "Failed to query gist: {}",
                http_error_message(status, &body)
            )
            .into());
        }

        let res = authed(client.patch(format!("{}/gists/{}", API, id)), token)
            .json(&files_body(&target_filename, Some(content)))
            .send()?;
        let status = res.status();
        if status.is_success() {
            return Ok(id);
        }
        // A known id that no longer exists means the gist was deleted on GitHub: recreate it.
        if status != reqwest::StatusCode::NOT_FOUND {
            let body = res.text().unwrap_or_default();
            return Err(format!(
                "Failed to update gist: {}",
                http_error_message(status, &body)
            )
            .into());
        }
    }

    let res = authed(client.post(format!("{}/gists", API)), token)
        .json(&create_body(remote_filename, content))
        .send()?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().unwrap_or_default();
        return Err(format!(
            "Failed to create gist: {}",
            http_error_message(status, &body)
        )
        .into());
    }
    let created: Value = res.json()?;
    created["id"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "GitHub did not return a gist id".into())
}

/// Removes `remote_filename` from its gist. The gist itself is only deleted when that file
/// is its only file.
pub fn delete_gist_file(
    token: &str,
    gist_id: Option<&str>,
    remote_filename: &str,
) -> Result<DeletePlan, Box<dyn Error>> {
    let Some(id) = resolve_gist_id(gist_id) else {
        return Err("Cannot delete: gist id is unknown".into());
    };
    let client = client()?;
    let res = authed(client.get(format!("{}/gists/{}", API, id)), token).send()?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().unwrap_or_default();
        return Err(format!("Failed to read gist: {}", http_error_message(status, &body)).into());
    }
    let gist: Value = res.json()?;
    let plan = deletion_plan(&gist, remote_filename);
    let res = match plan {
        DeletePlan::NotFound => return Err("File not found in gist".into()),
        DeletePlan::DeleteGist => {
            authed(client.delete(format!("{}/gists/{}", API, id)), token).send()?
        }
        DeletePlan::RemoveFile => authed(client.patch(format!("{}/gists/{}", API, id)), token)
            .json(&files_body(remote_filename, None))
            .send()?,
    };
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().unwrap_or_default();
        return Err(format!(
            "Failed to delete gist file: {}",
            http_error_message(status, &body)
        )
        .into());
    }
    Ok(plan)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub fn parse_device_code(json: &Value) -> Result<DeviceCode, String> {
    match (
        json["device_code"].as_str(),
        json["user_code"].as_str(),
        json["verification_uri"].as_str(),
    ) {
        (Some(d), Some(u), Some(v)) => Ok(DeviceCode {
            device_code: d.to_string(),
            user_code: u.to_string(),
            verification_uri: v.to_string(),
            interval: json["interval"].as_u64().unwrap_or(5).max(1),
            expires_in: json["expires_in"].as_u64().unwrap_or(900),
        }),
        _ => Err(json["error_description"]
            .as_str()
            .or(json["error"].as_str())
            .map(|e| format!("GitHub: {}", e))
            .unwrap_or_else(|| "Unexpected response from GitHub".to_string())),
    }
}

pub fn request_device_code(client_id: &str, scope: &str) -> Result<DeviceCode, String> {
    let client = client().map_err(|e| e.to_string())?;
    let mut params = vec![("client_id", client_id)];
    if !scope.is_empty() {
        params.push(("scope", scope));
    }
    let res = client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .map_err(|_| "Network error fetching code.".to_string())?;
    let json: Value = res
        .json()
        .map_err(|_| "Failed to parse device code response".to_string())?;
    parse_device_code(&json)
}

#[derive(Debug, PartialEq, Eq)]
pub enum PollResult {
    Token(String),
    Pending,
    SlowDown,
    Failed(String),
}

pub fn interpret_token_response(json: &Value) -> PollResult {
    if let Some(token) = json["access_token"].as_str() {
        return PollResult::Token(token.to_string());
    }
    match json["error"].as_str() {
        Some("authorization_pending") => PollResult::Pending,
        Some("slow_down") => PollResult::SlowDown,
        Some(err) => PollResult::Failed(err.to_string()),
        None => PollResult::Failed("Unexpected response from GitHub".to_string()),
    }
}

/// Polls until the user authorizes the device, the code expires, or `cancelled()` is true.
pub fn poll_device_token(
    client_id: &str,
    code: &DeviceCode,
    cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    let client = client().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval;
    loop {
        std::thread::sleep(Duration::from_secs(interval));
        if cancelled() {
            return Err("Login cancelled".to_string());
        }
        if std::time::Instant::now() >= deadline {
            return Err("Device code expired, reopen settings to retry".to_string());
        }
        let payload = [
            ("client_id", client_id),
            ("device_code", code.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];
        let json: Value = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&payload)
            .send()
            .map_err(|_| "Network error polling token.".to_string())?
            .json()
            .map_err(|_| "Failed to parse token response".to_string())?;
        match interpret_token_response(&json) {
            PollResult::Token(t) => return Ok(t),
            PollResult::Pending => {}
            PollResult::SlowDown => interval += 5,
            PollResult::Failed(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gist_id_resolution_with_unknown_id_never_patches_existing_gist() {
        assert_eq!(resolve_gist_id(None), None);
        assert_eq!(resolve_gist_id(Some("12345")), Some("12345".to_string()));
    }

    #[test]
    fn http_error_message_extracts_only_message_and_sanitizes() {
        let err = http_error_message(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"message": "Validation Failed", "errors": ["secret stuff"]}"#,
        );
        assert_eq!(err, "400 Bad Request: Validation Failed");
        assert!(!err.contains("secret stuff"));

        let rate_limit = http_error_message(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"message": "API rate limit exceeded for user"}"#,
        );
        assert!(rate_limit.contains("rate limit exceeded"));
    }

    #[test]
    fn find_gist_id_matches_filename() {
        let gists = vec![
            json!({"id": "a", "files": {"x.sh": {}}}),
            json!({"id": "b", "files": {"y.sh": {}, "z.sh": {}}}),
        ];
        assert_eq!(find_gist_id(&gists, "z.sh").as_deref(), Some("b"));
        assert_eq!(find_gist_id(&gists, "nope"), None);
    }

    #[test]
    fn deletion_only_deletes_gist_when_file_is_alone() {
        let single = json!({"files": {"a.sh": {}}});
        let multi = json!({"files": {"a.sh": {}, "b.sh": {}}});
        assert_eq!(deletion_plan(&single, "a.sh"), DeletePlan::DeleteGist);
        assert_eq!(deletion_plan(&multi, "a.sh"), DeletePlan::RemoveFile);
        assert_eq!(deletion_plan(&multi, "c.sh"), DeletePlan::NotFound);
    }

    #[test]
    fn bodies_have_the_shape_github_expects() {
        assert_eq!(files_body("a.sh", None), json!({"files": {"a.sh": null}}));
        let body = create_body("a.sh", "echo");
        assert_eq!(body["files"]["a.sh"]["content"], "echo");
        assert_eq!(body["public"], false);
    }

    #[test]
    fn token_responses_always_lead_somewhere() {
        assert_eq!(
            interpret_token_response(&json!({"access_token": "t"})),
            PollResult::Token("t".into())
        );
        assert_eq!(
            interpret_token_response(&json!({"error": "authorization_pending"})),
            PollResult::Pending
        );
        assert_eq!(
            interpret_token_response(&json!({"error": "slow_down"})),
            PollResult::SlowDown
        );
        assert!(matches!(
            interpret_token_response(&json!({"error": "expired_token"})),
            PollResult::Failed(_)
        ));
        // Previously this case looped forever.
        assert!(matches!(
            interpret_token_response(&json!({})),
            PollResult::Failed(_)
        ));
    }

    #[test]
    fn device_code_parsing() {
        let ok = parse_device_code(&json!({
            "device_code": "d", "user_code": "U-1", "verification_uri": "https://github.com/login/device",
            "interval": 0, "expires_in": 600
        }))
        .unwrap();
        assert_eq!(ok.interval, 1, "interval is clamped to at least 1s");
        assert_eq!(ok.expires_in, 600);
        let err = parse_device_code(&json!({"error": "x", "error_description": "bad client"}));
        assert_eq!(err, Err("GitHub: bad client".to_string()));
    }
}
