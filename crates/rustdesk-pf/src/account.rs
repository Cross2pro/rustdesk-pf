//! Minimal RustDesk API account login → LocalConfig access_token.
//! Password is never logged.

use anyhow::{bail, Context, Result};
use hbb_common::config::{Config, LocalConfig};
use serde_json::json;

/// POST `{api}/api/login` (official RustDesk shape) and store access_token.
pub async fn login(api_server: &str, username: &str, password: &str) -> Result<String> {
    if username.is_empty() || password.is_empty() {
        bail!("account username/password required");
    }
    let api = api_server.trim_end_matches('/');
    Config::set_option("api-server".into(), api.to_string());

    let url = format!("{api}/api/login");
    log::info!("account login → {url} user={username}");

    let body = json!({ "username": username, "password": password }).to_string();
    let host_hdr = Config::get_option("api-host");

    let text = match login_reqwest(&url, &body, &host_hdr).await {
        Ok(t) => t,
        Err(e) => {
            log::warn!("reqwest login failed ({e:#}); trying curl --resolve fallback");
            login_curl(&url, &body, &host_hdr)?
        }
    };
    parse_and_store(&text)
}

async fn login_reqwest(url: &str, body: &str, host_hdr: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .danger_accept_invalid_certs(true)
        .build()?;
    let mut req = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if !host_hdr.is_empty() {
        req = req.header("Host", host_hdr);
    }
    let resp = req.send().await.context("api login request failed")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("api login HTTP {status}: {}", truncate(&text, 200));
    }
    Ok(text)
}

fn login_curl(url: &str, body: &str, host_hdr: &str) -> Result<String> {
    // When api uses raw IP, map Host:port → IP for curl --resolve.
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sk", "-m", "20", "-H", "Content-Type: application/json", "-d", body]);
    if !host_hdr.is_empty() {
        if let Ok(u) = reqwest::Url::parse(url) {
            if let (Some(ip_host), Some(port)) = (u.host_str(), u.port_or_known_default()) {
                // --resolve hostname:port:ip
                let resolve = format!("{host_hdr}:{port}:{ip_host}");
                cmd.args(["--resolve", &resolve]);
                // Rewrite URL host to api-host so TLS/SNI + Host match
                let rewritten = url.replacen(ip_host, host_hdr, 1);
                cmd.arg(&rewritten);
            } else {
                cmd.args(["-H", &format!("Host: {host_hdr}")]).arg(url);
            }
        } else {
            cmd.args(["-H", &format!("Host: {host_hdr}")]).arg(url);
        }
    } else {
        cmd.arg(url);
    }
    let out = cmd.output().context("curl not available")?;
    if !out.status.success() {
        bail!(
            "curl login failed: {}",
            truncate(&String::from_utf8_lossy(&out.stderr), 200)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn parse_and_store(body: &str) -> Result<String> {
    let v: serde_json::Value =
        serde_json::from_str(body).context("api login response not JSON")?;
    let tok = v
        .pointer("/access_token")
        .or_else(|| v.pointer("/data/token"))
        .or_else(|| v.pointer("/data/access_token"))
        .or_else(|| v.pointer("/token"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if tok.is_empty() {
        let msg = v
            .get("message")
            .or_else(|| v.get("msg"))
            .and_then(|x| x.as_str())
            .unwrap_or("no access_token");
        bail!("api login failed: {msg}");
    }
    store_token(tok)
}

pub fn store_token(token: &str) -> Result<String> {
    LocalConfig::set_option("access_token".into(), token.to_string());
    log::info!("access_token stored (len={})", token.len());
    Ok(token.to_string())
}

pub fn get_token() -> String {
    LocalConfig::get_option("access_token")
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if t.len() < s.len() {
        format!("{t}…")
    } else {
        t
    }
}
