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
        .header("Accept", "application/vnd.github.v3+json")
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

fn resolve_gist_id(
    token: &str,
    gist_id: Option<&str>,
    filename: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    if let Some(id) = gist_id {
        return Ok(Some(id.to_string()));
    }
    Ok(find_gist_id(&list_gists(token)?, filename))
}

/// Uploads `content` as `remote_filename`, updating the given gist (or the gist that has a file
/// with that name) or creating a new private gist. Returns the gist id.
pub fn upload_gist(
    token: &str,
    content: &str,
    remote_filename: &str,
    gist_id: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    let client = client()?;

    if let Some(id) = resolve_gist_id(token, gist_id, remote_filename)? {
        let res = authed(client.patch(format!("{}/gists/{}", API, id)), token)
            .json(&files_body(remote_filename, Some(content)))
            .send()?;
        let status = res.status();
        if status.is_success() {
            return Ok(id);
        }
        // A known id that no longer exists means the gist was deleted on GitHub: recreate it.
        if !(status == reqwest::StatusCode::NOT_FOUND && gist_id.is_some()) {
            return Err(format!("Failed to update gist: {}", status).into());
        }
    }

    let res = authed(client.post(format!("{}/gists", API)), token)
        .json(&create_body(remote_filename, content))
        .send()?;
    let status = res.status();
    if !status.is_success() {
        let error_body = res.text().unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Failed to create gist: {} - {}", status, error_body).into());
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
    let Some(id) = resolve_gist_id(token, gist_id, remote_filename)? else {
        return Err("Gist not found on GitHub".into());
    };
    let client = client()?;
    let res = authed(client.get(format!("{}/gists/{}", API, id)), token).send()?;
    if !res.status().is_success() {
        return Err(format!("Failed to read gist: {}", res.status()).into());
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
    if res.status().is_success() {
        Ok(plan)
    } else {
        Err(format!("Failed to delete: {}", res.status()).into())
    }
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
        assert_eq!(
            files_body("a.sh", None),
            json!({"files": {"a.sh": null}})
        );
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
        assert!(matches!(interpret_token_response(&json!({})), PollResult::Failed(_)));
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
