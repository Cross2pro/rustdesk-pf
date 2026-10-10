use anyhow::{bail, Result};
use hbb_common::config::{Config, APP_DIR, APP_NAME};

const APP: &str = "rustdesk-pf";

pub fn apply(
    config_dir: Option<&str>,
    server: Option<&str>,
    relay: Option<&str>,
    key: Option<&str>,
) -> Result<()> {
    *APP_NAME.write().unwrap() = APP.to_string();
    if let Some(dir) = config_dir {
        std::fs::create_dir_all(dir)?;
        *APP_DIR.write().unwrap() = dir.to_string();
        // Desktop Config::path() uses ProjectDirs (~/.config/<APP_NAME>), not APP_DIR
        // (APP_DIR is only honored on Android/iOS). Point XDG_CONFIG_HOME so --config works.
        let xdg = std::path::Path::new(dir);
        // ProjectDirs appends APP_NAME under config_dir; if user passed the final
        // app dir, use its parent as XDG_CONFIG_HOME; else use dir itself.
        let xdg_home = if xdg.file_name().and_then(|s| s.to_str()) == Some(APP) {
            xdg.parent().unwrap_or(xdg).to_path_buf()
        } else {
            xdg.to_path_buf()
        };
        std::fs::create_dir_all(&xdg_home)?;
        std::env::set_var("XDG_CONFIG_HOME", &xdg_home);
    }
    // Force reload after path change: Config lazy_static already loaded with old path
    // on first access. Touching get_id() after APP_* is set is enough if we set APP_*
    // before any Config call — callers must call apply() first.
    if let Some(s) = server {
        Config::set_option("custom-rendezvous-server".into(), s.to_string());
    }
    if let Some(r) = relay {
        Config::set_option("relay-server".into(), r.to_string());
    }
    if let Some(k) = key {
        Config::set_option("key".into(), k.to_string());
    }
    // Password-only approval; no click/GUI confirm.
    Config::set_option("approve-mode".into(), "password".into());
    Config::set_option("verification-method".into(), "use-permanent-password".into());
    Ok(())
}

pub fn enable_tunnel() {
    Config::set_option("enable-tunnel".into(), "Y".into());
}

pub fn set_password(password: &str) -> Result<()> {
    if password.is_empty() {
        bail!("password must not be empty");
    }
    if !Config::set_permanent_password(password) {
        bail!("failed to store permanent password");
    }
    Ok(())
}

pub fn ensure_password() -> Result<()> {
    if Config::has_permanent_password() {
        return Ok(());
    }
    let p = Config::get_auto_password(8);
    set_password(&p)?;
    log::info!("generated permanent password (also printed below)");
    println!("password={p}");
    Ok(())
}

pub fn print_id_info() {
    println!("id={}", Config::get_id());
    println!("rendezvous={}", Config::get_rendezvous_server());
    let relay = Config::get_option("relay-server");
    if !relay.is_empty() {
        println!("relay={relay}");
    }
    let key = Config::get_option("key");
    if !key.is_empty() {
        println!("key={key}");
    }
    println!("enable-tunnel={}", Config::get_option("enable-tunnel"));
    println!("config_dir={}", Config::path("").display());
}
