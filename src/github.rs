use std::error::Error;
use serde_json;
use reqwest;

pub fn upload_gist(token: &str, file_path: &std::path::Path, remote_filename: &str) -> Result<(), Box<dyn Error>> {
    let content = std::fs::read_to_string(file_path)?;
    
    // We need to find the Gist ID. We can scan cached gists to find which one this file belongs to.
    let client = reqwest::blocking::Client::builder()
        .user_agent("termbookman/0.1.0")
        .build()?;
    
    let url = "https://api.github.com/gists";
    let res = client.get(url)
        .header("Authorization", format!("token {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .send()?;
    
    if !res.status().is_success() {
        return Err(format!("Failed to list gists: {}", res.status()).into());
    }
    
    let gists: serde_json::Value = res.json()?;
    let mut gist_id = None;
    
    if let Some(arr) = gists.as_array() {
        for gist in arr {
            if let Some(files) = gist["files"].as_object() {
                if files.contains_key(remote_filename) {
                    gist_id = gist["id"].as_str().map(|s| s.to_string());
                    break;
                }
            }
        }
    }
    
    if let Some(gist_id) = gist_id {
        // Update existing gist
        let update_url = format!("https://api.github.com/gists/{}", gist_id);
        
        let mut files = serde_json::Map::new();
        let mut file_data = serde_json::Map::new();
        file_data.insert("content".to_string(), serde_json::Value::String(content));
        files.insert(remote_filename.to_string(), serde_json::Value::Object(file_data));
        
        let mut body = serde_json::Map::new();
        body.insert("files".to_string(), serde_json::Value::Object(files));
        
        let res = client.patch(update_url)
            .header("Authorization", format!("token {}", token))
            .header("Accept", "application/vnd.github.v3+json")
            .json(&body)
            .send()?;
            
        if res.status().is_success() {
            Ok(())
        } else {
            Err(format!("Failed to update gist: {}", res.status()).into())
        }
    } else {
        // Create new gist
        let create_url = "https://api.github.com/gists";
        
        let mut files = serde_json::Map::new();
        let mut file_data = serde_json::Map::new();
        file_data.insert("content".to_string(), serde_json::Value::String(content));
        files.insert(remote_filename.to_string(), serde_json::Value::Object(file_data));
        
        let mut body = serde_json::Map::new();
        body.insert("description".to_string(), serde_json::Value::String("Uploaded via termbookman".to_string()));
        body.insert("public".to_string(), serde_json::Value::Bool(false));
        body.insert("files".to_string(), serde_json::Value::Object(files));
        
        let res = client.post(create_url)
            .header("Authorization", format!("token {}", token))
            .header("Accept", "application/vnd.github.v3+json")
            .json(&body)
            .send()?;
            
        if res.status().is_success() {
            Ok(())
        } else {
            let status = res.status();
            let error_body = res.text().unwrap_or_else(|_| "Unknown error".to_string());
            Err(format!("Failed to create gist: {} - {}", status, error_body).into())
        }
    }
}

pub fn delete_gist(token: &str, remote_filename: &str) -> Result<(), Box<dyn Error>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("termbookman/0.1.0")
        .build()?;
    
    let url = "https://api.github.com/gists";
    let res = client.get(url)
        .header("Authorization", format!("token {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .send()?;
    
    if !res.status().is_success() {
        return Err(format!("Failed to list gists: {}", res.status()).into());
    }
    
    let gists: serde_json::Value = res.json()?;
    let mut gist_id = None;
    
    if let Some(arr) = gists.as_array() {
        for gist in arr {
            if let Some(files) = gist["files"].as_object() {
                if files.contains_key(remote_filename) {
                    gist_id = gist["id"].as_str().map(|s| s.to_string());
                    break;
                }
            }
        }
    }
    
    if let Some(gist_id) = gist_id {
        let delete_url = format!("https://api.github.com/gists/{}", gist_id);
        let res = client.delete(delete_url)
            .header("Authorization", format!("token {}", token))
            .header("Accept", "application/vnd.github.v3+json")
            .send()?;
            
        if res.status().is_success() {
            Ok(())
        } else {
            Err(format!("Failed to delete gist: {}", res.status()).into())
        }
    } else {
        Err("Gist not found on GitHub".into())
    }
}
